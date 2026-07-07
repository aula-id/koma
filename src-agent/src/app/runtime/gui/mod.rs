//! Desktop GUI client (`koma gui`) — Wave 3+: interactive (display + input + session ops).
//!
//! The GUI is "just another daemon client". It reuses the terminal client's non-terminal
//! machinery verbatim — [`connect_attach_and_handshake`] for the socket bridge, [`apply_frame`]
//! for folding streamed snapshots/deltas into a shadow [`AppState`], and the UNCHANGED
//! [`crate::view::draw`] for rendering — and swaps ONLY the crossterm render loop for an
//! [`eframe`] window. The ratatui frame is rasterised by [`soft_ratatui`] (a software backend)
//! and shown as an egui widget through [`egui_ratatui::RataguiBackend`].
//!
//! Wave 3 wired input: each frame the app reads egui's input events and forwards them to the
//! daemon through `req_tx` exactly like the terminal client's `render_loop` does — keystrokes as
//! [`ClientRequest::SendKey`], bracketed/`Ctrl+V` paste as [`ClientRequest::Paste`] — plus the
//! CLIENT-owned mouse-wheel scroll on the shadow. Reused verbatim (no second logic path):
//! [`local_echo`] for render-ahead composer echo and [`handle_quit_confirm_key`] for the mirrored
//! `/quit` overlay.
//!
//! ## Session operations (`/new`, `/new kill`, `/resume`)
//!
//! The GUI is no longer pinned to ONE daemon session. It ports the terminal client's session-swap
//! state machine ([`crate::app::runtime::client`]'s `ClientState`): [`GuiState`] is either
//! `Attached` (rendering one daemon's frames + forwarding input) or `Swapper` (the detached
//! `/resume` picker driven LOCALLY, no connection). The daemon signals these via the same
//! `DaemonEvent` hand-off latches the render loop consumes — [`apply_frame`]'s
//! `new_session_requested` / `open_swapper_requested` out-params:
//!
//! - **`/new`** (`kill = false`) / **`/new kill`** (`kill = true`): tear the current connection
//!   down (on `kill`, queue [`ClientRequest::QuitDaemon`] FIRST so the old daemon is reaped) and
//!   attach a freshly-minted session-daemon. See [`GuiApp::do_new_session`].
//! - **`/resume`**: detach (leaving that daemon cooking), build a client-side [`SessionHub`] from
//!   cross-daemon discovery, and render/drive it LOCALLY (a Pick attaches the chosen daemon, a
//!   Cancel reconnects the one we left). See [`GuiApp::do_open_swapper`] + [`GuiApp::swapper_input`].
//! - **`/select`** (transcript dump) is terminal-only (it needs a crossterm alt-screen + a
//!   blocking keypress) — its latch is drained but deliberately IGNORED here.

use crate::app::mode::{Mode, SessionHub};
use crate::app::state::AppState;
use crate::cli::Opts;
use crate::ipc::proto::{ClientRequest, DaemonFrame, KeyWire, SessionStatus};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

// Reuse the terminal client's key handling verbatim (both bumped to `pub(crate)` — visibility
// only; see `client::mod`): the render-ahead composer echo + the mirrored `/quit` overlay keys.
use super::client::input::{handle_quit_confirm_key, local_echo, QuitConfirmKey};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use egui_ratatui::RataguiBackend;
use ratatui::Terminal;
use soft_ratatui::{CosmicText, SoftBackend};

// Reuse the terminal client's connect primitive + shadow-folding logic (both bumped to
// `pub(crate)` — visibility only). The GUI must NOT re-implement either: a second folding
// path would drift from the terminal client's.
use super::client::connect::{connect_attach_and_handshake, Connection};
use super::client::shadow::{apply_frame, reconcile_work_clock};
// The writer-flush timeout `client::mod::teardown_connection` bounds its final join by —
// reused verbatim (not redefined) so the GUI's teardown mirrors that exact sequencing.
use super::client::bridge::WRITER_FLUSH_TIMEOUT;
// Reuse the terminal swapper's cross-daemon hub builder + per-key handler + live-refresh merge
// (all bumped to `pub(crate)` — visibility only; see `client::swapper`). The GUI drives the hub
// one key/frame at a time under egui, so it can't call the blocking `run_swapper` loop, but it
// reuses everything BELOW that loop verbatim.
use super::client::swapper::{build_local_hub, handle_swapper_key, SwapperOutcome};

/// The soft-ratatui backend flavour we render with: `cosmic-text` shaping/layout (cosmic-text +
/// swash) of the bundled Nerd Font. Unlike the bitmap/RustType backends it does real glyph
/// shaping with fallback AND special-cases the block/box-drawing range (U+2580..=259F), so
/// box-drawing rules, powerline separators, braille spinners, and Nerd-Font private-use icons
/// all render without tofu or gaps. Trade-off: bold is synthesised (`Weight::BOLD`) and italic
/// faked (`FAKE_ITALIC`) from the single regular face, and colour emoji are NOT rendered.
type GuiBackend = RataguiBackend<CosmicText>;

/// The bundled monospace Nerd Font (JetBrainsMono Nerd Font Mono — OFL 1.1, see
/// `assets/fonts/LICENSE`). Baked into the binary via `include_bytes!` so `koma gui` needs no
/// font installed on the host; the "Mono" variant has fixed advance widths, required for a
/// character-cell grid.
/// (`CosmicText` takes ONE face and synthesises bold/italic, so the bold + italic `.ttf`s —
/// still on disk under `assets/fonts` — are no longer baked into the binary.)
const FONT_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf");

/// Pixel size passed to `cosmic-text` for cell rasterisation; the cell width/height (in pixels)
/// is derived from this by `SoftBackend::<CosmicText>::new` itself (it measures the `█` glyph).
/// `i32` to match that constructor's signature.
const FONT_SIZE_PX: i32 = 16;

/// Entry point for `koma gui`. Ensures a session daemon exists, attaches over its unix
/// socket, and renders that daemon's foreground session in an eframe window.
pub fn run_gui(opts: Opts) -> anyhow::Result<()> {
    use crate::model::store;

    // The client owns no sessions and writes no config; it only needs the dirs to resolve
    // the daemon's socket path (lock ownership belongs to the daemon).
    store::ensure_dirs()?;

    // Mirror main.rs' default `koma` path: `koma gui` carries no `--session` (main routes
    // here BEFORE the default id-minting), so mint a fresh uuid and make sure a daemon owns
    // it. A freshly minted id always takes the daemon's create branch.
    let session_id = opts
        .session
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // A multi-thread tokio runtime drives the two socket bridge tasks (reader + writer) that
    // `connect_attach_and_handshake` spawns. Those tasks must live for the WHOLE app (across
    // every re-attach a session-op triggers), so the runtime is moved into `GuiApp` below and
    // only dropped when the window closes.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Connect + attach + build-skew handshake through the SAME helper `/new` and `/resume`
    // re-attach with. Synchronous (it `block_on`s internally), so it runs on this plain main
    // thread — NOT inside an entered runtime context — exactly like the terminal client's
    // `attach_session`.
    let conn = connect_session(&rt, &session_id)?;

    // The shadow is a real AppState reconstructed PURELY from daemon frames (identical to the
    // terminal client's shadow). It starts as a neutral Chat; the first Snapshot replaces it.
    let mut shadow = AppState::new(Mode::Chat);
    shadow.rest.fg_mut().status = "attaching…".into();

    // Per-connection seq tracking (mirrors `render_loop`): `expected` is the seq the NEXT
    // frame should carry; `0` + `!seeded` means "seed from the first frame"; `awaiting_resync`
    // drops everything but a fresh Snapshot after a detected gap.
    let mut expected: u64 = 0;
    let mut seeded = false;
    let mut awaiting_resync = false;

    // Destructure the fresh connection so we can seed from its prebuffered frames before the
    // reader/writer channels move into `GuiState::Attached`.
    let Connection {
        frame_rx,
        req_tx,
        writer_handle,
        prebuffered,
        daemon_version: _,
    } = conn;

    // Apply any frames the handshake pulled off the wire while hunting for `Hello` (normally
    // none) BEFORE the live drain, through the SAME `apply_frame` path, so the seq stream
    // stays gap-free. Under a SCOPED runtime-enter guard because `apply_frame` folding
    // (`shadow_subagent`) `tokio::spawn`s an inert abort handle, which panics without a
    // reactor. The hand-off latches are throwaways — none can occur this early.
    {
        let _rt_guard = rt.enter();
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        for frame in prebuffered {
            apply_frame(
                frame,
                &mut shadow,
                &mut expected,
                &mut seeded,
                &mut awaiting_resync,
                &mut select_requested,
                &mut open_swapper_requested,
                &mut new_session_requested,
                &req_tx,
            );
        }
    }

    // The software ratatui backend. Initial 120x38 is a placeholder sized for the default
    // 1100x720 window at FONT_SIZE_PX — the `ui()` paint pre-sizes the soft backend to the egui
    // panel BEFORE each `terminal.draw` (see `GuiApp::paint`), so this only matters for the very
    // first frame.
    //
    // Fonts: `cosmic-text` shaping/layout (via soft_ratatui's `cosmic-text` backend) of the
    // bundled Nerd Font — real glyph shaping + fallback + block/box special-casing, so box-drawing
    // rules, powerline separators, braille spinners, AND Nerd-Font private-use icons render
    // without tofu or gaps. Takes ONE face (`FONT_REGULAR`); bold/italic are synthesised.
    let soft = SoftBackend::<CosmicText>::new(120, 38, FONT_SIZE_PX, FONT_REGULAR);
    let terminal = Terminal::new(RataguiBackend::new("koma", soft))?;

    let app = GuiApp {
        shadow,
        terminal,
        expected,
        seeded,
        awaiting_resync,
        last_sent_wrap_w: None,
        state: GuiState::Attached {
            frame_rx,
            req_tx,
            writer_handle,
        },
        current_session_id: Some(session_id),
        prev_session: None,
        _rt: rt,
    };

    // `run_native` blocks the main thread until the window closes; dropping `app` afterwards
    // drops `_rt`, cancelling the bridge tasks. 1100x720 comfortably fits the default 120x38
    // grid at FONT_SIZE_PX; 640x400 is the floor below which the TUI layout stops being usable.
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("koma")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("koma", native_options, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;
    Ok(())
}

/// The GUI client's run-state — the eframe analogue of the terminal client's `ClientState`.
///
/// Either ATTACHED to one session-daemon (rendering its frames + forwarding input) or running
/// the local detached SWAPPER (the `/resume` picker), with a transient CLOSING state used as the
/// `std::mem::replace` placeholder while a session-op tears the old connection down.
enum GuiState {
    /// Live attached to a session-daemon. Holds the bridge channels + writer handle unpacked
    /// (the GUI folds/forwards through these each frame); a session-op teardown moves the whole
    /// variant out via `std::mem::replace` and consumes `req_tx`/`writer_handle`.
    Attached {
        /// Incoming daemon frames (reader task -> this ui thread). std mpsc, so `Send`.
        frame_rx: std::sync::mpsc::Receiver<DaemonFrame>,
        /// Outgoing client requests: keystrokes ([`ClientRequest::SendKey`]), paste, editor
        /// wrap width, the graceful-close `Detach`, `QuitDaemon`, and `apply_frame`'s Resync.
        req_tx: Sender<ClientRequest>,
        /// The writer task's handle, JOINed (bounded by [`WRITER_FLUSH_TIMEOUT`]) at teardown so
        /// the final enqueued frame (`Detach`, or `QuitDaemon` ahead of it) actually reaches the
        /// socket before the runtime tears down — mirrors `client::mod::teardown_connection`.
        writer_handle: tokio::task::JoinHandle<()>,
    },
    /// Detached, showing the local cross-daemon `/resume` swapper. No connection feeds it;
    /// keys are handled LOCALLY (never sent to a daemon). Its `Drop` stops+joins the probe.
    Swapper(SwapperState),
    /// No live connection; a transition placeholder / terminal "window is closing" state.
    Closing,
}

/// The detached `/resume` swapper's client-side state: the hub plus its background discovery
/// probe thread (mirrors [`super::client::swapper::run_swapper`]'s probe, adapted to the
/// frame-driven GUI). Cross-daemon discovery ([`super::manage::list_live_sessions`]) blocks
/// per-socket, so it runs OFF the ui thread; each frame the ui only DRAINS the newest snapshot.
struct SwapperState {
    /// The picker hub, rendered each frame via `Mode::SessionHub` through the unchanged view.
    hub: SessionHub,
    /// Stop flag for the probe thread (checked every ~100ms so shutdown is prompt).
    stop: Arc<AtomicBool>,
    /// Freshest cross-daemon discovery snapshot (probe thread -> ui thread).
    snap_rx: std::sync::mpsc::Receiver<Vec<SessionStatus>>,
    /// The probe thread handle; `Drop` sets `stop` and joins so no thread is orphaned when the
    /// swapper closes (it opens/closes repeatedly within one long-lived window).
    probe: Option<std::thread::JoinHandle<()>>,
}

impl Drop for SwapperState {
    fn drop(&mut self) {
        // Mirrors the terminal swapper's `ProbeGuard`: signal, then join (the thread observes
        // the flag within ~100ms). A panicked probe just yields `Err` — nothing to do.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.probe.take() {
            let _ = handle.join();
        }
    }
}

/// The eframe app: a shadow [`AppState`] fed by the daemon's frames, rendered each repaint
/// through the unchanged ratatui view into a software backend shown as an egui widget, plus the
/// session-swap state machine ([`GuiState`]).
struct GuiApp {
    /// Shadow state rebuilt purely from daemon snapshots/deltas (and, in the swapper, the local
    /// hub written onto its `Mode::SessionHub`).
    shadow: AppState,
    /// Software ratatui backend wrapped as an egui widget.
    terminal: Terminal<GuiBackend>,
    /// Per-connection seq expectation (see `run_gui`). Reset on every (re)attach.
    expected: u64,
    seeded: bool,
    awaiting_resync: bool,
    /// Last agents-editor wrap width sent to the daemon, so `EditorWrapW` is only re-sent on a
    /// change (and re-sent on a fresh editor open, when the daemon's editor is back at
    /// `usize::MAX`). Mirrors `render_loop`'s `last_sent_wrap_w`; `None` when not in the editor.
    last_sent_wrap_w: Option<usize>,
    /// Attached / Swapper / Closing — see [`GuiState`].
    state: GuiState,
    /// The session we are (or are becoming) attached to. `None` only while detached in the
    /// swapper. Used to flag the swapper's foreground row and as the swapper's cancel target.
    current_session_id: Option<String>,
    /// What a swapper CANCEL reconnects to (the session `/resume` was invoked from). Set when
    /// entering the swapper; a `/new` deliberately leaves it as-is (a `/new` is not a cancel).
    prev_session: Option<String>,
    /// Owns the runtime so the bridge tasks outlive the window. Declared LAST so it drops
    /// after everything else, cancelling the tasks only once nothing else references them.
    /// Also used directly for the bounded writer join at teardown + `connect_session`.
    _rt: tokio::runtime::Runtime,
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use std::sync::mpsc::TryRecvError;

        // What transition (if any) this frame resolves to. It is applied AFTER all per-frame
        // borrows AND the runtime-enter guard(s) are released — the session-op transitions
        // `block_on` (connect/teardown), which PANICS ("Cannot start a runtime from within a
        // runtime") if a runtime context is entered on this thread. That is exactly why the
        // `_rt.enter()` guard below is NARROWED to wrap ONLY the frame-drain + `apply_frame`
        // folding (the sole code needing the reactor, for `shadow_subagent`'s `tokio::spawn`),
        // instead of the whole `ui()` body as before.
        enum Next {
            Stay,
            Close,
            NewSession { kill: bool },
            OpenSwapper,
            Pick(String),
            Cancel,
        }
        let mut next = Next::Stay;

        // Already closing (a prior cancel with nothing to return to): keep the window closing.
        if matches!(self.state, GuiState::Closing) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if matches!(self.state, GuiState::Attached { .. }) {
            // ===================== ATTACHED FRAME =====================
            let mut select_requested = false;
            let mut open_swapper_requested = false;
            let mut new_session_requested: Option<bool> = None;
            let mut disconnected = false;

            // --- (a) drain EVERY queued daemon frame non-blocking, folding each into the shadow.
            // NARROWED runtime-enter guard: it covers ONLY this drain (the `apply_frame` folding),
            // then drops — so the session-op `block_on`s later in this method never run under an
            // entered context. A closed socket => the daemon is gone.
            {
                let _rt_guard = self._rt.enter();
                if let GuiState::Attached { frame_rx, req_tx, .. } = &self.state {
                    loop {
                        match frame_rx.try_recv() {
                            Ok(frame) => {
                                // Disjoint-field borrows: `&self.state` (shared, for
                                // `frame_rx`/`req_tx`) coexists with the `&mut self.shadow` /
                                // `&mut self.expected` / … folding borrows.
                                apply_frame(
                                    frame,
                                    &mut self.shadow,
                                    &mut self.expected,
                                    &mut self.seeded,
                                    &mut self.awaiting_resync,
                                    &mut select_requested,
                                    &mut open_swapper_requested,
                                    &mut new_session_requested,
                                    req_tx,
                                );
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                disconnected = true;
                                break;
                            }
                        }
                    }
                }
            }
            // `/select` (transcript dump) is terminal-only — it needs a crossterm alt-screen and a
            // blocking keypress, neither of which the GUI has. Its latch is drained above but
            // deliberately IGNORED here (this read just documents that; no warning either way).
            let _ = select_requested;

            // --- (b) advance the LOCAL-clock animations so the comet + loading spinner tick
            // between snapshots. Pure clock/frame ticks — NO reactor needed, so outside the guard.
            reconcile_work_clock(&mut self.shadow);
            if let Mode::Loading(s) = self.shadow.mode_mut() {
                s.frame = s.frame.wrapping_add(1);
            }
            // Expire a locally-reconstructed toast once its TTL passes (the client owns its own
            // dismissal timer; the daemon never sends a "toast cleared" delta).
            {
                let fg = self.shadow.rest.fg_mut();
                if let Some((_, until, _)) = fg.toast.as_ref() {
                    if std::time::Instant::now() >= *until {
                        fg.toast = None;
                    }
                }
            }

            // --- (c) render the shadow into the software backend + present it.
            self.paint(ui);

            // Decide this frame's transition from the drained hand-off latches. `/new` takes
            // precedence over `/resume` (only one is ever emitted per drain); a dead socket
            // closes the window.
            if let Some(kill) = new_session_requested {
                next = Next::NewSession { kill };
            } else if open_swapper_requested {
                next = Next::OpenSwapper;
            } else if disconnected {
                next = Next::Close;
            }

            // --- (d)+(e) wrap-width forward + input forward — ONLY while staying attached. If a
            // session-op fired (or the socket died) we do NOT send more keys to a daemon we are
            // about to leave.
            if matches!(next, Next::Stay) {
                // A cheap clone of the sender sidesteps the borrow conflict between reading
                // `req_tx` out of `self.state` and `&mut self`-borrowing methods below.
                let req_tx = match &self.state {
                    GuiState::Attached { req_tx, .. } => req_tx.clone(),
                    _ => unreachable!("attached frame implies Attached state"),
                };

                // (d) forward the agents-editor wrap width (mirror render_loop's c-bis). The
                // shadow's agents editor publishes its `wrap_w` via interior mutability during
                // the draw above; send the client-side value whenever it changes, and reset to
                // `None` when NOT in the editor so each fresh open re-sends.
                let wrap_now: Option<usize> = if let Mode::Agents(ref a) = self.shadow.mode() {
                    a.editor.as_ref().map(|(_, ed)| ed.wrap_w.get())
                } else {
                    None
                };
                match wrap_now {
                    Some(w) if self.last_sent_wrap_w != Some(w) => {
                        self.last_sent_wrap_w = Some(w);
                        let _ = req_tx.send(ClientRequest::EditorWrapW(w));
                    }
                    Some(_) => {}
                    None => self.last_sent_wrap_w = None,
                }

                // (e) translate this frame's egui input into daemon requests.
                self.forward_input(ui.ctx(), &req_tx);
            }
        } else {
            // ===================== SWAPPER FRAME =====================
            // (1) Live refresh OFF the input thread: drain the probe channel to the NEWEST
            // snapshot (non-blocking) and, if one arrived, merge it — updating working/done flags
            // + the session list without disturbing the user's cursor/focus/query/pending-kill.
            let current = self.prev_session.clone();
            if let GuiState::Swapper(sw) = &mut self.state {
                let mut latest: Option<Vec<SessionStatus>> = None;
                while let Ok(snap) = sw.snap_rx.try_recv() {
                    latest = Some(snap);
                }
                if let Some(snap) = latest {
                    // Reuse the terminal swapper's identity-preserving merge (bumped pub(crate)).
                    super::client::swapper::apply_snapshot(&mut sw.hub, snap, current.as_deref());
                }
            }

            // (2) Render the hub through the EXISTING renderer: write it onto the shadow's
            // foreground mode (a clone — a couple of short Vecs of metadata) and draw. No daemon,
            // no live runtime — `view::draw` is pure-from-snapshot, so this renders identically to
            // a terminal `/resume`.
            if let GuiState::Swapper(sw) = &self.state {
                self.shadow
                    .set_mode(Mode::SessionHub(Box::new(sw.hub.clone())));
            }
            self.paint(ui);

            // (3) Drive the hub with this frame's egui input, LOCALLY (no SendKey — there is no
            // connection). A resolved outcome becomes the transition.
            match self.swapper_input(ui.ctx()) {
                Some(SwapperOutcome::Pick(target)) => next = Next::Pick(target),
                Some(SwapperOutcome::Cancel) => next = Next::Cancel,
                None => {}
            }
        }

        // ===================== apply the transition (OUTSIDE any runtime-enter guard) =====================
        // A transition swaps `self.state` (and re-seeds the shadow); this frame already painted the
        // OLD state, so request an IMMEDIATE repaint below so the new state (picker / fresh attach)
        // shows at once instead of waiting for the idle keep-alive.
        let transitioned = !matches!(next, Next::Stay);
        match next {
            Next::Stay => {}
            Next::Close => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            Next::NewSession { kill } => self.do_new_session(kill),
            Next::OpenSwapper => self.do_open_swapper(),
            Next::Pick(target) => self.do_pick(target),
            Next::Cancel => self.do_cancel(ui.ctx()),
        }

        // ===================== repaint scheduling =====================
        if transitioned {
            // The state just changed — paint the new state on the very next frame (no keep-alive
            // wait). Harmless for `Close` (the window is closing anyway).
            ui.ctx().request_repaint();
        }
        match &self.state {
            GuiState::Closing => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            // The picker refreshes from the ~1s background probe (and on input, which egui wakes
            // instantly) — a slow keep-alive is enough to pick that up.
            GuiState::Swapper(_) => ui
                .ctx()
                .request_repaint_after(Duration::from_millis(100)),
            GuiState::Attached { .. } => {
                if self.is_animating() {
                    // Something is animating: repaint at the monitor's cadence so it advances.
                    ui.ctx().request_repaint();
                } else {
                    // Idle: slow keep-alive (~10fps) so a daemon-pushed state change (which does
                    // NOT wake egui) is still picked up within ~100ms; egui repaints INSTANTLY on
                    // input, so typing/scroll latency is unaffected.
                    ui.ctx()
                        .request_repaint_after(Duration::from_millis(100));
                }
            }
        }
    }

    /// Flush the writer task's final queued frame before the runtime tears down — mirrors
    /// `client::mod::teardown_connection`, ported to eframe's shutdown hook.
    ///
    /// Only an [`GuiState::Attached`] state has a live connection to flush (`forward_input`'s
    /// window-close arm and the `/quit` overlay `[k]`/`[d]` paths only ENQUEUE their shutdown
    /// request on `req_tx`; the writer needs the channel to CLOSE before it does its final
    /// drain). A `Swapper` is already detached and `Closing` is done — both no-ops.
    fn on_exit(&mut self) {
        if let GuiState::Attached {
            req_tx,
            writer_handle,
            frame_rx: _,
        } = std::mem::replace(&mut self.state, GuiState::Closing)
        {
            self.teardown_attached(req_tx, writer_handle);
        }
    }

    /// Paint the window clear (the surface behind the panel, and anything the panel fill misses)
    /// with the active theme's canvas bg instead of eframe's default dark grey — the other half of
    /// the black-wedge fix (see [`GuiApp::paint`]'s panel fill + [`GuiApp::theme_bg`]). Recomputed
    /// each frame from the live shadow config so it tracks the palette picker. `clear_color` wants
    /// its value in sRGB gamma space, so convert via `to_normalized_gamma_f32` (the exact call the
    /// trait's own default uses).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme_bg().to_normalized_gamma_f32()
    }
}

impl GuiApp {
    /// The active theme's canvas background as an [`egui::Color32`] — the SAME `palette.bg`
    /// [`crate::view::draw`] paints the terminal canvas with. Resolved from the LIVE shadow config
    /// on every call through the shared [`crate::view::theme::palette`] (never a duplicated colour
    /// table), so it tracks the palette picker's live changes. Used for BOTH the panel fill behind
    /// the ratatui texture ([`Self::paint`]) and the window clear ([`eframe::App::clear_color`]) so
    /// the right/bottom margin the floor()'d cell-grid can't cover matches the terminal canvas
    /// instead of eframe's default dark panel — killing the black corner wedge.
    fn theme_bg(&self) -> egui::Color32 {
        match crate::view::theme::palette(&self.shadow.rest.config).bg {
            ratatui::style::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
            // Every registered palette's `bg` is an RGB literal, so this arm is unreachable in
            // practice; fall back to the `dark` canvas (black) rather than panic on a stray Color.
            _ => egui::Color32::BLACK,
        }
    }

    /// Pre-size the soft backend to the egui panel, render the shadow through the UNCHANGED view,
    /// and present the rasterised image — steps (c-pre)/(c)/(d), shared by the attached + swapper
    /// frames (both render `self.shadow` — the swapper writes its hub onto it first).
    fn paint(&mut self, ui: &mut egui::Ui) {
        // (c-pre) pre-size the soft backend to the egui panel BEFORE drawing (fixes the
        // right/bottom-edge character bleed on resize). Pre-sizing here makes the widget's own
        // resize (inside `ui.add` below) a no-op, and ratatui's autoresize (inside `draw`) then
        // clears the backend + back-buffer and repaints EVERY cell cleanly this same frame. The
        // dims formula mirrors the widget's exactly (available px / cell px, clamped) so the two
        // never disagree — a mismatch would re-trigger the widget resize and re-introduce garble.
        {
            let avail = ui.available_size();
            let soft = &mut self.terminal.backend_mut().soft_backend;
            let cols = (avail.x.clamp(1.0, 10000.0) / soft.char_width.max(1) as f32) as u16;
            let rows = (avail.y.clamp(1.0, 10000.0) / soft.char_height.max(1) as f32) as u16;
            let cur = soft.buffer.area;
            if cols > 0 && rows > 0 && (cols != cur.width || rows != cur.height) {
                soft.resize(cols, rows);
            }
        }

        // (c) render the shadow into the software backend via the UNCHANGED view. The backend's
        // error type is `Infallible`, so the draw cannot actually fail.
        let _ = self.terminal.draw(|f| crate::view::draw(f, &self.shadow));

        // (c-bg) fill the WHOLE panel with the active theme's canvas bg BEFORE presenting the
        // texture. The grid is `floor(panel_px / cell_px)` cells, so the rasterised image falls a
        // few px short of the panel on the right + bottom; without this the leftover margin shows
        // eframe's default dark panel as a black corner wedge. Painting `ui.max_rect()` (the full
        // panel, zero rounding) with the SAME `palette.bg` `view::draw` uses (`theme_bg`) makes
        // that margin seam-free with the terminal canvas. Emitted before `ui.add` so the texture,
        // added at the top-left (the root `Ui` has no margin), draws ON TOP of the fill.
        let bg = self.theme_bg();
        ui.painter().rect_filled(ui.max_rect(), 0.0, bg);

        // (d) present the rasterised terminal image. The soft backend was pre-sized to the panel
        // above, so the widget's own resize (identical dims) is a no-op; it just uploads the
        // freshly-rendered pixmap as an egui texture.
        ui.add(self.terminal.backend_mut());
    }

    /// `/new` (`kill = false`) / `/new kill` (`kill = true`): detach the current daemon and attach
    /// a BRAND-NEW one. In the daemon-per-session world a daemon owns exactly ONE session, so
    /// `/new` makes another DAEMON, not a tab. Mirrors `client_run`'s `NewSession` arm.
    fn do_new_session(&mut self, kill: bool) {
        // Move the connection bits out (leaving `Closing` behind) so we can consume them.
        if let GuiState::Attached {
            req_tx,
            writer_handle,
            frame_rx: _,
        } = std::mem::replace(&mut self.state, GuiState::Closing)
        {
            if kill {
                // Reap the old daemon: queue `QuitDaemon` on the request channel BEFORE teardown
                // so the writer drains it (then the polite `Detach`) before the socket closes —
                // the old daemon releases its lock, drops its session, and unlinks its socket.
                // The client is its daemon's controller, so `QuitDaemon` is accepted.
                let _ = req_tx.send(ClientRequest::QuitDaemon);
            }
            self.teardown_attached(req_tx, writer_handle);
        }

        // Mint a fresh uuid and attach its daemon (spawned on demand by `connect_session`).
        // `prev_session` is deliberately LEFT as-is — a `/new` is not a swapper cancel, so there
        // is nothing to "return to". On failure, DEGRADE to the swapper rather than crash.
        let new_id = uuid::Uuid::new_v4().to_string();
        match connect_session(&self._rt, &new_id) {
            Ok(conn) => {
                self.current_session_id = Some(new_id);
                self.become_attached(conn);
            }
            Err(e) => {
                eprintln!("koma gui: could not start a new session {new_id}: {e:#}");
                self.state = GuiState::Swapper(enter_swapper(self.prev_session.as_deref()));
            }
        }
    }

    /// `/resume`: DETACH from this daemon (leaving it cooking — NO `QuitDaemon`) and open the local
    /// swapper. Record where a cancel returns (`prev_session`), then build the hub from fresh
    /// cross-daemon discovery (flagging the row we just left as foreground). Mirrors `client_run`'s
    /// `OpenSwapper` arm.
    fn do_open_swapper(&mut self) {
        if let GuiState::Attached {
            req_tx,
            writer_handle,
            frame_rx: _,
        } = std::mem::replace(&mut self.state, GuiState::Closing)
        {
            self.teardown_attached(req_tx, writer_handle);
        }
        self.prev_session = self.current_session_id.take();
        self.state = GuiState::Swapper(enter_swapper(self.prev_session.as_deref()));
    }

    /// Swapper PICK: attach to the chosen session-daemon (spawning it if needed). Assigning the
    /// new state drops the [`SwapperState`] (its `Drop` stops+joins the probe). On failure DEGRADE
    /// back to the swapper rebuilt from fresh discovery. Mirrors `client_run`'s `Pick` arm.
    fn do_pick(&mut self, target: String) {
        // Drop the swapper NOW (stops the probe) so it isn't sweeping during the connect.
        self.state = GuiState::Closing;
        match connect_session(&self._rt, &target) {
            Ok(conn) => {
                self.current_session_id = Some(target);
                self.become_attached(conn);
            }
            Err(e) => {
                eprintln!("koma gui: could not attach to session {target}: {e:#}");
                self.state = GuiState::Swapper(enter_swapper(self.prev_session.as_deref()));
            }
        }
    }

    /// Swapper CANCEL: reconnect to the session we left (`prev_session`). `koma gui` only ever
    /// enters the swapper from an IN-session `/resume`, so `prev_session` is always set; if it
    /// somehow isn't, close the window (nothing to return to). Mirrors `client_run`'s `Cancel` arm.
    fn do_cancel(&mut self, ctx: &egui::Context) {
        self.state = GuiState::Closing; // drop the SwapperState (stops the probe)
        match self.prev_session.take() {
            Some(prev) => match connect_session(&self._rt, &prev) {
                Ok(conn) => {
                    self.current_session_id = Some(prev);
                    self.become_attached(conn);
                }
                Err(e) => {
                    eprintln!("koma gui: could not reconnect to session {prev}: {e:#}");
                    self.state = GuiState::Swapper(enter_swapper(None));
                }
            },
            None => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Reset the shadow + per-connection seq tracking for a fresh attach, apply the handshake's
    /// prebuffered frames, and become [`GuiState::Attached`]. Shared by every re-attach
    /// (`/new`, swapper Pick, swapper Cancel-reconnect); mirrors `run_gui`'s initial seed.
    fn become_attached(&mut self, conn: Connection) {
        let Connection {
            frame_rx,
            req_tx,
            writer_handle,
            prebuffered,
            daemon_version: _,
        } = conn;

        // Reset the shadow + seq state so the new daemon's first Snapshot seeds cleanly (the old
        // session's transcript/cache must not bleed into the new attach).
        self.shadow = AppState::new(Mode::Chat);
        self.shadow.rest.fg_mut().status = "attaching…".into();
        self.expected = 0;
        self.seeded = false;
        self.awaiting_resync = false;
        self.last_sent_wrap_w = None;

        // Apply any pre-Hello prebuffered frames through the SAME `apply_frame` path, under a
        // SCOPED runtime-enter guard (folding `tokio::spawn`s an inert abort handle, which panics
        // without a reactor). Normally empty; the hand-off latches can't fire this early.
        {
            let _rt_guard = self._rt.enter();
            let mut select_requested = false;
            let mut open_swapper_requested = false;
            let mut new_session_requested: Option<bool> = None;
            for frame in prebuffered {
                apply_frame(
                    frame,
                    &mut self.shadow,
                    &mut self.expected,
                    &mut self.seeded,
                    &mut self.awaiting_resync,
                    &mut select_requested,
                    &mut open_swapper_requested,
                    &mut new_session_requested,
                    &req_tx,
                );
            }
        }

        self.state = GuiState::Attached {
            frame_rx,
            req_tx,
            writer_handle,
        };
    }

    /// Tear a live connection down cleanly — replicates `client::mod::teardown_connection`'s
    /// sequencing (the GUI holds the connection bits unpacked in [`GuiState::Attached`], not a
    /// `Connection`, so it can't call that fn directly): queue a polite `Detach`, drop `req_tx`
    /// to close the outbound channel (the writer then treats its next drain as final), then
    /// `block_on` a bounded join so the final frame(s) flush before the runtime is touched.
    /// MUST run OUTSIDE an entered runtime context (it `block_on`s).
    fn teardown_attached(
        &self,
        req_tx: Sender<ClientRequest>,
        writer_handle: tokio::task::JoinHandle<()>,
    ) {
        let _ = req_tx.send(ClientRequest::Detach);
        drop(req_tx);
        // Build the `timeout` (and its inner `Sleep`) INSIDE the async block, so it is
        // constructed while `block_on` has this runtime's context ENTERED. Passing
        // `tokio::time::timeout(..)` as the `block_on` ARGUMENT instead builds the `Sleep`
        // FIRST (Rust evaluates the argument before the call): with no runtime entered yet it
        // tries to register with the current thread's timer, finds none, and panics "there is
        // no reactor running". `block_on` only enters the context to DRIVE the future it is
        // handed — it can't rescue one that already panicked while being constructed.
        let _ = self
            ._rt
            .block_on(async move { tokio::time::timeout(WRITER_FLUSH_TIMEOUT, writer_handle).await });
    }

    /// Drive the local `/resume` swapper with this frame's egui input, returning the resolved
    /// [`SwapperOutcome`] if any. Reuses the terminal swapper's per-key handler
    /// ([`handle_swapper_key`]) verbatim — the GUI only translates egui events into the crossterm
    /// [`KeyEvent`]s that handler consumes (the SAME translation `forward_input` uses), so the
    /// picker behaves identically. Handled LOCALLY: there is no connection to `SendKey` to.
    ///
    /// Note the `Ctrl+X` two-step session NUKE: egui-winit swallows `Ctrl/⌘+X` into an
    /// [`egui::Event::Cut`] BEFORE any `Event::Key`, so it is re-synthesised here — otherwise the
    /// nuke could never fire in the GUI.
    fn swapper_input(&mut self, ctx: &egui::Context) -> Option<SwapperOutcome> {
        // Clone `prev_session` (the hub's foreground reference) up front to release the borrow
        // before taking `&mut self.state`.
        let current = self.prev_session.clone();
        let hub = match &mut self.state {
            GuiState::Swapper(sw) => &mut sw.hub,
            _ => return None,
        };

        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let outcome = match ev {
                // Literal typed text → plain `Char` keystrokes (feeds the history search). A char
                // never resolves the swapper, so the inner loop only ever mutates the hub.
                egui::Event::Text(s) => {
                    let mut out = None;
                    for c in s.chars() {
                        if c.is_control() {
                            continue;
                        }
                        out = handle_swapper_key(
                            hub,
                            &KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                            current.as_deref(),
                        );
                        if out.is_some() {
                            break;
                        }
                    }
                    out
                }
                // Named / modified keys (presses only) → the shared egui→crossterm mapping, then
                // the reused hub handler (Up/Down/Tab/Enter/Esc/Backspace/…).
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => map_key_event(key, modifiers)
                    .and_then(|ke| handle_swapper_key(hub, &ke, current.as_deref())),
                // Ctrl/⌘+X (session nuke) + Ctrl/⌘+C: egui-winit routes these to Cut/Copy BEFORE
                // any Event::Key, so re-synthesise them. Ctrl+C is inert in the swapper.
                egui::Event::Cut => handle_swapper_key(
                    hub,
                    &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
                    current.as_deref(),
                ),
                egui::Event::Copy => handle_swapper_key(
                    hub,
                    &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                    current.as_deref(),
                ),
                // Paste + mouse wheel are irrelevant to the picker.
                _ => None,
            };
            if let Some(o) = outcome {
                return Some(o);
            }
        }
        None
    }

    /// Translate this frame's egui input into daemon requests + client-local scroll — the GUI's
    /// analogue of `client::render::render_loop`'s input drain, adapted to egui's event model.
    /// egui only delivers input while the window is focused, so no extra focus gating.
    ///
    /// `req_tx` is passed in (a clone of the Attached state's sender) so this method borrows only
    /// `&mut self` for the shadow (scroll / render-ahead echo) without also reaching into
    /// `self.state`.
    ///
    /// ## egui → crossterm key mapping (the crux)
    ///
    /// - [`egui::Event::Text`]: literal typed text (shift/caps already baked in; egui never emits
    ///   it while Ctrl/⌘ is held). Each non-control char becomes a plain `Char(c)` keystroke.
    /// - [`egui::Event::Key`] (presses only): named keys always forward; CHARACTER keys are
    ///   DE-DUPLICATED (they also arrive as `Event::Text`) — forwarded here only under Ctrl. See
    ///   [`map_key_event`].
    /// - [`egui::Event::Copy`] / [`egui::Event::Cut`]: egui-winit intercepts `Ctrl/⌘+C`/`+X` into
    ///   these BEFORE any `Event::Key`, so re-synthesise them (`Ctrl+C` inert but forwarded for
    ///   parity; `Ctrl+X` kills a `$`-panel sub-agent / cancels queued steers).
    /// - [`egui::Event::Paste`]: bracketed paste AND `Ctrl/⌘+V` both land here → one
    ///   [`ClientRequest::Paste`] so the daemon runs the SAME `handle_paste` the local TUI uses.
    /// - Mouse wheel → CLIENT-LOCAL scroll of the shadow (the daemon owns no scroll).
    fn forward_input(&mut self, ctx: &egui::Context, req_tx: &Sender<ClientRequest>) {
        // One input-lock acquisition: this frame's events + the window-close request.
        let (events, closing) = ctx.input(|i| (i.events.clone(), i.viewport().close_requested()));

        // Graceful close: the user/OS asked to close the window. Deregister cleanly so the daemon
        // keeps cooking headless (same as a terminal detach). Best-effort — if it doesn't flush
        // before the runtime drops, the daemon still notices the socket closing. Do NOT early-
        // return: egui still wants a well-formed frame this pass.
        if closing {
            let _ = req_tx.send(ClientRequest::Detach);
        }

        // Set by a `/quit`-overlay ACTIVATION (`[k]`/`[d]`/Enter) — the GUI's "exit client" is
        // closing this window; done once after the loop so we don't send Close mid-drain.
        let mut should_close = false;

        for ev in events {
            match ev {
                // (1) Literal typed text → plain Char keystrokes.
                egui::Event::Text(s) => {
                    for c in s.chars() {
                        if c.is_control() {
                            continue;
                        }
                        self.dispatch_key(
                            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                            &mut should_close,
                            req_tx,
                        );
                    }
                }
                // (2) Named / modified keys (presses only; releases ignored).
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if let Some(ke) = map_key_event(key, modifiers) {
                        self.dispatch_key(ke, &mut should_close, req_tx);
                    }
                }
                // (3) Paste (bracketed OR Ctrl/⌘+V) → one Paste request.
                egui::Event::Paste(text) => {
                    let _ = req_tx.send(ClientRequest::Paste { text });
                }
                // (3b) Recover the Ctrl/⌘+C and Ctrl/⌘+X chords egui-winit swallowed into
                // Copy/Cut before any Event::Key.
                egui::Event::Copy => {
                    self.dispatch_key(
                        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                        &mut should_close,
                        req_tx,
                    );
                }
                egui::Event::Cut => {
                    self.dispatch_key(
                        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
                        &mut should_close,
                        req_tx,
                    );
                }
                // (4) Mouse wheel → client-local scroll (mirrors render_loop).
                egui::Event::MouseWheel { delta, .. } => {
                    if delta.y != 0.0
                        && matches!(self.shadow.mode(), Mode::Chat | Mode::Bash(_) | Mode::Todo(_))
                    {
                        // Viewer open → scroll IT (client-owned); else the main transcript.
                        let viewer = self.shadow.rest.agent_viewer.is_some();
                        let up = delta.y > 0.0;
                        for _ in 0..3 {
                            match (up, viewer) {
                                (true, true) => self.shadow.rest.agent_viewer_scroll_up(1),
                                (true, false) => self.shadow.rest.scroll_up(),
                                (false, true) => self.shadow.rest.agent_viewer_scroll_down(1),
                                (false, false) => self.shadow.rest.scroll_down(),
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if should_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Route ONE synthesised crossterm key exactly like `render_loop`'s `Event::Key` arm:
    /// intercept the mirrored `/quit` overlay and the client-owned sub-agent viewer scroll
    /// locally, otherwise render-ahead echo (`local_echo`) + forward as [`ClientRequest::SendKey`].
    fn dispatch_key(&mut self, key: KeyEvent, should_close: &mut bool, req_tx: &Sender<ClientRequest>) {
        // The `/quit` overlay's choices are CLIENT-process-lifecycle decisions (kill/detach THIS
        // window), so the client acts on them locally instead of forwarding — mapping the
        // terminal client's "exit client" onto closing this window. Nav keys are still forwarded
        // by `handle_quit_confirm_key` (the daemon owns the focus index).
        if matches!(self.shadow.mode(), Mode::QuitConfirm(_)) {
            let sel = if let Mode::QuitConfirm(s) = self.shadow.mode() {
                s.selected
            } else {
                0
            };
            match handle_quit_confirm_key(&key, req_tx, sel) {
                QuitConfirmKey::ExitClient => *should_close = true,
                QuitConfirmKey::Stay => {}
            }
            return;
        }

        // Full-screen sub-agent viewer scroll is CLIENT-owned: the headless daemon never renders,
        // so its `last_max_scroll` is always 0 and forwarding these keys would collapse the view
        // to top/bottom. Handle them locally against THIS client's fresh max (mirrors the mouse-
        // wheel local pattern); everything else (Esc closes the viewer daemon-side) still forwards.
        if self.shadow.rest.agent_viewer.is_some() {
            match key.code {
                KeyCode::Up => {
                    self.shadow.rest.agent_viewer_scroll_up(1);
                    return;
                }
                KeyCode::Down => {
                    self.shadow.rest.agent_viewer_scroll_down(1);
                    return;
                }
                KeyCode::PageUp => {
                    self.shadow.rest.agent_viewer_scroll_up(10);
                    return;
                }
                KeyCode::PageDown => {
                    self.shadow.rest.agent_viewer_scroll_down(10);
                    return;
                }
                KeyCode::Home => {
                    self.shadow.rest.agent_viewer_scroll_to_top();
                    return;
                }
                KeyCode::End => {
                    self.shadow.rest.agent_viewer_scroll_to_bottom();
                    return;
                }
                _ => {}
            }
        }

        // Render-ahead: echo the unambiguous composer edits NOW (self-correcting — the daemon's
        // authoritative InputChanged/Snapshot reconciles on a later frame), then forward verbatim.
        local_echo(&mut self.shadow, &key);
        let _ = req_tx.send(ClientRequest::SendKey(KeyWire::from(key)));
    }

    /// Whether anything in the shadow is currently animating and so needs a full-rate repaint
    /// (drives the idle-throttle in [`eframe::App::ui`]). True when a loading splash is up, the
    /// working "comet" clock is running, any session is working or mid-stream, or a toast is
    /// visible. Errs toward `true` — a false positive costs one wasted repaint; a false negative
    /// visibly stalls an animation.
    fn is_animating(&self) -> bool {
        matches!(self.shadow.mode(), Mode::Loading(_))
            || self.shadow.rest.work_since.is_some()
            || self.shadow.rest.fg().toast.is_some()
            || self
                .shadow
                .rest
                .sessions
                .iter()
                .any(|s| s.waiting || s.streaming.is_some())
    }
}

/// Attach to a session-daemon, spawning it if needed, and run the build-skew handshake — the
/// GUI's analogue of `client::mod::attach_session`, used for the initial connect AND every
/// session-op re-attach (`/new`, swapper Pick/Cancel).
///
/// It (1) ensures the session's daemon is RUNNING ([`super::manage::ensure_daemon_running`],
/// `resume=false`) — a no-op for a live session, a spawn for a fresh/on-disk id; (2) connects +
/// attaches + runs the `Hello` handshake; (3) on a CONFIRMED build-skew mismatch, restarts that
/// one stale daemon (AT MOST ONCE) and reconnects.
///
/// Build-skew difference from the terminal client: it has no crossterm surface for a restart
/// SPINNER, so it restarts QUIETLY ([`super::manage::restart_daemon`] with `quiet=true`) and just
/// logs, rather than drawing `restart_daemon_animated`'s braille spinner. Everything else mirrors
/// `attach_session` exactly.
///
/// Synchronous: `connect_attach_and_handshake` `block_on`s the connect internally, so this MUST
/// be called OUTSIDE an entered runtime context (the caller narrows its `_rt.enter()` guard so it
/// is not held here).
fn connect_session(rt: &tokio::runtime::Runtime, session_id: &str) -> anyhow::Result<Connection> {
    use crate::model::store;

    // Make sure a daemon owns this session before we connect. No-op when it is already live;
    // spawns + waits otherwise.
    super::manage::ensure_daemon_running(session_id, false).map_err(|e| {
        anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}")
    })?;

    let sock_path = store::daemon_sock_path(session_id)?;
    let my_fingerprint = store::build_fingerprint();
    let handle = rt.handle().clone();

    let mut conn = connect_attach_and_handshake(&handle, &sock_path)?;
    let mut already_restarted = false;
    while conn
        .daemon_version
        .as_deref()
        .is_some_and(|v| v != my_fingerprint)
    {
        if already_restarted {
            eprintln!(
                "koma gui: daemon still reports a different build after a restart; \
                 continuing against it"
            );
            break;
        }
        already_restarted = true;

        // Tear down the stale connection's bridge before restarting: drop our request sender
        // (the writer drains + exits) and let the reader observe the daemon's death as EOF. The
        // old writer handle drops on the reassignment below; both tasks self-terminate.
        drop(conn.req_tx);
        drop(conn.frame_rx);

        // GUI has no alt-screen surface for a restart spinner — restart quietly + log.
        eprintln!("koma gui: daemon reports a stale build; restarting it…");
        super::manage::restart_daemon(session_id, true)
            .map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}"))?;

        conn = connect_attach_and_handshake(&handle, &sock_path)?;
    }
    Ok(conn)
}

/// Build a fresh [`SwapperState`] for the `/resume` picker: a client-side hub from cross-daemon
/// discovery ([`build_local_hub`], one synchronous sweep for the first paint) plus a background
/// probe thread that re-sweeps every ~1s and ships the raw [`SessionStatus`] set to the ui thread
/// (which merges it via `apply_snapshot`). Mirrors [`super::client::swapper::run_swapper`]'s probe
/// exactly, minus the blocking loop — the GUI drives the hub one key/frame from `ui()`.
///
/// `current_id` is the session the client is (or was) attached to; it flags the hub's foreground
/// row and is threaded into the live-refresh merge so that flag stays correct across rebuilds.
fn enter_swapper(current_id: Option<&str>) -> SwapperState {
    /// How often the probe re-sweeps live-session discovery.
    const PROBE_INTERVAL: Duration = Duration::from_millis(1000);
    /// Granularity of the probe's interruptible sleep, so a stop is honored within ~100ms.
    const PROBE_SLEEP_STEP: Duration = Duration::from_millis(100);

    // First paint: build synchronously (one blocking discovery sweep — a deliberate `/resume`
    // keypress, matching the terminal client's up-front `build_local_hub`).
    let hub = build_local_hub(current_id);

    let stop = Arc::new(AtomicBool::new(false));
    let (snap_tx, snap_rx) = std::sync::mpsc::channel::<Vec<SessionStatus>>();
    let probe = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            // The blocking sweep — done HERE, never on the ui thread.
            let live = super::manage::list_live_sessions();
            // A send failure means the receiver hung up (swapper closed) — stop.
            if snap_tx.send(live).is_err() {
                return;
            }
            // Interruptible sleep: wake early if a stop was requested mid-interval.
            let mut slept = Duration::ZERO;
            while slept < PROBE_INTERVAL {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(PROBE_SLEEP_STEP);
                slept += PROBE_SLEEP_STEP;
            }
        })
    };

    SwapperState {
        hub,
        stop,
        snap_rx,
        probe: Some(probe),
    }
}

/// Map one egui [`egui::Event::Key`] press to the crossterm [`KeyEvent`] the daemon's controller
/// (and the swapper's hub handler) consume, or `None` if it should be dropped — an unbound key, or
/// a CHARACTER key that [`egui::Event::Text`] already delivered (de-duplicated: forwarded here only
/// when Ctrl is held). Applies the modifier mapping (⌘/`mac_cmd` and `ctrl` both → CONTROL, so
/// koma's Ctrl-based bindings work on macOS) and the `Shift+Tab → BackTab` special-case. Shared by
/// [`GuiApp::forward_input`] (attached) and [`GuiApp::swapper_input`] (`/resume`) so there is ONE
/// key-translation path, never two to drift.
fn map_key_event(key: egui::Key, modifiers: egui::Modifiers) -> Option<KeyEvent> {
    // macOS ⌘ (`mac_cmd`/`command`) AND `ctrl` both map to the daemon's Ctrl.
    let ctrl = modifiers.ctrl || modifiers.mac_cmd;
    let alt = modifiers.alt;
    let mut mods = KeyModifiers::empty();
    if modifiers.shift {
        mods |= KeyModifiers::SHIFT;
    }
    if ctrl {
        mods |= KeyModifiers::CONTROL;
    }
    if alt {
        mods |= KeyModifiers::ALT;
    }

    let (code, is_char) = map_egui_key(key)?;
    // De-dup: a character key already came through `Event::Text` (egui-winit only suppresses
    // Event::Text for Ctrl/⌘, not Alt), so only forward it here when Ctrl is held. Named keys
    // always forward.
    if is_char && !ctrl {
        return None;
    }
    // Shift+Tab is crossterm `BackTab` (reported WITHOUT the Shift bit).
    let (code, mods) = if code == KeyCode::Tab && modifiers.shift {
        (KeyCode::BackTab, mods & !KeyModifiers::SHIFT)
    } else {
        (code, mods)
    };
    Some(KeyEvent::new(code, mods))
}

/// Map an egui logical [`egui::Key`] to the crossterm [`KeyCode`] the daemon's controller
/// consumes, plus whether it is a "character key" — one that ALSO produces an [`egui::Event::Text`]
/// and so must be de-duplicated (forwarded from the `Event::Key` path only under Ctrl/Alt).
/// Character keys map to their BASE char (lowercase letters), so `Ctrl+C` → `Char('c')` + CONTROL,
/// matching `controller::input::is_ctrl`. Returns `None` for keys koma has no binding for.
fn map_egui_key(key: egui::Key) -> Option<(KeyCode, bool)> {
    use egui::Key as K;
    let named = |c: KeyCode| Some((c, false));
    let ch = |c: char| Some((KeyCode::Char(c), true));
    match key {
        // --- named keys: always forwarded; never emit Event::Text ---
        K::Enter => named(KeyCode::Enter),
        K::Escape => named(KeyCode::Esc),
        K::Tab => named(KeyCode::Tab), // Shift+Tab → BackTab is applied by the caller
        K::Backspace => named(KeyCode::Backspace),
        K::Delete => named(KeyCode::Delete),
        K::Insert => named(KeyCode::Insert),
        K::Home => named(KeyCode::Home),
        K::End => named(KeyCode::End),
        K::PageUp => named(KeyCode::PageUp),
        K::PageDown => named(KeyCode::PageDown),
        K::ArrowUp => named(KeyCode::Up),
        K::ArrowDown => named(KeyCode::Down),
        K::ArrowLeft => named(KeyCode::Left),
        K::ArrowRight => named(KeyCode::Right),
        K::F1 => named(KeyCode::F(1)),
        K::F2 => named(KeyCode::F(2)),
        K::F3 => named(KeyCode::F(3)),
        K::F4 => named(KeyCode::F(4)),
        K::F5 => named(KeyCode::F(5)),
        K::F6 => named(KeyCode::F(6)),
        K::F7 => named(KeyCode::F(7)),
        K::F8 => named(KeyCode::F(8)),
        K::F9 => named(KeyCode::F(9)),
        K::F10 => named(KeyCode::F(10)),
        K::F11 => named(KeyCode::F(11)),
        K::F12 => named(KeyCode::F(12)),
        // --- character keys: de-duplicated (Event::Text already delivered them) ---
        K::Space => ch(' '),
        K::A => ch('a'),
        K::B => ch('b'),
        K::C => ch('c'),
        K::D => ch('d'),
        K::E => ch('e'),
        K::F => ch('f'),
        K::G => ch('g'),
        K::H => ch('h'),
        K::I => ch('i'),
        K::J => ch('j'),
        K::K => ch('k'),
        K::L => ch('l'),
        K::M => ch('m'),
        K::N => ch('n'),
        K::O => ch('o'),
        K::P => ch('p'),
        K::Q => ch('q'),
        K::R => ch('r'),
        K::S => ch('s'),
        K::T => ch('t'),
        K::U => ch('u'),
        K::V => ch('v'),
        K::W => ch('w'),
        K::X => ch('x'),
        K::Y => ch('y'),
        K::Z => ch('z'),
        K::Num0 => ch('0'),
        K::Num1 => ch('1'),
        K::Num2 => ch('2'),
        K::Num3 => ch('3'),
        K::Num4 => ch('4'),
        K::Num5 => ch('5'),
        K::Num6 => ch('6'),
        K::Num7 => ch('7'),
        K::Num8 => ch('8'),
        K::Num9 => ch('9'),
        K::Colon => ch(':'),
        K::Comma => ch(','),
        K::Backslash => ch('\\'),
        K::Slash => ch('/'),
        K::Pipe => ch('|'),
        K::Questionmark => ch('?'),
        K::Exclamationmark => ch('!'),
        K::OpenBracket => ch('['),
        K::CloseBracket => ch(']'),
        K::OpenCurlyBracket => ch('{'),
        K::CloseCurlyBracket => ch('}'),
        K::Backtick => ch('`'),
        K::Minus => ch('-'),
        K::Period => ch('.'),
        K::Plus => ch('+'),
        K::Equals => ch('='),
        K::Semicolon => ch(';'),
        K::Quote => ch('\''),
        // Everything else (Copy/Cut/Paste keys, higher F-keys, BrowserBack, …): not a koma
        // binding — dropped here. Plain punctuation still reaches the composer via Event::Text.
        _ => None,
    }
}

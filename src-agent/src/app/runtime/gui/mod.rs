//! Desktop GUI client (`koma gui`) — Wave 3: interactive (display + input).
//!
//! The GUI is "just another daemon client". It reuses the terminal client's non-terminal
//! machinery verbatim — [`connect_attach_and_handshake`] for the socket bridge, [`apply_frame`]
//! for folding streamed snapshots/deltas into a shadow [`AppState`], and the UNCHANGED
//! [`crate::view::draw`] for rendering — and swaps ONLY the crossterm render loop for an
//! [`eframe`] window. The ratatui frame is rasterised by [`soft_ratatui`] (a software backend)
//! and shown as an egui widget through [`egui_ratatui::RataguiBackend`].
//!
//! Wave 3 wires input: each frame the app reads egui's input events and forwards them to the
//! daemon through `req_tx` exactly like the terminal client's `render_loop` does — keystrokes as
//! [`ClientRequest::SendKey`], bracketed/`Ctrl+V` paste as [`ClientRequest::Paste`] — plus the
//! CLIENT-owned mouse-wheel scroll on the shadow. Reused verbatim (no second logic path):
//! [`local_echo`] for render-ahead composer echo and [`handle_quit_confirm_key`] for the mirrored
//! `/quit` overlay. The daemon-side `/resume` `/new` `/select` hand-offs stay deferred (see below).

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::cli::Opts;
use crate::ipc::proto::{ClientRequest, DaemonFrame, KeyWire};

// Reuse the terminal client's key handling verbatim (both bumped to `pub(crate)` — visibility
// only; see `client::mod`): the render-ahead composer echo + the mirrored `/quit` overlay keys.
use super::client::input::{handle_quit_confirm_key, local_echo, QuitConfirmKey};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use egui_ratatui::RataguiBackend;
use ratatui::Terminal;
use soft_ratatui::rusttype::Font;
use soft_ratatui::{EmbeddedTTF, SoftBackend};

// Reuse the terminal client's connect primitive + shadow-folding logic (both bumped to
// `pub(crate)` — visibility only). The GUI must NOT re-implement either: a second folding
// path would drift from the terminal client's.
use super::client::connect::{connect_attach_and_handshake, Connection};
use super::client::shadow::{apply_frame, reconcile_work_clock};
// The writer-flush timeout `client::mod::teardown_connection` bounds its final join by —
// reused verbatim (not redefined) so `GuiApp::on_exit` mirrors that exact sequencing.
// `bridge` bumped `mod` -> `pub(crate) mod` in `client::mod` for this (visibility only).
use super::client::bridge::WRITER_FLUSH_TIMEOUT;

/// The soft-ratatui backend flavour we render with: TrueType (RustType) rendering of the
/// bundled Nerd Font, so box-drawing, powerline separators, braille spinners, and Nerd-Font
/// private-use icons all render (the embedded-graphics bitmap atlas covers only the first two).
type GuiBackend = RataguiBackend<EmbeddedTTF>;

/// The bundled monospace Nerd Font (JetBrainsMono Nerd Font Mono — OFL 1.1, see
/// `assets/fonts/LICENSE`). Baked into the binary via `include_bytes!` so `koma gui` needs no
/// font installed on the host; the "Mono" variant has fixed advance widths, required for a
/// character-cell grid.
const FONT_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMonoNerdFontMono-Bold.ttf");
const FONT_ITALIC: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMonoNerdFontMono-Italic.ttf");

/// Pixel size passed to `rusttype`/`embedded-ttf` for cell rasterisation; the cell width/height
/// (in pixels) is derived from this by `SoftBackend::<EmbeddedTTF>::new` itself.
const FONT_SIZE_PX: u32 = 16;

/// Entry point for `koma gui`. Ensures a session daemon exists, attaches over its unix
/// socket, and renders that daemon's foreground session in an eframe window (read-only).
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
    crate::app::ensure_daemon_running(&session_id, false).map_err(|e| {
        anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}")
    })?;

    // A multi-thread tokio runtime drives the two socket bridge tasks (reader + writer) that
    // `connect_attach_and_handshake` spawns. Those tasks must live for the WHOLE app, so the
    // runtime is moved into `GuiApp` below and only dropped when the window closes.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // Connect + attach + build-skew handshake. Synchronous (it `block_on`s the connect
    // internally), so it must run on this plain main thread — NOT inside an entered runtime
    // context — exactly like the terminal client's `attach_session`.
    let sock_path = store::daemon_sock_path(&session_id)?;
    let Connection {
        frame_rx,
        req_tx,
        writer_handle,
        prebuffered,
        daemon_version,
    } = connect_attach_and_handshake(&handle, &sock_path)?;

    // Build-skew: the terminal client auto-restarts a stale daemon (its restart spinner needs
    // a crossterm terminal we don't have here). We only WARN on a mismatch — the daemon we just
    // spawned is fresh, so a mismatch is only possible against a pre-existing stale one.
    // TODO(later): GUI-side stale-daemon restart (needs an egui restart-spinner surface).
    let my_fingerprint = store::build_fingerprint();
    if let Some(v) = daemon_version.as_deref() {
        if v != my_fingerprint {
            eprintln!(
                "koma gui: daemon reports a different build than this client; \
                 rendering against it anyway"
            );
        }
    }

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

    // Apply any frames the handshake pulled off the wire while hunting for `Hello` (normally
    // none) BEFORE the live drain, through the SAME `apply_frame` path, so the seq stream
    // stays gap-free. The hand-off latches are throwaways — none can occur this early.
    {
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
    // 1100x720 window at FONT_SIZE_PX — the RataguiBackend widget resizes the soft backend to
    // the egui panel on every paint, and ratatui's own autoresize picks the new grid up on the
    // next `terminal.draw`, so this only matters for the very first frame.
    //
    // Fonts: TrueType rendering (RustType, via soft_ratatui's `embedded-ttf` backend) of the
    // bundled Nerd Font — renders box-drawing, powerline separators, braille spinners, AND
    // Nerd-Font private-use icons (the embedded-graphics bitmap atlas covered only the first
    // two). `try_from_bytes` only fails on a malformed font file, which `include_bytes!`'d
    // known-good TTFs never are.
    let font_regular = Font::try_from_bytes(FONT_REGULAR).expect("bundled regular TTF is valid");
    let font_bold = Font::try_from_bytes(FONT_BOLD).expect("bundled bold TTF is valid");
    let font_italic = Font::try_from_bytes(FONT_ITALIC).expect("bundled italic TTF is valid");
    let soft = SoftBackend::<EmbeddedTTF>::new(
        120,
        38,
        FONT_SIZE_PX,
        font_regular,
        Some(font_bold),
        Some(font_italic),
    );
    let terminal = Terminal::new(RataguiBackend::new("koma", soft))?;

    let app = GuiApp {
        shadow,
        terminal,
        frame_rx,
        req_tx: Some(req_tx),
        expected,
        seeded,
        awaiting_resync,
        last_sent_wrap_w: None,
        writer_handle: Some(writer_handle),
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

/// The eframe app: a shadow [`AppState`] fed by the daemon's frames, rendered each repaint
/// through the unchanged ratatui view into a software backend shown as an egui widget.
struct GuiApp {
    /// Shadow state rebuilt purely from daemon snapshots/deltas.
    shadow: AppState,
    /// Software ratatui backend wrapped as an egui widget.
    terminal: Terminal<GuiBackend>,
    /// Incoming daemon frames (reader task -> this ui thread). std mpsc, so `Send`.
    frame_rx: std::sync::mpsc::Receiver<DaemonFrame>,
    /// Outgoing client requests: keystrokes ([`ClientRequest::SendKey`]), paste, editor
    /// wrap width, the graceful-close `Detach`, and `apply_frame`'s Resync flow through here.
    /// `Option` so [`Self::on_exit`] can `.take()` (drop) the sender to close the channel —
    /// that is what tells the writer task (`bridge::writer_task`) its next drain is the
    /// final one. `None` ONLY after `on_exit` has run; every other call site goes through
    /// [`Self::req_tx`], which assumes `Some`.
    req_tx: Option<std::sync::mpsc::Sender<ClientRequest>>,
    /// Per-connection seq expectation (see `run_gui`).
    expected: u64,
    seeded: bool,
    awaiting_resync: bool,
    /// Last agents-editor wrap width sent to the daemon, so `EditorWrapW` is only re-sent on a
    /// change (and re-sent on a fresh editor open, when the daemon's editor is back at
    /// `usize::MAX`). Mirrors `render_loop`'s `last_sent_wrap_w`; `None` when not in the editor.
    last_sent_wrap_w: Option<usize>,
    /// The writer task's handle. `Option` so [`Self::on_exit`] can `.take()` and JOIN it
    /// (bounded by [`WRITER_FLUSH_TIMEOUT`]) — mirroring `client::mod::teardown_connection` —
    /// so the final enqueued frame (the window-close `Detach`, or a `/quit` overlay's `[k]`
    /// `QuitDaemon`) is actually written to the socket before the runtime tears down. Before
    /// `on_exit` runs this is just held so the writer task isn't dropped early.
    writer_handle: Option<tokio::task::JoinHandle<()>>,
    /// Owns the runtime so the bridge tasks outlive the window. Declared LAST so it drops
    /// after the channels/handle, cancelling the tasks only once nothing else references them.
    /// Also used directly by [`Self::on_exit`] to `block_on` the bounded writer join.
    _rt: tokio::runtime::Runtime,
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use std::sync::mpsc::TryRecvError;

        // --- (a) drain EVERY queued daemon frame non-blocking, folding each into the shadow.
        // TODO(later): the daemon-side hand-off latches (`/select` transcript dump, `/resume`
        // swapper, `/new` session) are collected but still IGNORED — the GUI has no reconnect /
        // session-swap machinery yet (the terminal client's `ClientState::Swapper` path), so a
        // `/resume` or `/new` typed in the GUI is a no-op for now rather than swapping sockets.
        // A closed socket => the daemon is gone.
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        let mut disconnected = false;
        loop {
            match self.frame_rx.try_recv() {
                Ok(frame) => {
                    apply_frame(
                        frame,
                        &mut self.shadow,
                        &mut self.expected,
                        &mut self.seeded,
                        &mut self.awaiting_resync,
                        &mut select_requested,
                        &mut open_swapper_requested,
                        &mut new_session_requested,
                        // Direct field projection (not `self.req_tx()`): the call above also
                        // borrows `&mut self.shadow`/`&mut self.expected`/etc, and a method call
                        // would borrow all of `self`, conflicting with those. A field projection
                        // borrows only `self.req_tx`, which the disjoint-field borrow checker
                        // allows alongside the other fields' mutable borrows.
                        self.req_tx
                            .as_ref()
                            .expect("req_tx is only taken during GuiApp::on_exit"),
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        // --- (b) advance the LOCAL-clock animations so the comet + loading spinner tick
        // between snapshots (reimplements client::render::advance_local_animations so we
        // never touch the crossterm render loop).
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

        // --- (c) render the shadow into the software backend via the UNCHANGED view. The
        // backend's error type is `Infallible`, so the draw cannot actually fail.
        let _ = self.terminal.draw(|f| crate::view::draw(f, &self.shadow));

        // --- (d) present the rasterised terminal image. The widget resizes the soft backend
        // to the panel; ratatui's autoresize picks up the new grid on the next draw.
        ui.add(self.terminal.backend_mut());

        // No point talking to a daemon that is already gone; the close below runs regardless.
        if !disconnected {
            // --- (e) forward the agents-editor wrap width (mirror render_loop's c-bis) ---
            // The shadow's agents editor publishes its `wrap_w` via interior mutability during
            // the draw above; the daemon's editor starts at `usize::MAX` (never rendered), so
            // send the client-side value whenever it changes. Reset to `None` when NOT in the
            // editor so each fresh open re-sends (the daemon's freshly-opened editor is back at
            // `usize::MAX`). Read AFTER the draw so `wrap_w` reflects this frame's layout.
            let wrap_now: Option<usize> = if let Mode::Agents(ref a) = self.shadow.mode() {
                a.editor.as_ref().map(|(_, ed)| ed.wrap_w.get())
            } else {
                None
            };
            match wrap_now {
                Some(w) if self.last_sent_wrap_w != Some(w) => {
                    self.last_sent_wrap_w = Some(w);
                    let _ = self.req_tx().send(ClientRequest::EditorWrapW(w));
                }
                Some(_) => {}
                None => self.last_sent_wrap_w = None,
            }

            // --- (f) translate this frame's egui input into daemon requests (Wave 3) ---
            self.forward_input(ui.ctx());
        }

        if disconnected {
            // Daemon socket closed (session ended / daemon killed): close the window.
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            // Keep repainting so streaming tokens + spinners animate at the monitor's cadence.
            ui.ctx().request_repaint();
        }
    }

    /// Flush the writer task's final queued frame before the runtime tears down — mirrors
    /// `client::mod::teardown_connection` exactly, ported to eframe's shutdown hook.
    ///
    /// `forward_input`'s `closing` arm (and the `/quit` overlay's `[k]`/`[d]` paths) only
    /// ENQUEUE their shutdown request (`Detach`, or `QuitDaemon` ahead of it) on the `req_tx`
    /// mpsc; `bridge::writer_task` drains that queue to the socket on its own `REQ_POLL`
    /// tick, but only notices it should do a FINAL drain-and-return once the channel actually
    /// CLOSES. Without joining that task here, `eframe::run_native` returning drops `_rt` —
    /// which cancels the writer mid-poll — before the last frame is necessarily on the wire,
    /// so the `[k]` kill-this-daemon path could silently lose its `QuitDaemon`.
    ///
    /// Sequencing (identical to `teardown_connection`): drop `req_tx` FIRST — closing the
    /// channel is what makes the writer's next `try_recv` batch report `Disconnected` and
    /// treat what it just drained as final — THEN `block_on` a bounded join of the writer
    /// handle so this call doesn't return until that batch is actually written (or
    /// [`WRITER_FLUSH_TIMEOUT`] elapses against a wedged socket, so exit can never hang).
    /// Both fields are taken via `.take()`, so a second `on_exit` call is a harmless no-op.
    fn on_exit(&mut self) {
        drop(self.req_tx.take());
        if let Some(handle) = self.writer_handle.take() {
            let _ = self._rt.block_on(tokio::time::timeout(WRITER_FLUSH_TIMEOUT, handle));
        }
    }
}

impl GuiApp {
    /// Translate this frame's egui input into daemon requests + client-local scroll — the GUI's
    /// analogue of `client::render::render_loop`'s input drain (step d), adapted to egui's event
    /// model. egui only delivers input while the window is focused, so no extra focus gating.
    ///
    /// ## egui → crossterm key mapping (the crux)
    ///
    /// - [`egui::Event::Text`]: literal typed text (shift/caps already baked in; egui never emits
    ///   it while Ctrl/⌘ is held). Each non-control char becomes a plain `Char(c)` keystroke —
    ///   this is how normal typing reaches the composer.
    /// - [`egui::Event::Key`] (presses only; releases ignored): named keys (Enter/arrows/F-keys/…)
    ///   are always forwarded; CHARACTER keys (letters/digits/space/punct) are DE-DUPLICATED —
    ///   they also arrive as `Event::Text`, so we forward them here ONLY when Ctrl is held (e.g.
    ///   `Ctrl+C`, `Ctrl+R`), as `Char(<lowercase>)` + the modifier (crossterm convention).
    ///   egui-winit only suppresses `Event::Text` for Ctrl/⌘ (NOT Alt), so a plain `Alt+<letter>`
    ///   still arrives as `Event::Text` too — forwarding it again here would double-dispatch it;
    ///   Alt-held character keys are left to the `Event::Text` path like normal typing, with only
    ///   their ALT bit — set below — lost (koma has no Alt-based character bindings today).
    ///   Modifiers: egui `shift`→SHIFT, `alt`→ALT, and (`ctrl` OR macOS `mac_cmd`/`command`)→
    ///   CONTROL, so ⌘ and Ctrl both drive koma's Ctrl-based bindings. `Shift+Tab`→`BackTab`.
    /// - [`egui::Event::Copy`] / [`egui::Event::Cut`]: egui-winit intercepts `Ctrl/⌘+C` and
    ///   `Ctrl/⌘+X` into these and returns BEFORE emitting an `Event::Key`, so on Linux those
    ///   chords never arrive as keys. Re-synthesise them (`Ctrl+C`, `Ctrl+X`) so koma's bindings
    ///   still fire — `Ctrl+X` kills a `$`-panel sub-agent / cancels queued steers; `Ctrl+C` is
    ///   inert but forwarded for parity with the terminal client (which forwards every key).
    /// - [`egui::Event::Paste`]: bracketed paste AND `Ctrl/⌘+V` both land here (egui-winit routes
    ///   `Ctrl+V` to Paste with the clipboard TEXT). One [`ClientRequest::Paste`] so the daemon
    ///   runs the SAME `handle_paste` the local TUI uses (image-file-path ingestion, CRLF-
    ///   normalised multiline). KNOWN LIMITATION: koma's `Ctrl+V` grabs a raw clipboard IMAGE
    ///   (`wl-paste`/`xclip`); egui only surfaces clipboard TEXT (an image-only clipboard yields
    ///   NO event), so raw-image paste is unavailable in the GUI until a dedicated affordance.
    /// - Mouse wheel → CLIENT-LOCAL scroll of the shadow (the daemon owns no scroll). egui's
    ///   `+delta.y` = content moves down = older content, matching the terminal's `ScrollUp →
    ///   scroll_up()`. Mirrors render_loop: 3 lines per wheel event, to the sub-agent viewer when
    ///   open else the transcript, only in the transcript modes.
    fn forward_input(&mut self, ctx: &egui::Context) {
        // One input-lock acquisition: this frame's events + the window-close request.
        let (events, closing) = ctx.input(|i| (i.events.clone(), i.viewport().close_requested()));

        // Graceful close: the user/OS asked to close the window. Deregister cleanly so the daemon
        // keeps cooking headless (same as a terminal detach). Best-effort — if it doesn't flush
        // before the runtime drops, the daemon still notices the socket closing. Do NOT early-
        // return: egui still wants a well-formed frame this pass.
        if closing {
            let _ = self.req_tx().send(ClientRequest::Detach);
        }

        // Set by a `/quit`-overlay ACTIVATION (`[k]`/`[d]`/Enter) — the GUI's "exit client" is
        // closing this window; done once after the loop so we don't send Close mid-drain.
        let mut should_close = false;

        for ev in events {
            match ev {
                // (1) Literal typed text → plain Char keystrokes (see the de-dup note in (2)).
                egui::Event::Text(s) => {
                    for c in s.chars() {
                        if c.is_control() {
                            continue;
                        }
                        self.dispatch_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), &mut should_close);
                    }
                }
                // (2) Named / modified keys. Ignore releases (`pressed: false`); `KeyEvent::new`
                // sets `KeyEventKind::Press`, which the daemon's `handle_key` requires.
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    // macOS ⌘ (`mac_cmd`/`command`) AND `ctrl` both map to the daemon's Ctrl —
                    // koma's keybindings are Ctrl-based, so treating ⌘ as Ctrl makes them work
                    // on macOS too.
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
                    if let Some((code, is_char)) = map_egui_key(key) {
                        // De-dup: a character key already came through `Event::Text` (egui-winit
                        // only suppresses Event::Text for Ctrl/⌘, not Alt — see the fn doc), so
                        // only forward it here when Ctrl is held. Named keys always forward.
                        if is_char && !ctrl {
                            continue;
                        }
                        // Shift+Tab is crossterm `BackTab` (reported WITHOUT the Shift bit).
                        let (code, mods) = if code == KeyCode::Tab && modifiers.shift {
                            (KeyCode::BackTab, mods & !KeyModifiers::SHIFT)
                        } else {
                            (code, mods)
                        };
                        self.dispatch_key(KeyEvent::new(code, mods), &mut should_close);
                    }
                }
                // (3) Paste (bracketed OR Ctrl/⌘+V) → one Paste request.
                egui::Event::Paste(text) => {
                    let _ = self.req_tx().send(ClientRequest::Paste { text });
                }
                // (3b) Recover the Ctrl/⌘+C and Ctrl/⌘+X chords egui-winit swallowed into
                // Copy/Cut before any Event::Key (see the fn doc).
                egui::Event::Copy => {
                    self.dispatch_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &mut should_close);
                }
                egui::Event::Cut => {
                    self.dispatch_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL), &mut should_close);
                }
                // (4) Mouse wheel → client-local scroll (mirrors render_loop 312-332).
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
    fn dispatch_key(&mut self, key: KeyEvent, should_close: &mut bool) {
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
            match handle_quit_confirm_key(&key, self.req_tx(), sel) {
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
        let _ = self.req_tx().send(ClientRequest::SendKey(KeyWire::from(key)));
    }

    /// Borrow the outbound sender. `Option` only so [`GuiApp::on_exit`] can `.take()` it to
    /// close the channel; every OTHER call site runs strictly before `on_exit` (which is
    /// eframe's terminal shutdown hook), so `Some` always holds here.
    fn req_tx(&self) -> &std::sync::mpsc::Sender<ClientRequest> {
        self.req_tx
            .as_ref()
            .expect("req_tx is only taken during GuiApp::on_exit")
    }
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

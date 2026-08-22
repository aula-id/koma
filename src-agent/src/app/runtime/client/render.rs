use std::io::{stdout, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{Mode, QuitConfirmState, SessionHub, SessionKind};
use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, KeyWire};
use crate::view;

use super::input::{handle_quit_confirm_key, local_echo, send_overlay_cancel, QuitConfirmKey};
use super::project::{push_hub, serialize_and_push};
use super::project_config::{push_config, ConfigProjection};
use super::push_proto::{
    push_file_diff, push_switching, push_usage_preview, PushAttachment, PushBashJob, PushCooking,
    PushEnvelope, PushFileChange, PushHistory, PushMcpServer, PushModel, PushMsg, PushPalette,
    PushPaletteInfo, PushPendingCall, PushPlanTodo, PushProvider, PushRoute, PushSubAgent,
    PushToolCall,
};
use super::shadow::{apply_frame, reconcile_work_clock};

/// Local TTL for a toast reconstructed from a [`StateDelta::Toast`]. The daemon's
/// toast `Instant` is daemon-local and never crosses the wire (see `ipc::snapshot`);
/// the client re-derives its own dismissal timer here, matching the ~4s feel of the
/// local TUI's toasts.
pub(super) const TOAST_TTL: Duration = Duration::from_secs(4);

/// Target frame budget: ~60fps. Each loop iteration paints once and then sleeps the
/// remainder of this budget, so animations advance smoothly from the local clock and
/// the client never busy-spins. This is the FIXED cadence the render loop runs at,
/// independent of the daemon's frame rate (the socket is drained non-blocking).
pub(super) const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// Why [`render_loop`] returned — i.e. what the client run-loop in
/// [`super::client_run`] should do next.
///
/// Three outcomes:
/// - [`Exit`](ClientTransition::Exit): leave the client (detach, the `/quit` overlay's
///   ExitClient choice, or the daemon's socket closing).
/// - [`OpenSwapper`](ClientTransition::OpenSwapper): the daemon sent a
///   [`crate::ipc::proto::DaemonEvent::OpenSwapper`] (a `/resume` hand-off), so
///   `client_run` should DETACH from the current daemon (leave it cooking) and run the
///   local session swapper standalone; on pick it attaches to the chosen daemon, on
///   cancel it reconnects to the one it just left.
/// - [`NewSession`](ClientTransition::NewSession): the daemon sent a
///   [`crate::ipc::proto::DaemonEvent::NewSession`] (a `/new` hand-off), so `client_run`
///   should DETACH from the current daemon — or, on `kill`, send `QuitDaemon` first to reap
///   it — and attach a freshly minted brand-new session-daemon.
pub(crate) enum ClientTransition {
    /// Tear the client down and return from `client_run` (detach / ExitClient /
    /// frame channel disconnected). `kill` is true when the quit-confirm overlay's
    /// `[k]` was activated — the client must wait for the daemon to die before
    /// returning so a reopened session never reattaches to the dying process.
    Exit { kill: bool },
    /// Detach from the current daemon and open the local daemon swapper (`/resume`).
    OpenSwapper,
    /// Detach (or kill, on `kill`) the current daemon and attach a brand-new
    /// session-daemon (`/new` / `/new kill`). The bool is the `/new kill` flag.
    NewSession { kill: bool },
    /// Detach from the local daemon and connect to a remote host via SSH.
    /// Carries the `user@host[:port]` address string and optional SSH key path.
    ConnectRemote {
        target: String,
        key: Option<String>,
        new_session: bool,
        session_id: Option<String>,
        host_id: Option<String>,
    },
}

/// The synchronous render loop, decoupled from the socket and paced at ~60fps.
///
/// Each frame, in order: (a) drain ALL pending [`DaemonFrame`]s non-blocking and
/// apply them (snapshot/delta or seq-gap -> Resync); (b) advance animations from a
/// LOCAL monotonic clock (reconcile the comet's `work_since`, re-anchor the loading
/// spinner) — never from daemon ticks; (c) repaint the shadow UNCONDITIONALLY (the
/// ratatui buffer diff makes an unchanged frame ~free); (d) poll terminal input with
/// a ZERO timeout and handle it (local echo for the plain composer edits, forward the
/// rest). The loop NEVER blocks on the socket: if no frame arrived it still paints and
/// animations still advance. Returns when the client detaches (via the `/quit`
/// overlay) or the socket closes. (Ctrl-C is fully inert now.)
///
/// Returns a [`ClientTransition`] telling [`super::client_run`] what to do next:
/// [`ClientTransition::Exit`] to leave the client, or [`ClientTransition::OpenSwapper`]
/// when the daemon signals a `/resume` (so `client_run` detaches + opens the swapper).
pub(super) fn render_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    frame_rx: &Receiver<DaemonFrame>,
    req_tx: &Sender<ClientRequest>,
    prebuffered: Vec<DaemonFrame>,
) -> Result<ClientTransition> {
    use std::sync::mpsc::TryRecvError;

    // The shadow is a real AppState reconstructed purely from frames. It starts in
    // a neutral Chat with a single empty session; the first Snapshot replaces it.
    let mut shadow = AppState::new(Mode::Chat);
    // Until the first Snapshot lands the shadow is empty — show a clear status so
    // the screen isn't a blank "ready". Status is per-session (C6); the shadow has a
    // single placeholder session here, so write it on the foreground.
    shadow.rest.fg_mut().status = "attaching…".into();

    // Tracks the last wrap width we sent to the daemon for the agents editor, so we
    // only send `EditorWrapW` when it changes and always re-send on a fresh editor open
    // (the daemon's newly-opened editor starts at usize::MAX). Reset to None whenever
    // the shadow is NOT in the agents full-screen editor so each fresh open re-sends.
    let mut last_sent_wrap_w: Option<usize> = None;

    // One-shot: re-apply the session's mouse_capture setting after the first Snapshot
    // populates the shadow. Until then, the initial Auto (from `client_run`) is active.
    let mut mouse_capture_synced = false;

    // Per-connection seq tracking (critique #1). `expected` is the seq the NEXT
    // frame should carry. `0` means "not yet seeded" — the first frame seeds it.
    let mut expected: u64 = 0;
    let mut seeded = false;
    // While true, every frame except a fresh Snapshot is dropped: a gap was seen and
    // a Resync was sent, so the shadow is stale until the full snapshot rebuilds it.
    let mut awaiting_resync = false;

    // GUI-live palette sync (see `crate::app::runtime::gui::run_gui`, which sets
    // `KOMA_GUI=1` on the pty child it spawns): when running under the GUI host,
    // emit a private OSC 5380 carrying the current palette's canvas bg whenever it
    // changes, so the webview can repaint its window gutter to match live. Checked
    // once here (env doesn't change mid-run); `last_gui_bg` diffs so the OSC is only
    // emitted on an actual palette change, not every ~60fps frame.
    let gui_mode = std::env::var("KOMA_GUI").is_ok();
    let mut last_gui_theme: Option<(ratatui::style::Color, ratatui::style::Color)> = None;

    // Local exit feedback: when the user activates quit/detach in the overlay,
    // the client renders its OWN braille-spinner exit screen (the daemon may
    // still be shutting down, but the user sees immediate feedback). `true`
    // means `kill` (the daemon was asked to shut down vs just detach). The
    // render loop continues draining frames until the socket disconnects or a
    // timeout fires, showing the exit state each frame.
    let mut pending_exit: Option<bool> = None;
    let mut exit_started: std::time::Instant = std::time::Instant::now();
    /// Safety timeout: if the daemon doesn't disconnect within this duration,
    /// the client forces exit to prevent hanging forever.
    const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

    // Apply any frames the pre-render handshake pulled off the wire while hunting for
    // `Hello` (task #142) BEFORE the live drain, through the SAME `apply_frame` path so
    // seq seeding + snapshot/delta handling are identical. Normally empty (the daemon
    // sends `Hello` first), so usually a no-op; when non-empty these are the lowest-seq
    // frames and must be folded first to keep the seq stream gap-free. Neither an
    // `EnterSelect`, an `OpenSwapper`, nor a `NewSession` can occur this early (each needs a
    // forwarded `/select` / `/resume` / `/new` first), so the throwaway `select_requested` /
    // `open_swapper_requested` / `new_session_requested` here are never acted on.
    {
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        let mut connect_remote_requested: Option<(String, Option<String>, bool, Option<String>, Option<String>)> =
            None;
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
                &mut connect_remote_requested,
                req_tx,
            );
        }
    }

    loop {
        // Pace to ~60fps: stamp the frame start, do the work, sleep the remainder.
        let frame_start = Instant::now();

        // Latched by `apply_frame` when a `DaemonEvent::EnterSelect` arrives this drain
        // pass: the daemon asked THIS (controller) client to run the `/select` transcript
        // dump on its own terminal. Acted on AFTER the drain (we own `terminal` here).
        let mut select_requested = false;
        // Latched by `apply_frame` on a `DaemonEvent::OpenSwapper` (the `/resume` hand-off):
        // the daemon asked this client to open its local swapper. Checked AFTER the drain,
        // where we return `OpenSwapper` so `client_run` detaches + runs the swapper.
        let mut open_swapper_requested = false;
        // Latched by `apply_frame` on a `DaemonEvent::NewSession { kill }` (the `/new`
        // hand-off): the daemon asked this client to spawn + attach a brand-new
        // session-daemon. `Some(kill)` carries the `/new kill` flag. Checked AFTER the drain,
        // where we return `NewSession { kill }` so `client_run` detaches — or kills, then
        // detaches — and attaches a freshly minted daemon.
        let mut new_session_requested: Option<bool> = None;
        // Latched by `apply_frame` on a `DaemonEvent::ConnectRemote` (the `/remote`
        // hand-off): the daemon asked this client to connect to a remote host via SSH.
        // Checked AFTER the drain, where we return `ConnectRemote` so `client_run`
        // tears down the local connection and runs the remote client.
        let mut connect_remote_requested: Option<(String, Option<String>, bool, Option<String>, Option<String>)> =
            None;

        // --- (a) drain every queued incoming frame (NON-BLOCKING) ---
        // try_recv never blocks, so a quiet daemon can't stall the paint below. The
        // per-frame `dirty` bookkeeping is gone: we repaint unconditionally, so the
        // only thing that matters here is keeping the shadow current.
        loop {
            match frame_rx.try_recv() {
                Ok(frame) => {
                    apply_frame(
                        frame,
                        &mut shadow,
                        &mut expected,
                        &mut seeded,
                        &mut awaiting_resync,
                        &mut select_requested,
                        &mut open_swapper_requested,
                        &mut new_session_requested,
                        &mut connect_remote_requested,
                        req_tx,
                    );
                }
                Err(TryRecvError::Empty) => break,
                // The reader task dropped its sender: the daemon's socket closed.
                // Nothing more will ever arrive — leave the client.
                Err(TryRecvError::Disconnected) => {
                    return Ok(ClientTransition::Exit {
                        kill: pending_exit.unwrap_or(false),
                    })
                }
            }
        }

        // --- Exit-feedback timeout safety ---
        // If a quit/detach was initiated locally, check whether the daemon
        // disconnected during the drain above (already returned) or whether the
        // safety timeout has elapsed. This prevents the client from hanging
        // forever if the daemon is unresponsive.
        if pending_exit.is_some() && exit_started.elapsed() > EXIT_TIMEOUT {
            return Ok(ClientTransition::Exit {
                kill: pending_exit.unwrap_or(false),
            });
        }

        // One-shot: after the first Snapshot populates the shadow, re-apply the
        // session's mouse_capture setting so an explicit `Off` overrides the
        // startup `Auto`.
        if !mouse_capture_synced && seeded {
            mouse_capture_synced = true;
            let mc = shadow
                .rest
                .fg()
                .session
                .as_ref()
                .map(|s| s.settings.mouse_capture)
                .unwrap_or_default();
            crate::app::runtime::actions::apply_mouse_capture(mc);
        }

        // `/resume` hand-off: the daemon signalled `OpenSwapper` this drain pass. Hand
        // control back to `client_run` so it DETACHES from this daemon (leaving it
        // cooking) and runs the local swapper standalone — we must NOT keep rendering this
        // daemon's frames underneath the swapper. Returned BEFORE any further work this
        // frame; `client_run` owns the detach + swapper + reconnect.
        if open_swapper_requested {
            return Ok(ClientTransition::OpenSwapper);
        }

        // `/new` hand-off: the daemon signalled `NewSession { kill }` this drain pass. Hand
        // control back to `client_run` so it DETACHES from this daemon — or, on `kill`, sends
        // `QuitDaemon` to reap it first — and attaches a freshly minted brand-new
        // session-daemon. Like `OpenSwapper`, returned BEFORE any further work this frame; we
        // must NOT keep rendering this daemon's frames once we are tearing the connection
        // down. `client_run` owns the detach/kill + mint + attach.
        if let Some(kill) = new_session_requested {
            return Ok(ClientTransition::NewSession { kill });
        }

        // `/remote` hand-off: the daemon signalled `ConnectRemote { target }` this drain
        // pass. Hand control back to `client_run` so it tears down the local connection
        // (leaving the daemon cooking) and runs `run_remote_client_target`. Like
        // `OpenSwapper`/`NewSession`, returned BEFORE any further work this frame.
        if let Some((target, key, new_session, session_id, host_id)) = connect_remote_requested {
            return Ok(ClientTransition::ConnectRemote {
                target,
                key,
                new_session,
                session_id,
                host_id,
            });
        }

        // --- (a-bis) `/select` transcript dump (controller-side) ---
        // If the daemon signalled EnterSelect this pass, run the dump NOW — we hold the
        // `terminal`, so we can leave the alt-screen, print the shadow conversation,
        // block for a keypress, and re-enter. This is a synchronous, blocking detour
        // (exactly like the standalone loop's `/select`); the socket keeps buffering
        // frames meanwhile and the next pass drains them. A no-op if there is no shadow
        // session/conversation (the dump leaves the terminal exactly as it found it).
        if select_requested {
            client_select_dump(terminal, &shadow)?;
        }

        // --- (b) advance LOCAL animations from the monotonic clock ---
        // The comet shimmer + loading spinner derive their phase from
        // `Instant::elapsed()` read inside `view::draw`, so they advance every frame
        // for free once we repaint at 60fps below. Two things still need a nudge:
        // reconcile the comet's `work_since` on the rising/falling working edge (so it
        // starts/stops promptly between snapshots), and tick the loading splash's
        // local spinner counter (the daemon's projected `frame` is stale between
        // snapshots — drive it locally so the braille glyph cycles).
        advance_local_animations(&mut shadow);

        // Expire a locally-reconstructed toast once its TTL passes (the daemon never
        // sends a "toast cleared" delta; the client owns its own dismissal timer). The
        // toast is per-session (C6); the rendered toast is the foreground session's, so
        // sweep that one.
        let fg = shadow.rest.fg_mut();
        if let Some((_, until, _)) = fg.toast.as_ref() {
            if Instant::now() >= *until {
                fg.toast = None;
            }
        }

        // --- (c) repaint UNCONDITIONALLY ---
        // ratatui computes the cell-level diff against the previous buffer, so an
        // unchanged frame flushes ~nothing; painting every frame is what lets the
        // local animations advance smoothly without any dirty-tracking.
        //
        // Keep the quit dialog stable while waiting; only the activated chip
        // changes to its inline spinner state. If a daemon snapshot already
        // replaced the overlay, recreate it as a fallback.
        if let Some(kill) = pending_exit {
            let selected = if kill { 0 } else { 1 };
            if let Mode::QuitConfirm(s) = shadow.mode_mut() {
                s.selected = selected;
                s.phase = crate::app::mode::QuitConfirmPhase::Exiting;
            } else {
                shadow.set_mode(Mode::QuitConfirm(Box::new(QuitConfirmState::exiting(
                    selected,
                ))));
            }
        }
        terminal.draw(|f| view::draw(f, &shadow))?;

        // --- (c-ter) GUI-live palette sync: emit OSC 5380 on bg change ---
        // Runs AFTER `terminal.draw` returns (it flushes its own frame diff first),
        // so the private OSC never interleaves with ratatui's output. Gated on
        // `KOMA_GUI` (set only by `run_gui`'s pty spawn), so a normal terminal never
        // sees this escape. Diffed against `last_gui_bg` so it's only emitted when
        // `/settings` actually changes the palette, not every ~60fps frame.
        if gui_mode {
            let pal = crate::view::theme::palette(&shadow.rest.config);
            let bg = pal.bg;
            let fg = pal.fg;
            if last_gui_theme != Some((bg, fg)) {
                last_gui_theme = Some((bg, fg));
                let bg_hex = match bg {
                    ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                    _ => "#000000".to_string(),
                };
                let fg_hex = match fg {
                    ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                    _ => "#c8d3f5".to_string(),
                };
                // Private OSC 5380: tell the GUI host our canvas bg + titlebar fg so it
                // repaints the window gutter and titlebar text/buttons to match. Payload
                // is `#rrggbb,#rrggbb` (bg first, fg second). Emitted only when the
                // palette changes; gated on KOMA_GUI so normal terminals never see it.
                // ST-terminated (ESC backslash).
                let mut out = stdout();
                let _ = write!(out, "\x1b]5380;{bg_hex},{fg_hex}\x1b\\");
                let _ = out.flush();
            }
        }

        // --- (c-bis) forward the agents editor wrap width to the daemon ---
        // The shadow's agents editor publishes its wrap_w via interior mutability
        // during draw. The daemon's editor starts at usize::MAX (never rendered),
        // so we send the client-side value whenever it changes. Reset last_sent_wrap_w
        // when not in the agents editor so each fresh editor open triggers a resend
        // (the daemon's freshly-opened editor is back at usize::MAX).
        if let Mode::Agents(ref a) = shadow.mode() {
            if let Some((_, ref ed)) = a.editor {
                let w = ed.wrap_w.get();
                if last_sent_wrap_w != Some(w) {
                    last_sent_wrap_w = Some(w);
                    let _ = req_tx.send(ClientRequest::EditorWrapW(w));
                }
            } else {
                last_sent_wrap_w = None;
            }
        } else {
            last_sent_wrap_w = None;
        }

        // --- (d) poll + handle terminal input (ZERO timeout, never blocks) ---
        // Skip input handling entirely while an exit is in progress — all keys
        // are suppressed, and we're just draining frames until disconnect/timeout.
        if pending_exit.is_some() {
            // Pace to ~60fps: sleep the remainder of the budget.
            if let Some(rem) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
                std::thread::sleep(rem);
            }
            continue;
        }
        // Drain EVERY buffered event this frame so fast typing / paste never lag.
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) => {
                    // Windows crossterm reports both Press and Release KeyEventKinds
                    // (unix only ever sends Press); this is the choke point where keys
                    // enter this client process (locally handled AND forwarded to the
                    // daemon via `SendKey`, whose wire form has no `kind` field — see
                    // `ipc::proto::KeyWire` — so an unfiltered Release would replay the
                    // whole key a second time both here and daemon-side). A `kind ==
                    // Press` filter is a no-op on unix (every event already is Press).
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    // The `/quit` overlay's choices are CLIENT-process decisions, so
                    // when the shadow is in QuitConfirm (mirrored from the daemon's
                    // mode) the client intercepts its keys locally instead of
                    // forwarding them (daemon stage 12). `[k]` kills this window's
                    // daemon, `[d]` detaches it; both show an exit-feedback screen
                    // instead of returning immediately, so the user sees the braille
                    // spinner while the daemon shuts down / disconnects.
                    if matches!(shadow.mode(), Mode::QuitConfirm(_)) {
                        // The daemon owns the focus index; read the shadow's mirrored
                        // `selected` so Enter activates the focused button (and nav keys
                        // can be forwarded for the daemon to move focus).
                        let sel = if let Mode::QuitConfirm(s) = shadow.mode() {
                            s.selected
                        } else {
                            0
                        };
                        match handle_quit_confirm_key(&key, req_tx, sel) {
                            QuitConfirmKey::ExitClient { kill } => {
                                // Show exit feedback instead of returning immediately.
                                // The render loop continues draining frames until the
                                // daemon socket disconnects or a timeout fires.
                                pending_exit = Some(kill);
                                exit_started = Instant::now();
                                // Transition the shadow to the exit-feedback phase so
                                // the next draw shows the braille spinner.
                                if let Mode::QuitConfirm(s) = shadow.mode_mut() {
                                    s.phase = crate::app::mode::QuitConfirmPhase::Exiting;
                                }
                            }
                            QuitConfirmKey::Stay => {}
                        }
                        continue;
                    }
                    // Ctrl-C is fully inert now (koma disables it): it no longer
                    // detaches/exits the client. Detach is ONLY via /quit. Every key
                    // (including Ctrl-C, which the daemon-side handlers swallow) is
                    // forwarded to the daemon.
                    // Render-ahead: apply the plain composer edits to the shadow NOW
                    // (the daemon's authoritative InputChanged reconciles later), then
                    // forward the key verbatim for the daemon to interpret. Only the
                    // unambiguous text edits are echoed — see `local_echo`.
                    // Full-screen sub-agent viewer scroll is CLIENT-owned: the
                    // headless daemon never renders, so its `last_max_scroll` is
                    // always 0 and forwarding these keys would collapse the view to
                    // top/bottom. Handle them locally against THIS client's fresh
                    // max (mirrors the mouse-wheel local pattern); everything else
                    // (Esc closes the viewer daemon-side) still forwards.
                    if shadow.rest.agent_viewer.is_some() {
                        match key.code {
                            KeyCode::Up => {
                                shadow.rest.agent_viewer_scroll_up(1);
                                continue;
                            }
                            KeyCode::Down => {
                                shadow.rest.agent_viewer_scroll_down(1);
                                continue;
                            }
                            KeyCode::PageUp => {
                                shadow.rest.agent_viewer_scroll_up(10);
                                continue;
                            }
                            KeyCode::PageDown => {
                                shadow.rest.agent_viewer_scroll_down(10);
                                continue;
                            }
                            KeyCode::Home => {
                                shadow.rest.agent_viewer_scroll_to_top();
                                continue;
                            }
                            KeyCode::End => {
                                shadow.rest.agent_viewer_scroll_to_bottom();
                                continue;
                            }
                            _ => {}
                        }
                    }
                    local_echo(&mut shadow, &key);
                    let _ = req_tx.send(ClientRequest::SendKey(KeyWire::from(key)));
                }
                // Mouse wheel scrolls the LOCAL shadow transcript (a pure view
                // concern — the daemon's scroll is its own; scrolling the shadow
                // gives immediate feedback without a round-trip). Bottom-pinning
                // follow is reconstructed from snapshots, so a manual scroll just
                // nudges the local offset for this render.
                Event::Mouse(m)
                    if matches!(shadow.mode(), Mode::Chat | Mode::Bash(_) | Mode::Todo(_)) =>
                {
                    // When the full-screen sub-agent viewer is open, the wheel
                    // scrolls IT (client-owned); otherwise it scrolls the main
                    // transcript. Both use the client's fresh `last_max_scroll`.
                    let viewer = shadow.rest.agent_viewer.is_some();
                    match m.kind {
                        MouseEventKind::ScrollUp => {
                            for _ in 0..3 {
                                if viewer {
                                    shadow.rest.agent_viewer_scroll_up(1);
                                } else {
                                    shadow.rest.scroll_up();
                                }
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            for _ in 0..3 {
                                if viewer {
                                    shadow.rest.agent_viewer_scroll_down(1);
                                } else {
                                    shadow.rest.scroll_down();
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // A resize just needs the next unconditional paint to relayout.
                Event::Resize(_, _) => {}
                // Bracketed paste: forward the WHOLE text as one Paste request so the
                // daemon runs the SAME `handle_paste` the local TUI uses. This is what
                // makes path-image paste work remotely — a pasted image-file path is
                // detected daemon-side and ingested into the session's `images/` dir as
                // an `[Image #N]` attachment, and multi-line text keeps its newlines
                // (CRLF-normalised). Forwarding char-by-char (the old behaviour) ran the
                // daemon's plain `Char` handler instead, which can't detect an image
                // path and mangles line endings. NOT echoed locally: a paste may become
                // a marker rather than literal text, so faking the raw text would
                // flicker — the daemon's InputChanged/Snapshot reconciles within a frame.
                Event::Paste(text) => {
                    let _ = req_tx.send(ClientRequest::Paste { text });
                }
                _ => {}
            }
        }

        // --- frame pacing: sleep the remainder of the ~16ms budget ---
        // Keeps the loop at ~60fps instead of busy-spinning. If a frame overran the
        // budget (a big snapshot rebuild) we skip the sleep and proceed immediately.
        if let Some(rem) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }
}

/// Advance the LOCAL-clock animations on the shadow once per frame.
///
/// The client owns NO daemon ticks, so animations that the local TUI advances from
/// its event loop must be advanced here from the client's own monotonic clock:
///
/// - **Comet shimmer (`work_since`).** Reconcile it on the rising/falling working
///   edge exactly like the local loop's `service_global`: stamp `now` when the
///   foreground session starts working (and isn't paused on a y/n approval) and it
///   isn't already running; clear it when work ends or an approval takes over. The
///   travelling head + elapsed counter then derive from `work_since.elapsed()` at
///   draw time. (A snapshot may also set `work_since` from the daemon's anchored
///   `work_elapsed_ms`; this only fills the rising/falling edges BETWEEN snapshots so
///   the comet never freezes or lingers.)
/// - **Loading splash spinner (`frame`).** The braille glyph is indexed by
///   `frame % 10`; the daemon's projected `frame` is frozen between snapshots, so
///   tick it locally each frame to keep the spinner rotating (the footer's elapsed
///   counter already derives from `started.elapsed()`).
pub(super) fn advance_local_animations(shadow: &mut AppState) {
    // Comet: rising/falling-edge reconcile (mirrors `service_global`).
    reconcile_work_clock(shadow);

    // Loading splash: keep the local spinner counter rotating between snapshots.
    if let Mode::Loading(s) = shadow.mode_mut() {
        s.frame = s.frame.wrapping_add(1);
    }
}

/// Run the `/select` transcript dump on the CLIENT's terminal (the controller-side
/// half of the `/select` hand-off — see [`crate::ipc::proto::DaemonEvent::EnterSelect`]).
///
/// The daemon owns no TTY, so when a forwarded `/select` set its `select_pending` flag
/// it signalled THIS client to perform the dump. This mirrors the standalone loop's
/// [`super::super::event_loop::drains`] `enter_select`/`exit_select`, but sourced from
/// the SHADOW conversation and self-contained (it blocks for the return keypress here
/// rather than threading a `select_active` state through the render loop):
///   1. leave the alt-screen,
///   2. print the foreground shadow session's conversation as plain text (so the user
///      can select/copy with the terminal's native selection) — raw mode stays on, so
///      lines are terminated with `\r\n`,
///   3. block until the user presses any key,
///   4. re-enter the alt-screen and force a full repaint
///      (`terminal.clear()`), so the next loop pass redraws the live shadow cleanly.
///
/// Robustness: if the shadow has no foreground session/conversation there is nothing to
/// dump, so it returns immediately WITHOUT touching the terminal — the alt-screen is
/// never left, so the terminal can't be stranded in a half-restored state.
pub(super) fn client_select_dump(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    shadow: &AppState,
) -> Result<()> {
    // No shadow session → nothing to dump. Return before any terminal mutation so we
    // never leave the alt-screen with nothing to show (clean no-op).
    if shadow.rest.fg().session.is_none() {
        return Ok(());
    }

    // (1) Drop to the normal screen so the printed transcript uses the scrollback the
    // user can select from. Disable mouse capture first so native selection works.
    use ratatui::crossterm::event::DisableMouseCapture;
    let _ = execute!(stdout(), DisableMouseCapture);
    execute!(stdout(), LeaveAlternateScreen)?;

    // (2) Print the conversation as plain text (raw mode is on → `\r\n`). Mirrors
    // `drains::enter_select`'s formatting exactly: skip System/Tool, label you/ai.
    let mut out = stdout();
    if let Some(sess) = shadow.rest.fg().session.as_ref() {
        for m in sess.conversation.messages() {
            let label = match m.role {
                Role::System | Role::Tool => continue,
                Role::User => "you",
                Role::Assistant => "ai",
            };
            write!(out, "\r\n{label}:\r\n")?;
            for line in m.content.split('\n') {
                write!(out, "{line}\r\n")?;
            }
        }
    }
    write!(
        out,
        "\r\n-- copy with your mouse, then press any key to return --\r\n"
    )?;
    out.flush()?;

    // (3) Block until a key is pressed. Read events (blocking) and ignore non-key ones
    // (a stray resize/mouse must NOT count as the "any key" return).
    loop {
        if let Event::Key(_) = event::read()? {
            break;
        }
    }

    // (4) Restore the alt-screen + mouse and force a full repaint next draw.
    execute!(stdout(), EnterAlternateScreen)?;
    // Re-apply the shadow session's mouse capture setting (not unconditional enable).
    let mc = shadow
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.settings.mouse_capture)
        .unwrap_or_default();
    crate::app::runtime::actions::apply_mouse_capture(mc);
    terminal.clear()?;
    Ok(())
}

/// Serialise `env` and hand it to `push`, dropping it silently on the (never-
/// expected) serialisation error rather than panicking mid-frame.
pub(super) fn emit(push: &dyn Fn(String), env: &PushEnvelope) {
    if let Ok(json) = serde_json::to_string(env) {
        push(json);
    }
}

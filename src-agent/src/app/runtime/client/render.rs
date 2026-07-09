use std::io::{stdout, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{Mode, QuitConfirmState, SessionHub, SessionKind};
use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, KeyWire};
use crate::view;

use super::input::{handle_quit_confirm_key, local_echo, send_overlay_cancel, QuitConfirmKey};
use super::shadow::{apply_frame, reconcile_work_clock};
use super::push_proto::{
    push_file_diff, push_switching, push_usage_preview, PushAttachment, PushBashJob,
    PushCooking, PushEnvelope, PushFileChange, PushHistory, PushMcpServer, PushModel, PushMsg,
    PushPalette, PushPaletteInfo, PushPendingCall, PushPlanTodo, PushProvider, PushRoute,
    PushSubAgent, PushToolCall,
};
use super::project::{push_config, push_hub, serialize_and_push, ConfigProjection};

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
pub(super) enum ClientTransition {
    /// Tear the client down and return from `client_run` (detach / ExitClient /
    /// frame channel disconnected).
    Exit,
    /// Detach from the current daemon and open the local daemon swapper (`/resume`).
    OpenSwapper,
    /// Detach (or kill, on `kill`) the current daemon and attach a brand-new
    /// session-daemon (`/new` / `/new kill`). The bool is the `/new kill` flag.
    NewSession { kill: bool },
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
                        req_tx,
                    );
                }
                Err(TryRecvError::Empty) => break,
                // The reader task dropped its sender: the daemon's socket closed.
                // Nothing more will ever arrive — leave the client.
                Err(TryRecvError::Disconnected) => return Ok(ClientTransition::Exit),
            }
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
        // Drain EVERY buffered event this frame so fast typing / paste never lag.
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) => {
                    // The `/quit` overlay's choices are CLIENT-process decisions, so
                    // when the shadow is in QuitConfirm (mirrored from the daemon's
                    // mode) the client intercepts its keys locally instead of
                    // forwarding them (daemon stage 12). `[k]` kills this window's
                    // daemon, `[d]` detaches it; both ask the loop to exit the client.
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
                            QuitConfirmKey::ExitClient => return Ok(ClientTransition::Exit),
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
                            KeyCode::Up       => { shadow.rest.agent_viewer_scroll_up(1);   continue; }
                            KeyCode::Down     => { shadow.rest.agent_viewer_scroll_down(1);  continue; }
                            KeyCode::PageUp   => { shadow.rest.agent_viewer_scroll_up(10);   continue; }
                            KeyCode::PageDown => { shadow.rest.agent_viewer_scroll_down(10); continue; }
                            KeyCode::Home     => { shadow.rest.agent_viewer_scroll_to_top();    continue; }
                            KeyCode::End      => { shadow.rest.agent_viewer_scroll_to_bottom(); continue; }
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
                Event::Mouse(m) if matches!(shadow.mode(), Mode::Chat | Mode::Bash(_) | Mode::Todo(_)) => {
                    // When the full-screen sub-agent viewer is open, the wheel
                    // scrolls IT (client-owned); otherwise it scrolls the main
                    // transcript. Both use the client's fresh `last_max_scroll`.
                    let viewer = shadow.rest.agent_viewer.is_some();
                    match m.kind {
                        MouseEventKind::ScrollUp => {
                            for _ in 0..3 {
                                if viewer { shadow.rest.agent_viewer_scroll_up(1); }
                                else { shadow.rest.scroll_up(); }
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            for _ in 0..3 {
                                if viewer { shadow.rest.agent_viewer_scroll_down(1); }
                                else { shadow.rest.scroll_down(); }
                            }
                        }
                        _ => {}
                    }
                },
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
///   1. leave the alt-screen + disable mouse capture,
///   2. print the foreground shadow session's conversation as plain text (so the user
///      can select/copy with the terminal's native selection) — raw mode stays on, so
///      lines are terminated with `\r\n`,
///   3. block until the user presses any key,
///   4. re-enter the alt-screen + mouse capture and force a full repaint
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
    // user can select from.
    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

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
    write!(out, "\r\n-- copy with your mouse, then press any key to return --\r\n")?;
    out.flush()?;

    // (3) Block until a key is pressed. Read events (blocking) and ignore non-key ones
    // (a stray resize/mouse must NOT count as the "any key" return).
    loop {
        if let Event::Key(_) = event::read()? {
            break;
        }
    }

    // (4) Restore the alt-screen + mouse and force a full repaint next draw.
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    Ok(())
}


/// Per-connection dedup memory for the push pipeline: the last values pushed, so
/// [`serialize_and_push`] / [`push_hub`] only emit an envelope when something
/// actually changed (the fold loop calls them every ~16ms).
pub(super) struct PushState {
    /// Fingerprint of the last `Snapshot` (session + messages + title + palette).
    pub(super) snapshot_fp: Option<u64>,
    /// Last streaming buffer pushed (`None` once cleared).
    pub(super) stream: Option<String>,
    /// Last reasoning buffer pushed (empty once cleared).
    pub(super) reasoning: String,
    /// Last `(working, toast, toast_kind, tokens_in, tokens_cached, tokens_out, cost,
    /// mode)` pushed — the full `Status` envelope payload, so a counter tick, a
    /// mode flip, or a working/toast change each independently re-emit `Status`.
    /// `cost` is `f64`; plain `!=` (`PartialEq`, not `Eq`) is fine here — this tuple
    /// is only ever compared, never hashed or used as a map key.
    pub(super) status: Option<(
        bool,
        Option<String>,
        Option<&'static str>,
        u64,
        u64,
        u64,
        f64,
        String,
    )>,
    /// Last serialised `Hub` JSON (the swapper is diffed as a whole).
    pub(super) hub_json: Option<String>,
    /// Last serialised `Config` JSON (the global config catalogue, diffed as a whole so
    /// an unchanged config emits nothing). Cleared by [`reset`](Self::reset) on `Ready`
    /// so a page reload re-emits the full current catalogue.
    pub(super) config_json: Option<String>,
}

impl PushState {
    pub(super) fn new() -> Self {
        Self {
            snapshot_fp: None,
            stream: None,
            reasoning: String::new(),
            status: None,
            hub_json: None,
            config_json: None,
        }
    }

    /// Forget every last-pushed value so the next [`serialize_and_push`] re-emits the
    /// FULL current state. Called on a `Ready` (the page (re)booted and needs a fresh
    /// authoritative snapshot), so a webview reload never renders against stale deltas.
    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }
}

/// What [`push_loop`] resolved to — the instruction the host-relay state machine in
/// [`super::run_host_relay`] acts on next. Mirrors [`ClientTransition`] for the
/// headless GUI host (no terminal): leave, fall back to the swapper, or attach a
/// different session.
pub(super) enum HostTransition {
    /// Leave the host entirely (the control channel closed — the window is gone).
    Exit,
    /// Detach and show the local session swapper (the daemon's socket closed, or it
    /// signalled `OpenSwapper`). `run_host_relay` rebuilds the hub from discovery.
    ToSwapper,
    /// Attach to this session UUID (a hub `SelectSession`/`NewSession`, or a daemon
    /// `NewSession` hand-off). A minted uuid for a new session; an existing id otherwise.
    /// `workdir` is the folder a GUI `[+ new session]` native picker chose (the new
    /// session's working dir); `None` for every other attach inherits the host's cwd.
    Attach {
        id: String,
        workdir: Option<std::path::PathBuf>,
    },
}

/// The HEADLESS twin of [`render_loop`]: fold the daemon's frames into the shadow and
/// PUSH the resulting state to the webview instead of drawing it to a terminal. Same
/// 16ms cadence, same non-blocking frame drain + local-animation advance + toast
/// sweep, but the crossterm input poll is gone (input arrives as `HostCtl` from the
/// ipc thread and `SubmitInput` goes straight to the daemon over `req_tx`).
///
/// Each frame, in order: (0) drain `ctl_rx` — `Ready` forces a full re-push, a
/// `Select`/`New` returns an [`HostTransition::Attach`]; (a) drain every queued
/// [`DaemonFrame`] and apply it (an `OpenSwapper`/`NewSession` hand-off returns the
/// matching transition, a closed socket returns [`HostTransition::ToSwapper`]); (b)
/// advance the local-clock animations + sweep the toast; (c) serialise the shadow and
/// push whatever changed; then pace to the frame budget. Returns when a transition is
/// resolved.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(super) fn push_loop(
    push: &dyn Fn(String),
    frame_rx: &Receiver<DaemonFrame>,
    req_tx: &Sender<ClientRequest>,
    prebuffered: Vec<DaemonFrame>,
    ctl_tx: &Sender<super::HostCtl>,
    ctl_rx: &Receiver<super::HostCtl>,
    last: &mut PushState,
    current_session: Option<&str>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: &std::sync::Arc<std::sync::Mutex<super::StreamView>>,
) -> HostTransition {
    use std::sync::mpsc::TryRecvError;

    // The shadow is a real AppState reconstructed purely from frames (identical to
    // `render_loop`); the first Snapshot replaces the neutral placeholder.
    let mut shadow = AppState::new(Mode::Chat);
    shadow.rest.fg_mut().status = "attaching…".into();

    let mut expected: u64 = 0;
    let mut seeded = false;
    let mut awaiting_resync = false;

    // Latest authoritative config catalogue, cached off each incoming full snapshot so
    // `push_config` can (re)emit the `Config` envelope every frame (dedup'd) — including
    // after a `Ready` reset, without waiting for the daemon to resend a snapshot.
    let mut current_config: Option<ConfigProjection> = None;

    // --- attached-state hub refresh (RefreshHub) ---
    // Cross-daemon discovery (`build_local_hub` → `list_live_sessions`) BLOCKS on a
    // per-socket Status probe, so it must NOT run inline on this 16ms fold loop (it
    // would stall frame folding + animation for the whole multi-socket sweep). Instead
    // a `RefreshHub` spawns a ONE-SHOT worker thread that runs the blocking sweep off
    // this thread and ships the built `SessionHub` back over `hub_rx`; the loop drains
    // it non-blocking and calls `push_hub` (which diffs `last.hub_json`, so a no-change
    // refresh is silent and repeated palette-opens are cheap). `refresh_inflight`
    // coalesces bursts — React may re-emit RefreshHub on an interval while the palette
    // stays open — so at most one sweep runs at a time. `current_owned` flags the
    // attached row as `is_foreground` in the rebuilt hub.
    let (hub_tx, hub_rx) = std::sync::mpsc::channel::<SessionHub>();
    let mut refresh_inflight = false;
    let current_owned: Option<String> = current_session.map(str::to_string);

    // --- FILE CHANGED diff fetch (FileDiff) ---
    // `compute_file_diff` shells out to git + reads the file, both blocking, so — same
    // reasoning as `RefreshHub` above — it runs on a one-shot worker thread rather than
    // inline on this 16ms fold loop; the loop drains completed results non-blocking and
    // pushes each as a `FileDiff` envelope. Unlike the hub refresh there is no "latest
    // wins" coalescing: each request is for a (possibly different) path, so every
    // completed result is pushed, not just the newest.
    let (file_diff_tx, file_diff_rx) = std::sync::mpsc::channel::<super::diff::FileDiffResult>();

    // --- USAGE PANEL preview fetch (UsagePreview) ---
    // `compute_usage_preview` hits sqlite, blocking, so — same reasoning as `FileDiff`
    // above — it runs on a one-shot worker thread; the loop drains completed results
    // non-blocking and pushes each as a `UsagePreview` envelope. The `String` riding
    // alongside is the request's `scope` ("all"/"session"); the `Option<String>` is the
    // `session` uuid that was ACTUALLY queried (only `Some` for a real "session" scope).
    // Both are echoed back unchanged so React can drop a reply whose scope OR session id
    // no longer matches what's currently selected/attached — a rapid toggle, OR a
    // foreground session switch, racing an in-flight request must never render the
    // wrong session's numbers.
    let (usage_preview_tx, usage_preview_rx) =
        std::sync::mpsc::channel::<(super::diff::UsagePreviewResult, String, Option<String>)>();

    // Fold the handshake's prebuffered frames first, through the SAME `apply_frame`
    // path (seq seeding stays gap-free). The select/swapper/new latches can't fire
    // this early, so the throwaways here are never acted on.
    {
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        for frame in prebuffered {
            // Cache the config off any prebuffered full snapshot (normally none — Hello
            // is first, so the attach Snapshot lands in the live drain — but stay safe).
            if let DaemonEvent::Snapshot(snap) = &frame.event {
                current_config = Some(ConfigProjection::from_global(&snap.global));
            }
            apply_frame(
                frame,
                &mut shadow,
                &mut expected,
                &mut seeded,
                &mut awaiting_resync,
                &mut select_requested,
                &mut open_swapper_requested,
                &mut new_session_requested,
                req_tx,
            );
        }
    }

    loop {
        let frame_start = Instant::now();

        // --- (0) control messages from the ipc thread (NON-BLOCKING) ---
        loop {
            match ctl_rx.try_recv() {
                // The page (re)booted: re-push the full authoritative state this frame.
                Ok(super::HostCtl::Ready) => last.reset(),
                // A hub pick / new-session request: signal swap-START (so React raises the
                // loader BEFORE this attached push_loop returns + the connection is torn
                // down — the ONLY seam still holding a live socket), then hand back to the
                // state machine to detach + attach the chosen (or freshly minted) session.
                Ok(super::HostCtl::Select(id)) => {
                    push_switching(push, &id);
                    return HostTransition::Attach { id, workdir: None };
                }
                // `[+ new session]` while attached: the GUI picker already confirmed a
                // folder (a cancel sends no `New`), so carry it into the fresh session. On
                // `kill` reap the CURRENT daemon as part of the switch — queue a graceful
                // QuitDaemon on the live conn (flushed by the upcoming teardown, mirroring the
                // TUI `/new kill`) and ensure its death OFF-thread so the fresh attach never
                // waits on the old daemon's corpse. `kill: false` leaves the old daemon
                // cooking (resumable), exactly as before.
                Ok(super::HostCtl::New { workdir, kill }) => {
                    if kill {
                        if let Some(old) = current_owned.clone() {
                            let _ = req_tx.send(ClientRequest::QuitDaemon);
                            super::host::spawn_ensure_dead(old);
                        }
                    }
                    let new_id = uuid::Uuid::new_v4().to_string();
                    push_switching(push, &new_id);
                    return HostTransition::Attach { id: new_id, workdir };
                }
                // KILL the daemon `id`. Killing the CURRENTLY-ATTACHED session: queue a
                // graceful QuitDaemon on the live conn (flushed by teardown), ensure its death
                // OFF-thread — a harmless double-QuitDaemon that ALSO fires a follow-up
                // RefreshHub so the swapper we're about to land in drops the row the instant
                // it is gone (its entry push may briefly show it for <1s) — then hand back to
                // the swapper (the same path `ToSwapper` takes). A BACKGROUND kill just
                // escalates OFF-thread and refreshes the hub once the daemon is confirmed dead
                // (the off-thread sweep drained at (b-bis) pushes the rebuilt hub).
                Ok(super::HostCtl::KillSession(id)) => {
                    if current_owned.as_deref() == Some(id.as_str()) {
                        let _ = req_tx.send(ClientRequest::QuitDaemon);
                        super::host::spawn_kill_and_refresh(ctl_tx.clone(), id);
                        return HostTransition::ToSwapper;
                    }
                    super::host::spawn_kill_and_refresh(ctl_tx.clone(), id);
                }
                // Physically DELETE a history session OFF-thread (guarded host-side against
                // deleting a live/locked session), then RefreshHub. A history row is never the
                // attached session, so there is no live-conn interaction here.
                Ok(super::HostCtl::DeleteSession(id)) => {
                    super::host::spawn_delete_and_refresh(ctl_tx.clone(), id);
                }
                // Cancel-switch (best-effort): the swap in flight can't be interrupted, so
                // this simply drops to the hub AFTER the current/queued attach resolves —
                // `host_swapper` then pushes a fresh `Hub`, and the loader clears on it.
                Ok(super::HostCtl::ToSwapper) => return HostTransition::ToSwapper,
                // The ResumePalette opened: kick a hub refresh OFF this thread (the
                // discovery sweep blocks). Coalesced by `refresh_inflight` so a burst of
                // RefreshHubs while the palette stays open runs at most one sweep; the
                // result is drained + pushed below.
                Ok(super::HostCtl::RefreshHub) => {
                    if !refresh_inflight {
                        refresh_inflight = true;
                        let tx = hub_tx.clone();
                        let cur = current_owned.clone();
                        std::thread::spawn(move || {
                            let hub = super::build_local_hub(cur.as_deref());
                            let _ = tx.send(hub);
                        });
                    }
                }
                // A config mutation raced in while attached (the ipc handler normally
                // routes these straight to the daemon via `live_req` when a session is
                // attached; this only lands here if the attach state flipped between the
                // check and the send). Forward the carried request to the daemon — it owns
                // the authoritative config and re-pushes a fresh `Config` on the change.
                Ok(super::HostCtl::ConfigMutate(req)) => {
                    let _ = req_tx.send(req);
                }
                // A live model / route fetch raced in while attached (the ipc handler routes
                // these straight to the daemon via `live_req` when attached; they only land
                // here if the attach state flipped between the detached-check and the send).
                // Forward the equivalent daemon request — the daemon fetches + replies
                // out-of-band and the `ModelList`/`ModelRoutes` frame is re-pushed above — so
                // the reply is never dropped on the race.
                Ok(super::HostCtl::ListModels { provider }) => {
                    let _ = req_tx.send(ClientRequest::ListModels { provider });
                }
                Ok(super::HostCtl::ListRoutes { provider, model_id }) => {
                    let _ = req_tx.send(ClientRequest::ListRoutes { provider, model_id });
                }
                // GUI Settings fetch raced in while attached (the ipc handler routes it to
                // the daemon via `live_req` when attached; it only lands here if the attach
                // state flipped between the detached-check and the send). Forward the daemon
                // request — the daemon replies with `SettingsValues`, re-pushed above.
                Ok(super::HostCtl::GetSettings) => {
                    let _ = req_tx.send(ClientRequest::GetSettings);
                }
                // FILE CHANGED diff fetch: NEVER touches the daemon (host-side only,
                // regardless of attach state) — spawn the blocking git+fs work off this
                // thread; the result is drained + pushed below at (b-quat).
                Ok(super::HostCtl::FileDiff { path }) => {
                    let tx = file_diff_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let result = super::diff::compute_file_diff(&path, cur.as_deref());
                        let _ = tx.send(result);
                    });
                }
                // USAGE PANEL preview fetch: NEVER touches the daemon (host-side ledger
                // read only, regardless of attach state) — spawn the blocking sqlite work
                // off this thread; the result is drained + pushed below at (b-quin).
                // `scope` AND `session` both ride along so the reply can echo them.
                Ok(super::HostCtl::UsagePreview { session, scope }) => {
                    let tx = usage_preview_tx.clone();
                    std::thread::spawn(move || {
                        let result = super::diff::compute_usage_preview(session.as_deref());
                        let _ = tx.send((result, scope, session));
                    });
                }
                Err(TryRecvError::Empty) => break,
                // The ipc side hung up (window gone) — leave the host.
                Err(TryRecvError::Disconnected) => return HostTransition::Exit,
            }
        }

        // --- (a) drain every queued incoming frame (NON-BLOCKING) ---
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        loop {
            match frame_rx.try_recv() {
                Ok(frame) => {
                    // Omnisearch reply: intercept the one-shot `FileSearchResults` and
                    // re-push it to JS as a `SearchResults` envelope BEFORE folding (the
                    // fold treats it as a non-visual no-op, keeping the seq gap-free).
                    if let DaemonEvent::FileSearchResults { query, items } = &frame.event {
                        let env = PushEnvelope::SearchResults {
                            query: query.clone(),
                            items: items.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Cache the authoritative config off every full snapshot so the
                    // `Config` envelope can be (re)emitted below (a config edit forces a
                    // full snapshot — see `ipc::snapshot::diff`).
                    if let DaemonEvent::Snapshot(snap) = &frame.event {
                        current_config = Some(ConfigProjection::from_global(&snap.global));
                    }
                    // Live model-id catalogue reply (Connector model picker): re-push it as
                    // a `ModelList` envelope BEFORE folding (the fold treats it as a
                    // non-visual no-op, keeping the seq gap-free).
                    if let DaemonEvent::ModelList { provider, models } = &frame.event {
                        let env = PushEnvelope::ModelList {
                            provider: provider.clone(),
                            models: models.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Live provider-route reply (Connector ModelForm route picker): re-push
                    // it as a `RouteList` envelope BEFORE folding (a non-visual fold no-op),
                    // flattening each wire route to the camelCase `PushRoute` JS contract.
                    if let DaemonEvent::ModelRoutes { provider, model_id, routes } = &frame.event {
                        let env = PushEnvelope::RouteList {
                            provider: provider.clone(),
                            model_id: model_id.clone(),
                            routes: routes
                                .iter()
                                .map(|r| PushRoute {
                                    name: r.name.clone(),
                                    provider_name: r.provider_name.clone(),
                                    price_prompt: r.price_prompt.clone(),
                                    price_completion: r.price_completion.clone(),
                                    uptime_last_30m: r.uptime_last_30m,
                                })
                                .collect(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // GUI Settings-tab reply (GetSettings / post-SetSessionPrefs re-push):
                    // re-push it as a `SettingsValues` envelope BEFORE folding (a non-visual
                    // fold no-op, keeping the seq gap-free), same as the ModelList/RouteList
                    // intercepts above.
                    if let DaemonEvent::SettingsValues {
                        name,
                        workdir,
                        short_send,
                        sliding_cache,
                        bash_saving,
                        internet_mode,
                        palette,
                        effort,
                    } = &frame.event
                    {
                        let env = PushEnvelope::SettingsValues {
                            name: name.clone(),
                            workdir: workdir.clone(),
                            short_send: *short_send,
                            sliding_cache: *sliding_cache,
                            bash_saving: *bash_saving,
                            internet_mode: internet_mode.clone(),
                            palette: palette.clone(),
                            effort: effort.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Composer EFFORT-picker reply (GetEffortOptions): re-push it as an
                    // `EffortOptions` envelope BEFORE folding (a non-visual fold no-op,
                    // keeping the seq gap-free), same as the SettingsValues intercept above.
                    if let DaemonEvent::EffortOptions {
                        options,
                        selected,
                        note,
                        state,
                    } = &frame.event
                    {
                        let env = PushEnvelope::EffortOptions {
                            options: options.clone(),
                            selected: *selected,
                            note: note.clone(),
                            state: state.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    apply_frame(
                        frame,
                        &mut shadow,
                        &mut expected,
                        &mut seeded,
                        &mut awaiting_resync,
                        &mut select_requested,
                        &mut open_swapper_requested,
                        &mut new_session_requested,
                        req_tx,
                    );
                }
                Err(TryRecvError::Empty) => break,
                // The reader task dropped its sender: the daemon's socket closed. Fall
                // back to the swapper so the user can pick another session.
                Err(TryRecvError::Disconnected) => return HostTransition::ToSwapper,
            }
        }

        // `/resume` hand-off from the daemon: detach + show the swapper.
        if open_swapper_requested {
            return HostTransition::ToSwapper;
        }
        // `/new` hand-off from the daemon: attach a freshly minted session. (The `kill`
        // flag is a daemon-side reap the headless host does not drive in W0; a plain
        // detach-then-attach is fine — the old daemon keeps cooking, resumable.)
        if new_session_requested.is_some() {
            let new_id = uuid::Uuid::new_v4().to_string();
            // Same swap-START loader signal as a hub `New` — this is a daemon-driven attach
            // gap, equally frozen until the new session's first Snapshot.
            push_switching(push, &new_id);
            // Daemon-driven hand-off carries no picked folder — inherit the host cwd.
            return HostTransition::Attach { id: new_id, workdir: None };
        }
        // `/select` transcript dump needs a terminal the host does not own — ignore it.

        // --- (b) advance LOCAL-clock animations + sweep the toast ---
        advance_local_animations(&mut shadow);
        {
            let fg = shadow.rest.fg_mut();
            if let Some((_, until, _)) = fg.toast.as_ref() {
                if Instant::now() >= *until {
                    fg.toast = None;
                }
            }
        }

        // --- (b-bis) attached-state hub refresh: push any completed off-thread sweep ---
        // Drain the worker channel to the NEWEST built hub (non-blocking), clear the
        // in-flight latch, and push it. `push_hub` diffs `last.hub_json`, so an
        // unchanged live set emits nothing. This is what keeps the React ResumePalette's
        // cooking/history current while ATTACHED, not frozen at the cold boot build.
        {
            let mut latest_hub: Option<SessionHub> = None;
            while let Ok(hub) = hub_rx.try_recv() {
                latest_hub = Some(hub);
                refresh_inflight = false;
            }
            if let Some(hub) = latest_hub {
                push_hub(&hub, push, last);
            }
        }

        // --- (b-quat) FILE CHANGED diff fetch: push any completed off-thread diffs ---
        // Drain ALL completed results (not just the newest — see the channel's doc
        // comment above) and push each as its own one-shot `FileDiff` envelope.
        while let Ok(result) = file_diff_rx.try_recv() {
            push_file_diff(push, result);
        }

        // --- (b-quin) USAGE PANEL: push any completed off-thread preview fetch ---
        while let Ok((result, scope, session_id)) = usage_preview_rx.try_recv() {
            push_usage_preview(push, result, scope, session_id);
        }

        // --- (b-ter) mirror the staged-attachment markers for the ipc Submit append ---
        // The ipc thread appends these `[Image #N]` markers to a chat send so the daemon's
        // submit-time reconcile keeps the staged images (React's text carries no markers).
        if let Ok(mut marks) = live_marks.lock() {
            marks.clear();
            marks.extend(shadow.rest.fg().pending_attachments.iter().map(|a| a.marker_n));
        }

        // --- (c) serialise + push whatever changed (the draw seam) ---
        // Snapshot the current stream view (Copy) out of the shared lock so the fold folds
        // the viewed sub-agent's transcript / viewed bash job's output tail into the push.
        let view = live_view.lock().map(|v| *v).unwrap_or_default();
        serialize_and_push(&shadow, push, last, view);
        // Config catalogue (Connector + MCP panels): emit whenever it changed since the
        // last frame, or re-emit after a `Ready` reset. Independent of the per-session
        // draw so a page reload always re-pushes the current global config.
        push_config(current_config.as_ref(), push, last);

        // --- frame pacing: sleep the remainder of the ~16ms budget ---
        if let Some(rem) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }
}


/// Serialise `env` and hand it to `push`, dropping it silently on the (never-
/// expected) serialisation error rather than panicking mid-frame.
pub(super) fn emit(push: &dyn Fn(String), env: &PushEnvelope) {
    if let Ok(json) = serde_json::to_string(env) {
        push(json);
    }
}

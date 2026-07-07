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

// ─── host-relay push envelopes (native-React GUI client) ─────────────────────────
//
// The GUI host is itself the daemon client (see `crate::app::runtime::gui`): instead
// of drawing the shadow `AppState` to a terminal, it SERIALISES it into the JSON
// envelopes the React client consumes and pushes them through
// `window.__komaClient.push(...)`. These structs are the Rust half of the bridge
// contract — `#[serde(tag = "k")]` names each envelope, matching the JS `push`
// dispatcher's `k` switch EXACTLY. The host always pushes AUTHORITATIVE full values
// (React REPLACES on `StreamMsg` / `Reasoning`, never appends); [`PushState`] dedups
// so an unchanged frame emits nothing.

/// One committed conversation turn in a [`PushEnvelope::Snapshot`].
#[derive(serde::Serialize)]
struct PushMsg {
    role: &'static str,
    content: String,
    reasoning: Option<String>,
}

/// One STAGED (not-yet-sent) composer attachment chip in a [`PushEnvelope::Snapshot`].
/// Mirrors the daemon's `pending_attachments`: `marker_n` (serialised `markerN`) ties
/// the chip to its `[Image #N]` marker so React can round-trip it back in a
/// `RemoveAttachment`; `name` is the on-disk basename; `kind` is `"image"`/`"file"`
/// derived from the sniffed mime. Authoritative full array — React REPLACES on each
/// Snapshot (a stage/drop re-emits the Snapshot via the folded fingerprint).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushAttachment {
    marker_n: usize,
    name: String,
    kind: &'static str,
}

/// One sub-agent row in a [`PushEnvelope::Snapshot`] (list + status only — the live
/// transcript/report is NOT shipped this wave). `name` is the agent definition name,
/// `summary` is the compact one-line label (the truncated task), and `status` is the
/// canonical lifecycle string `running`/`done`/`killed`/`error`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushSubAgent {
    name: String,
    status: &'static str,
    summary: String,
}

/// One background-bash job row in a [`PushEnvelope::Snapshot`] (list + status only).
/// `id` is the model-facing job id (`bash-<n>`), `cmd` is the shell command, and
/// `status` is the canonical lifecycle string `running`/`done`/`killed`/`error`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushBashJob {
    id: String,
    cmd: String,
    status: &'static str,
}

/// The canvas bg/fg the React client paints its chrome with (resolved from the
/// shadow's palette, so a themed daemon repaints the window live).
#[derive(serde::Serialize, PartialEq, Clone)]
struct PushPalette {
    bg: String,
    fg: String,
}

/// A COOKING-pane row in a [`PushEnvelope::Hub`]. The synthetic `[+ new session]`
/// row carries only `kind`/`id`/`name`; a real session row fills the rest (the
/// session-only fields are `Option` + skip-if-none so the two shapes match the
/// contract's per-row shape).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushCooking {
    kind: &'static str,
    id: Option<String>,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    working: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dir_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_dir: Option<bool>,
}

/// A HISTORY-pane row in a [`PushEnvelope::Hub`] (an on-disk session not currently
/// live). `id` is the session UUID (the on-disk dir name); `last_active` is unix ms.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushHistory {
    id: String,
    name: String,
    last_active: u64,
    dir_label: String,
    current_dir: bool,
}

/// One provider row in a [`PushEnvelope::Config`] (the Connector panel's ProviderForm
/// model). `id` is the config uuid (stable identity a `SetProvider`/`DeleteProvider`
/// round-trips). The plaintext `api_key` is NEVER sent to the webview (devtools are
/// enabled, and the key would sit readable in the DOM/console) — only `has_key`, a
/// presence flag the form uses to render a "leave blank to keep" placeholder. Saving
/// with a blank key preserves the existing stored key (see `upsert_provider`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushProvider {
    id: String,
    name: String,
    endpoint: String,
    has_key: bool,
}

/// One model row in a [`PushEnvelope::Config`] (the Connector panel's ModelForm model).
/// `id` is the config/session-override uuid; `provider` is the serving provider's uuid
/// (matches the ProviderForm option value); `roles` are the lowercase role tokens; and
/// `scope` is `"global"` (from `AppConfig.models`) or `"local"` (from the foreground
/// session's `settings.session_models`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushModel {
    id: String,
    name: String,
    model_id: String,
    provider: String,
    route: String,
    roles: Vec<&'static str>,
    scope: &'static str,
}

/// One MCP-server row in a [`PushEnvelope::Config`] (the McpPanel Server model). `id` is
/// the config uuid. The daemon stores `args` as a `Vec<String>` and `env` as ordered
/// `(key,value)` pairs; both are rendered back into the panel's single-line STRING forms
/// (`args` space-joined, `env` as `K=V, K2=V2`) so the round-trip matches the form
/// exactly (a `SetMcpServer` re-parses them daemon-side).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PushMcpServer {
    id: String,
    name: String,
    enabled: bool,
    transport: &'static str,
    command: String,
    args: String,
    env: String,
    url: String,
}

/// The daemon->JS envelope, tagged on `k`. One variant per bridge message; every
/// field name matches the contract verbatim (camelCase where the contract uses it).
#[derive(serde::Serialize)]
#[serde(tag = "k")]
enum PushEnvelope {
    /// Structural / commit tick (the catch-all): the full committed transcript +
    /// title + palette for `session`. `state` is always `"attached"`.
    Snapshot {
        session: String,
        state: &'static str,
        messages: Vec<PushMsg>,
        title: String,
        palette: PushPalette,
        /// Foreground session's sub-agents (list + status). Authoritative full array —
        /// React REPLACES on each Snapshot, never accumulates.
        subagents: Vec<PushSubAgent>,
        /// Foreground session's background-bash jobs (list + status). Authoritative
        /// full array — React REPLACES on each Snapshot, never accumulates.
        bash: Vec<PushBashJob>,
        /// Foreground session's STAGED composer attachments (chips). Authoritative full
        /// array — React REPLACES on each Snapshot; empty once the message is sent.
        attachments: Vec<PushAttachment>,
    },
    /// The FULL live streaming buffer (React REPLACES the live bubble). Emitted every
    /// frame the buffer changes; an empty `text` clears the bubble on commit.
    StreamMsg { session: String, text: String },
    /// The FULL live reasoning buffer (React REPLACES). Empty `text` clears it.
    Reasoning { session: String, text: String },
    /// Working flag + optional toast. React animates the spinner locally; the host
    /// only says whether the session is working and what toast (if any) to show.
    Status {
        session: String,
        working: bool,
        toast: Option<String>,
    },
    /// The detached session swapper: the `[+ new session]` row + live cooking rows +
    /// on-disk history. `state` is always `"swapper"`.
    Hub {
        state: &'static str,
        cooking: Vec<PushCooking>,
        history: Vec<PushHistory>,
    },
    /// One-shot omnisearch results for `query` — the GUI overlay REPLACES its list with
    /// `items` (each `{ path, label }`; an empty `path` marks a non-attachable dir row).
    /// Pushed out-of-band (not fingerprinted) whenever the daemon answers a `FileSearch`;
    /// `query` is echoed so the overlay can drop a stale/out-of-order reply.
    SearchResults {
        query: String,
        items: Vec<crate::ipc::proto::FileSearchItem>,
    },
    /// The authoritative GLOBAL config catalogue for the Connector + MCP panels. React
    /// REPLACES its config slices on each push. Emitted whenever the projected config
    /// changes (a full snapshot carries it) and re-emitted on `Ready` (page reload) so a
    /// fresh webview always has the current catalogue. `models` folds the global scope
    /// and the foreground session's local-override scope into one list, each row tagged
    /// with its `scope`.
    Config {
        providers: Vec<PushProvider>,
        models: Vec<PushModel>,
        mcp: Vec<PushMcpServer>,
    },
    /// One-shot live model-id catalogue for `provider` (uuid), answering a
    /// `ListModels` — the Connector ModelForm REPLACES its model-id picker options with
    /// `models`. Pushed out-of-band (not fingerprinted) whenever the daemon answers a
    /// `ListModels`; `provider` is echoed so the form can drop a stale/out-of-order reply.
    ModelList { provider: String, models: Vec<String> },
}

/// Per-connection dedup memory for the push pipeline: the last values pushed, so
/// [`serialize_and_push`] / [`push_hub`] only emit an envelope when something
/// actually changed (the fold loop calls them every ~16ms).
pub(super) struct PushState {
    /// Fingerprint of the last `Snapshot` (session + messages + title + palette).
    snapshot_fp: Option<u64>,
    /// Last streaming buffer pushed (`None` once cleared).
    stream: Option<String>,
    /// Last reasoning buffer pushed (empty once cleared).
    reasoning: String,
    /// Last `(working, toast)` pushed.
    status: Option<(bool, Option<String>)>,
    /// Last serialised `Hub` JSON (the swapper is diffed as a whole).
    hub_json: Option<String>,
    /// Last serialised `Config` JSON (the global config catalogue, diffed as a whole so
    /// an unchanged config emits nothing). Cleared by [`reset`](Self::reset) on `Ready`
    /// so a page reload re-emits the full current catalogue.
    config_json: Option<String>,
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
    Attach(String),
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
    ctl_rx: &Receiver<super::HostCtl>,
    last: &mut PushState,
    current_session: Option<&str>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
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
                // A hub pick / new-session request: hand back to the state machine to
                // detach + attach the chosen (or freshly minted) session.
                Ok(super::HostCtl::Select(id)) => return HostTransition::Attach(id),
                Ok(super::HostCtl::New) => {
                    return HostTransition::Attach(uuid::Uuid::new_v4().to_string())
                }
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
            return HostTransition::Attach(uuid::Uuid::new_v4().to_string());
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

        // --- (b-ter) mirror the staged-attachment markers for the ipc Submit append ---
        // The ipc thread appends these `[Image #N]` markers to a chat send so the daemon's
        // submit-time reconcile keeps the staged images (React's text carries no markers).
        if let Ok(mut marks) = live_marks.lock() {
            marks.clear();
            marks.extend(shadow.rest.fg().pending_attachments.iter().map(|a| a.marker_n));
        }

        // --- (c) serialise + push whatever changed (the draw seam) ---
        serialize_and_push(&shadow, push, last);
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

/// Resolve a ratatui [`Color`] to a `#rrggbb` string, mirroring the fallbacks the
/// GUI host uses elsewhere (near-black bg, near-white fg for non-Rgb palettes).
fn color_hex(c: ratatui::style::Color, fallback: &str) -> String {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_string(),
    }
}

/// Serialise the foreground session of `shadow` into the push envelopes and emit any
/// that changed since the last call, through `push` (the host's
/// `window.__komaClient.push` sink). This is the headless twin of `terminal.draw`:
/// the fold loop calls it every frame instead of painting.
///
/// Emits, in order, only when changed: a `Snapshot` (committed transcript + title +
/// palette), a `StreamMsg` (full live buffer, or empty to clear on commit), a
/// `Reasoning` (full live thinking, or empty to clear), and a `Status` (working +
/// toast). `PushState` holds the last-pushed values so a quiescent frame is silent.
pub(super) fn serialize_and_push(shadow: &AppState, push: &dyn Fn(String), last: &mut PushState) {
    let fg = shadow.rest.fg();
    let session = fg.id.clone();

    // Title: the session's display name, falling back to its id, then a constant.
    let title = fg
        .session
        .as_ref()
        .map(|s| {
            if s.settings.name.is_empty() {
                s.id.clone()
            } else {
                s.settings.name.clone()
            }
        })
        .unwrap_or_else(|| "koma".to_string());

    // Palette from the shadow config (a themed daemon repaints the window live).
    let pal = crate::view::theme::palette(&shadow.rest.config);
    let palette = PushPalette {
        bg: color_hex(pal.bg, "#000000"),
        fg: color_hex(pal.fg, "#c8d3f5"),
    };

    // Committed transcript: skip System/Tool (chrome the chat view never shows as a
    // bubble), carry role + content + display-only reasoning for user/assistant.
    let messages: Vec<PushMsg> = fg
        .session
        .as_ref()
        .map(|s| {
            s.conversation
                .messages()
                .iter()
                .filter_map(|m| {
                    let role = match m.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::System | Role::Tool => return None,
                    };
                    Some(PushMsg {
                        role,
                        content: m.content.clone(),
                        reasoning: m.reasoning.clone(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Sub-agents: the foreground session's spawned agents (running + finished), list +
    // status only (no live transcript this wave). `name` = agent name, `summary` = the
    // compact one-line label, `status` = canonical lifecycle string.
    let subagents: Vec<PushSubAgent> = fg
        .subagents
        .iter()
        .map(|sa| PushSubAgent {
            name: sa.agent_name.clone(),
            status: match &sa.status {
                crate::app::subagent::SubAgentStatus::Running => "running",
                crate::app::subagent::SubAgentStatus::Done(_) => "done",
                crate::app::subagent::SubAgentStatus::Killed => "killed",
                crate::app::subagent::SubAgentStatus::Error(_) => "error",
            },
            summary: sa.label.clone(),
        })
        .collect();

    // Background-bash jobs: the foreground session's registry (running + finished),
    // list + status only. `id` = model-facing `bash-<n>`, `cmd` = the command,
    // `status` = canonical lifecycle string.
    let bash: Vec<PushBashJob> = fg
        .bash_jobs
        .iter()
        .map(|job| PushBashJob {
            id: format!("bash-{}", job.id),
            cmd: job.command.clone(),
            status: match job.snapshot_status() {
                crate::app::bgbash::BashJobStatus::Running => "running",
                crate::app::bgbash::BashJobStatus::Done(_) => "done",
                crate::app::bgbash::BashJobStatus::Killed => "killed",
                crate::app::bgbash::BashJobStatus::Error(_) => "error",
            },
        })
        .collect();

    // Staged composer attachments: the foreground session's `pending_attachments` (not
    // yet sent). `marker_n` ties each chip to its `[Image #N]` marker; `kind` is derived
    // from the sniffed mime (all attachments are images today, but keep it general).
    let attachments: Vec<PushAttachment> = fg
        .pending_attachments
        .iter()
        .map(|a| PushAttachment {
            marker_n: a.marker_n,
            name: a.file_name().to_string(),
            kind: if a.mime.starts_with("image/") {
                "image"
            } else {
                "file"
            },
        })
        .collect();

    // --- Snapshot (structural): fingerprint session + transcript + title + palette ---
    let fp = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        session.hash(&mut h);
        title.hash(&mut h);
        palette.bg.hash(&mut h);
        palette.fg.hash(&mut h);
        messages.len().hash(&mut h);
        for m in &messages {
            m.role.hash(&mut h);
            m.content.hash(&mut h);
            m.reasoning.hash(&mut h);
        }
        // Fold sub-agents in so a status/list change re-emits the Snapshot.
        subagents.len().hash(&mut h);
        for sa in &subagents {
            sa.name.hash(&mut h);
            sa.status.hash(&mut h);
            sa.summary.hash(&mut h);
        }
        // Fold bash jobs in so a status/list change re-emits the Snapshot.
        bash.len().hash(&mut h);
        for b in &bash {
            b.id.hash(&mut h);
            b.cmd.hash(&mut h);
            b.status.hash(&mut h);
        }
        // Fold staged attachments in so a stage/drop re-emits the Snapshot (chips).
        attachments.len().hash(&mut h);
        for a in &attachments {
            a.marker_n.hash(&mut h);
            a.name.hash(&mut h);
            a.kind.hash(&mut h);
        }
        h.finish()
    };
    if last.snapshot_fp != Some(fp) {
        last.snapshot_fp = Some(fp);
        let env = PushEnvelope::Snapshot {
            session: session.clone(),
            state: "attached",
            messages,
            title,
            palette,
            subagents,
            bash,
            attachments,
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }

    // --- StreamMsg: full live buffer; empty text clears the bubble on commit ---
    match &fg.streaming {
        Some(text) => {
            if last.stream.as_deref() != Some(text.as_str()) {
                last.stream = Some(text.clone());
                emit(push, &PushEnvelope::StreamMsg {
                    session: session.clone(),
                    text: text.clone(),
                });
            }
        }
        None => {
            if last.stream.is_some() {
                last.stream = None;
                emit(push, &PushEnvelope::StreamMsg {
                    session: session.clone(),
                    text: String::new(),
                });
            }
        }
    }

    // --- Reasoning: full live thinking buffer; empty text clears it ---
    if !fg.stream_reasoning.is_empty() {
        if last.reasoning != fg.stream_reasoning {
            last.reasoning = fg.stream_reasoning.clone();
            emit(push, &PushEnvelope::Reasoning {
                session: session.clone(),
                text: fg.stream_reasoning.clone(),
            });
        }
    } else if !last.reasoning.is_empty() {
        last.reasoning.clear();
        emit(push, &PushEnvelope::Reasoning {
            session: session.clone(),
            text: String::new(),
        });
    }

    // --- Status: working flag (waiting or mid-stream) + optional toast ---
    let working = fg.waiting || fg.streaming.is_some();
    let toast = fg.toast.as_ref().map(|(t, _, _)| t.clone());
    let status = (working, toast);
    if last.status.as_ref() != Some(&status) {
        last.status = Some(status.clone());
        emit(push, &PushEnvelope::Status {
            session,
            working: status.0,
            toast: status.1,
        });
    }
}

/// Serialise a [`SessionHub`] into a `Hub` envelope and push it if it changed since
/// the last call (the swapper is diffed as one whole JSON blob — the panes are small
/// metadata Vecs). Called by the host's swapper state while detached from any daemon.
pub(super) fn push_hub(hub: &SessionHub, push: &dyn Fn(String), last: &mut PushState) {
    use std::time::UNIX_EPOCH;

    let cooking: Vec<PushCooking> = hub
        .cooking
        .iter()
        .map(|e| match e.kind {
            SessionKind::NewSession => PushCooking {
                kind: "new",
                id: None,
                name: e.name.clone(),
                working: None,
                foreground: None,
                dir_label: None,
                current_dir: None,
            },
            SessionKind::Session => PushCooking {
                kind: "session",
                id: e.session_id.clone(),
                name: e.name.clone(),
                working: Some(e.working),
                foreground: Some(e.is_foreground),
                dir_label: Some(e.dir_label.clone()),
                current_dir: Some(e.is_current_dir),
            },
        })
        .collect();

    let history: Vec<PushHistory> = hub
        .history
        .iter()
        .map(|h| PushHistory {
            id: h
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
            name: h.name.clone(),
            last_active: h
                .last_active
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            dir_label: h.dir_label.clone(),
            current_dir: h.is_current_dir,
        })
        .collect();

    let env = PushEnvelope::Hub {
        state: "swapper",
        cooking,
        history,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        if last.hub_json.as_deref() != Some(json.as_str()) {
            last.hub_json = Some(json.clone());
            push(json);
        }
    }
}

/// The GUI-relevant slice of the daemon's authoritative config, cached by
/// [`push_loop`] from each incoming full [`crate::ipc::proto::StateSnapshot`] so the
/// `Config` envelope can be (re)built + diffed independently of the frame stream — e.g.
/// re-emitted on a `Ready` reload without waiting for the next snapshot. Mirrors the
/// four `GlobalSnapshot` config fields: `models` is the GLOBAL scope, `session_models`
/// the foreground session's LOCAL override scope.
struct ConfigProjection {
    providers: Vec<crate::model::app_config::ProviderConn>,
    models: Vec<crate::model::app_config::ModelEntry>,
    session_models: Vec<crate::model::app_config::ModelEntry>,
    mcp_servers: Vec<crate::model::app_config::McpServerEntry>,
}

impl ConfigProjection {
    /// Snapshot the config slice off a [`crate::ipc::proto::GlobalSnapshot`].
    fn from_global(g: &crate::ipc::proto::GlobalSnapshot) -> Self {
        Self {
            providers: g.providers.clone(),
            models: g.config_models.clone(),
            session_models: g.session_models.clone(),
            mcp_servers: g.mcp_servers.clone(),
        }
    }
}

/// Map a persisted [`crate::model::app_config::ModelRole`] to its lowercase wire token
/// (matches the React role tokens + the config serde form).
fn role_token(r: crate::model::app_config::ModelRole) -> &'static str {
    use crate::model::app_config::ModelRole;
    match r {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
        ModelRole::Planner => "planner",
    }
}

/// Build one [`PushModel`] from a persisted [`crate::model::app_config::ModelEntry`],
/// tagged with its `scope` (`"global"` / `"local"`). Roles fold in the legacy single-
/// role field via `effective_roles`.
fn push_model(m: &crate::model::app_config::ModelEntry, scope: &'static str) -> PushModel {
    PushModel {
        id: m.uuid.clone(),
        name: m.name.clone(),
        model_id: m.model_id.clone(),
        provider: m.provider_uuid.clone(),
        route: m.route.clone().unwrap_or_default(),
        roles: m.effective_roles().into_iter().map(role_token).collect(),
        scope,
    }
}

/// Serialise `cfg` into a [`PushEnvelope::Config`] and push it if it changed since the
/// last call. Called every frame from [`push_loop`]; `last.config_json` dedups so an
/// unchanged catalogue is silent, and a `Ready` reset re-emits the full current config.
/// A `None` projection (no snapshot seen yet) is a no-op.
fn push_config(cfg: Option<&ConfigProjection>, push: &dyn Fn(String), last: &mut PushState) {
    let Some(cfg) = cfg else { return };
    use crate::model::app_config::McpTransport;

    let providers: Vec<PushProvider> = cfg
        .providers
        .iter()
        .map(|p| PushProvider {
            id: p.uuid.clone(),
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
            has_key: !p.api_key.is_empty(),
        })
        .collect();

    // Global scope first, then the foreground session's local overrides, each tagged.
    let mut models: Vec<PushModel> = cfg.models.iter().map(|m| push_model(m, "global")).collect();
    models.extend(cfg.session_models.iter().map(|m| push_model(m, "local")));

    let mcp: Vec<PushMcpServer> = cfg
        .mcp_servers
        .iter()
        .map(|s| PushMcpServer {
            id: s.uuid.clone(),
            name: s.name.clone(),
            enabled: s.enabled,
            transport: match s.transport {
                McpTransport::Stdio => "stdio",
                McpTransport::Http => "http",
            },
            command: s.command.clone(),
            // Render the daemon's array/pair forms back into the panel's STRING forms.
            args: s.args.join(" "),
            env: s
                .env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", "),
            url: s.url.clone(),
        })
        .collect();

    let env = PushEnvelope::Config { providers, models, mcp };
    if let Ok(json) = serde_json::to_string(&env) {
        if last.config_json.as_deref() != Some(json.as_str()) {
            last.config_json = Some(json.clone());
            push(json);
        }
    }
}

/// Serialise `env` and hand it to `push`, dropping it silently on the (never-
/// expected) serialisation error rather than panicking mid-frame.
fn emit(push: &dyn Fn(String), env: &PushEnvelope) {
    if let Ok(json) = serde_json::to_string(env) {
        push(json);
    }
}

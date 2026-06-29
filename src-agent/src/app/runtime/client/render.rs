use std::io::{stdout, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{Mode, QuitConfirmState};
use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::ipc::proto::{ClientRequest, DaemonFrame, KeyWire};
use crate::view;

use super::input::{handle_quit_confirm_key, is_detach, local_echo, send_overlay_cancel, QuitConfirmKey};
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

/// The synchronous render loop, decoupled from the socket and paced at ~60fps.
///
/// Each frame, in order: (a) drain ALL pending [`DaemonFrame`]s non-blocking and
/// apply them (snapshot/delta or seq-gap -> Resync); (b) advance animations from a
/// LOCAL monotonic clock (reconcile the comet's `work_since`, re-anchor the loading
/// spinner) — never from daemon ticks; (c) repaint the shadow UNCONDITIONALLY (the
/// ratatui buffer diff makes an unchanged frame ~free); (d) poll terminal input with
/// a ZERO timeout and handle it (local echo for the plain composer edits, forward the
/// rest). The loop NEVER blocks on the socket: if no frame arrived it still paints and
/// animations still advance. Returns when the user detaches (Ctrl-C) or the socket
/// closes.
pub(super) fn render_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    frame_rx: &Receiver<DaemonFrame>,
    req_tx: &Sender<ClientRequest>,
    prebuffered: Vec<DaemonFrame>,
    resume: bool,
) -> Result<()> {
    use std::sync::mpsc::TryRecvError;

    // The shadow is a real AppState reconstructed purely from frames. It starts in
    // a neutral Chat with a single empty session; the first Snapshot replaces it.
    let mut shadow = AppState::new(Mode::Chat);
    // Until the first Snapshot lands the shadow is empty — show a clear status so
    // the screen isn't a blank "ready".
    shadow.rest.status = "attaching…".into();

    // Tracks the last wrap width we sent to the daemon for the agents editor, so we
    // only send `EditorWrapW` when it changes and always re-send on a fresh editor open
    // (the daemon's newly-opened editor starts at usize::MAX). Reset to None whenever
    // the shadow is NOT in the agents full-screen editor so each fresh open re-sends.
    let mut last_sent_wrap_w: Option<usize> = None;

    // Fire once after the first full snapshot lands when launched with --resume.
    let mut resume_fired = false;

    // Per-connection seq tracking (critique #1). `expected` is the seq the NEXT
    // frame should carry. `0` means "not yet seeded" — the first frame seeds it.
    let mut expected: u64 = 0;
    let mut seeded = false;
    // While true, every frame except a fresh Snapshot is dropped: a gap was seen and
    // a Resync was sent, so the shadow is stale until the full snapshot rebuilds it.
    let mut awaiting_resync = false;

    // Apply any frames the pre-render handshake pulled off the wire while hunting for
    // `Hello` (task #142) BEFORE the live drain, through the SAME `apply_frame` path so
    // seq seeding + snapshot/delta handling are identical. Normally empty (the daemon
    // sends `Hello` first), so usually a no-op; when non-empty these are the lowest-seq
    // frames and must be folded first to keep the seq stream gap-free. An `EnterSelect`
    // can't occur this early (it needs a forwarded `/select` first), so a throwaway
    // `select_requested` here is never acted on.
    {
        let mut select_requested = false;
        for frame in prebuffered {
            apply_frame(
                frame,
                &mut shadow,
                &mut expected,
                &mut seeded,
                &mut awaiting_resync,
                &mut select_requested,
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
                        req_tx,
                    );
                }
                Err(TryRecvError::Empty) => break,
                // The reader task dropped its sender: the daemon's socket closed.
                // Nothing more will ever arrive — leave the client.
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // Fire OpenSessionHub once after the first full snapshot lands when the client
        // was launched with --resume / koma agents. Gated on the shadow having a real
        // session so we don't fire against the initial empty placeholder state.
        if resume && !resume_fired && shadow.rest.fg().session.is_some() {
            resume_fired = true;
            let _ = req_tx.send(ClientRequest::OpenSessionHub);
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
        // sends a "toast cleared" delta; the client owns its own dismissal timer).
        if let Some((_, until, _)) = shadow.rest.toast.as_ref() {
            if Instant::now() >= *until {
                shadow.rest.toast = None;
            }
        }

        // --- (c) repaint UNCONDITIONALLY ---
        // ratatui computes the cell-level diff against the previous buffer, so an
        // unchanged frame flushes ~nothing; painting every frame is what lets the
        // local animations advance smoothly without any dirty-tracking.
        terminal.draw(|f| view::draw(f, &shadow))?;

        // --- (c-bis) forward the agents editor wrap width to the daemon ---
        // The shadow's agents editor publishes its wrap_w via interior mutability
        // during draw. The daemon's editor starts at usize::MAX (never rendered),
        // so we send the client-side value whenever it changes. Reset last_sent_wrap_w
        // when not in the agents editor so each fresh editor open triggers a resend
        // (the daemon's freshly-opened editor is back at usize::MAX).
        if let Mode::Agents(ref a) = shadow.mode {
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
                    // forwarding them (daemon stage 12). `Detach`/`Kill` ask the loop
                    // to exit the client.
                    if matches!(shadow.mode, Mode::QuitConfirm(_)) {
                        match handle_quit_confirm_key(&key, req_tx) {
                            QuitConfirmKey::ExitClient => return Ok(()),
                            QuitConfirmKey::Stay => {}
                        }
                        continue;
                    }
                    // Outside the overlay: the ONE locally-interpreted gesture is
                    // Ctrl-C, which detaches the client (leaves the daemon running).
                    if is_detach(&key) {
                        return Ok(());
                    }
                    // Render-ahead: apply the plain composer edits to the shadow NOW
                    // (the daemon's authoritative InputChanged reconciles later), then
                    // forward the key verbatim for the daemon to interpret. Only the
                    // unambiguous text edits are echoed — see `local_echo`.
                    local_echo(&mut shadow, &key);
                    let _ = req_tx.send(ClientRequest::SendKey(KeyWire::from(key)));
                }
                // Mouse wheel scrolls the LOCAL shadow transcript (a pure view
                // concern — the daemon's scroll is its own; scrolling the shadow
                // gives immediate feedback without a round-trip). Bottom-pinning
                // follow is reconstructed from snapshots, so a manual scroll just
                // nudges the local offset for this render.
                Event::Mouse(m) if matches!(shadow.mode, Mode::Chat) => match m.kind {
                    MouseEventKind::ScrollUp => {
                        for _ in 0..3 {
                            shadow.rest.scroll_up();
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        for _ in 0..3 {
                            shadow.rest.scroll_down();
                        }
                    }
                    _ => {}
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
    if let Mode::Loading(s) = &mut shadow.mode {
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

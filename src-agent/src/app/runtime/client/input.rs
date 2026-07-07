use std::sync::mpsc::Sender;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::ipc::proto::{ClientRequest, KeyWire};

/// Apply the SAFE subset of composer edits to the shadow immediately (render-ahead),
/// so typing appears with zero round-trip. The key is ALSO forwarded to the daemon by
/// the caller; the daemon's authoritative [`StateDelta::InputChanged`] (or a full
/// Snapshot) reconciles on a later frame and ALWAYS wins, so a mispredicted echo is
/// self-correcting.
///
/// Only edits that PURELY mutate `input`/`cursor` with no dependence on daemon-side
/// state are echoed — using the EXACT same `AppStateRest` helpers `controller::input`
/// calls, so the local result matches the daemon's byte-for-byte:
///   - a plain `Char(c)` (no Ctrl) — EXCEPT `$` on an empty input, which opens the
///     sub-agents panel daemon-side (a mode change, not a text edit);
///   - `Backspace`, and the pure caret moves `Left` / `Right` / `Home`.
///
/// Everything else is deliberately NOT echoed (forwarded only), because its meaning
/// depends on state the client doesn't authoritatively own: `Enter` (submit / slash /
/// palette-complete), `Up`/`Down` (history recall / palette nav / multiline caret),
/// `End` (scroll-to-bottom when empty, else caret), `Tab`/`BackTab` (completion /
/// mode toggle), `Esc` (interrupt / rewind), and any Ctrl-modified key. Those still
/// reconcile from the daemon's snapshot.
///
/// The echo is suppressed unless the shadow is in plain `Chat` with no modal surface
/// open (sub-agents panel / viewer / tool-approval), matching where the daemon's chat
/// composer actually consumes these keys. (`/help` is now its own mode, so the
/// `Mode::Chat` guard already excludes it.)
pub(crate) fn local_echo(shadow: &mut AppState, key: &KeyEvent) {
    // Only echo in plain Chat with no modal overlay capturing keys. In any other mode
    // (or with a modal open) the daemon routes the key elsewhere, so faking a text
    // edit would desync until the next snapshot corrects it.
    if !matches!(shadow.mode(), Mode::Chat) {
        return;
    }
    let rest = &mut shadow.rest;
    if rest.subagents_open
        || rest.agent_viewer.is_some()
        || rest.fg().awaiting_approval
    {
        return;
    }
    // Never echo a Ctrl-modified key (Ctrl-J newline, Ctrl-V paste, interrupts, …);
    // those are gestures, not plain text the composer inserts at the caret.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return;
    }
    match key.code {
        // `$` on an EMPTY input opens the `$` panel daemon-side (not a typed char), so
        // don't echo it; with text present it is a normal character and echoes below.
        KeyCode::Char('$') if rest.fg().input.is_empty() => {}
        KeyCode::Char(c) => rest.push_char(c),
        KeyCode::Backspace => rest.backspace(),
        KeyCode::Left => rest.cursor_left(),
        KeyCode::Right => rest.cursor_right(),
        KeyCode::Home => rest.cursor_home(),
        // Enter / Up / Down / End / Tab / BackTab / Esc / everything else: forwarded
        // only (handled above by NOT matching here) — the daemon snapshot reconciles.
        _ => {}
    }
}

/// What a key handled inside the mirrored `/quit` overlay tells the render loop to do.
pub(crate) enum QuitConfirmKey {
    /// Tear down the client process (the request to act on it was already queued).
    ExitClient,
    /// Stay attached and keep rendering (cancel, or a swallowed stray key).
    Stay,
}

/// Handle a key while the shadow mirrors the daemon's `/quit` confirm overlay
/// (daemon stage 12). The overlay is a navigable horizontal button row —
/// `[close window (quit)]` `[detach]` `[cancel]` (indices 0/1/2) — whose three choices
/// are CLIENT-process-lifecycle decisions. Two classes of key:
///
/// NAVIGATION (`Left`/`Right`, `Tab`/`Shift+Tab`, `h`/`l`) — the daemon owns the focus
/// index (`selected`), so these are FORWARDED verbatim (like the cancel `Esc`). The
/// daemon's `handle_quit_confirm` moves `selected` and the next snapshot flips the
/// shadow, so the highlight tracks. The client never mutates `selected` itself (it
/// would just be overwritten by the snapshot, and could race). Returns `Stay`.
///
/// ACTIVATION — the client acts on the lifecycle choice itself rather than letting it
/// cross to the daemon, because closing/detaching tears down THIS process. `Enter`
/// activates the CURRENTLY FOCUSED button (`selected`): 0 (close window (quit)) → like `k`,
/// 1 (detach) → like `d`, 2/other → like `Esc`. The direct shortcuts fire regardless
/// of focus:
///   - `[k]` CLOSE THIS WINDOW — KILL this window's daemon. A window now IS its own
///     single-session daemon (daemon-per-session), and the attached client holds the
///     controller seat, so it sends [`ClientRequest::QuitDaemon`] (controller-only,
///     accepted here): the daemon aborts its one session, shuts down, and unlinks its
///     socket. The session is left on disk only → it reappears in the swapper's HISTORY
///     pane (reloadable). Then [`ClientRequest::Detach`] exits THIS client. See
///     [`quit_daemon_and_detach`]. (Phase B change: the old `[k]` sent a per-session
///     `QuitSession`, which only tombstoned the session and left the daemon alive in
///     grace — stale multi-session-per-daemon framing.)
///   - `[d]` DETACH & keep — reset the daemon's overlay back to Chat (a forwarded `Esc`
///     = the daemon's own cancel, so a later reattach lands in Chat, not the stale
///     overlay), send [`ClientRequest::Detach`] (the daemon passes the controller seat
///     and keeps its session COOKING headless), then exit ONLY the client. The live
///     session → it reappears in the swapper's COOKING pane and a resume reattaches to
///     the live process.
///   - `Esc` cancel — forward an `Esc` so the daemon's `handle_quit_confirm`
///     runs `QuitCancel` and returns to Chat; the resulting snapshot flips the shadow
///     back. The client stays attached. (`Ctrl-C` is fully inert now.)
///
/// Every other key is swallowed (the overlay has no text entry — mirrors the daemon's
/// own `handle_quit_confirm`, which returns `Action::None` for anything else).
///
/// Requests share the ordered outbound queue, so the `[k]` (QuitDaemon-then-Detach) and
/// `[d]` (Esc-then-Detach) pairs are delivered in sequence, guaranteeing the daemon
/// processes the kill/cancel before the client drops.
///
/// `selected` is the shadow's current focus index (mirrored from the daemon), used to
/// resolve what `Enter` activates.
pub(crate) fn handle_quit_confirm_key(
    key: &KeyEvent,
    req_tx: &Sender<ClientRequest>,
    selected: usize,
) -> QuitConfirmKey {
    // Ctrl-C is fully inert now (koma disables it): it is no longer handled here
    // (Esc still cancels the overlay via the arm below).
    match key.code {
        // --- Navigation: the daemon owns `selected`, so forward and let its
        // `handle_quit_confirm` move focus; the next snapshot reflects it. ---
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Tab
        | KeyCode::BackTab
        | KeyCode::Char('h')
        | KeyCode::Char('l') => {
            let _ = req_tx.send(ClientRequest::SendKey(KeyWire::from(*key)));
            QuitConfirmKey::Stay
        }
        // --- Activate the focused button (same effect as its direct shortcut). ---
        KeyCode::Enter => match selected {
            0 => {
                // close window (quit) — like `k`: kill this window's daemon, then detach.
                quit_daemon_and_detach(req_tx)
            }
            1 => {
                // detach — like `d`: reset the overlay then detach (daemon keeps cooking).
                send_overlay_cancel(req_tx);
                let _ = req_tx.send(ClientRequest::Detach);
                QuitConfirmKey::ExitClient
            }
            // cancel (2) or any out-of-range — like `Esc`: cancel + stay.
            _ => {
                send_overlay_cancel(req_tx);
                QuitConfirmKey::Stay
            }
        },
        // --- Direct shortcuts (fire regardless of focus). ---
        KeyCode::Char('d') | KeyCode::Char('D') => {
            // Reset the daemon overlay → Chat first, then detach. Ordered queue keeps
            // the sequence, so a reattaching client sees Chat rather than the overlay.
            // No QuitDaemon/QuitSession: the daemon keeps running + cooking headless.
            send_overlay_cancel(req_tx);
            let _ = req_tx.send(ClientRequest::Detach);
            QuitConfirmKey::ExitClient
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            // Kill this window's daemon, then detach this client.
            quit_daemon_and_detach(req_tx)
        }
        KeyCode::Esc => {
            send_overlay_cancel(req_tx);
            QuitConfirmKey::Stay
        }
        // No text entry: swallow every other key (don't forward) so nothing leaks.
        _ => QuitConfirmKey::Stay,
    }
}

/// `[k]` (close window (quit)): KILL this window's daemon, then detach THIS client. A window
/// now IS its own single-session daemon (daemon-per-session), so closing the window ends
/// the daemon — there is no other window or session sharing it to spare.
///
/// Two ordered requests on the shared outbound queue:
///   1. [`ClientRequest::QuitDaemon`] — the controller-only daemon-wide teardown. The
///      attached client holds the controller seat, so the daemon accepts it: it latches
///      its shutdown flag, aborts its one session on the way out, drops the runtime, and
///      UNLINKS its socket. The session is left on disk only → it reappears in the
///      swapper's HISTORY pane (reloadable). The same request `/new kill` already uses to
///      reap a daemon (see `client::client_run`).
///   2. [`ClientRequest::Detach`] — the SAME clean client-exit `[d]` uses, queued AFTER
///      so the daemon flushes its `QuitDaemon` Ack first. With the daemon shutting down it
///      is largely a courtesy (the controller seat is moot once the daemon exits), but it
///      keeps the teardown symmetric with `[d]` and harmless if delivery races the
///      shutdown.
///
/// Returns `ExitClient` so `client_run`'s Attached→Exit arm runs `teardown_connection`,
/// whose writer drains BOTH queued requests to the socket before the connection drops.
fn quit_daemon_and_detach(req_tx: &Sender<ClientRequest>) -> QuitConfirmKey {
    let _ = req_tx.send(ClientRequest::QuitDaemon);
    let _ = req_tx.send(ClientRequest::Detach);
    QuitConfirmKey::ExitClient
}

/// Forward a bare `Esc` so the daemon's `/quit` overlay cancels back to Chat. Used by
/// both the explicit cancel and the detach reset (so the daemon never lingers in
/// QuitConfirm with no input source after the client leaves).
pub(super) fn send_overlay_cancel(req_tx: &Sender<ClientRequest>) {
    let _ = req_tx.send(ClientRequest::SendKey(KeyWire::from(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::empty(),
    ))));
}

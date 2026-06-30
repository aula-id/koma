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
pub(super) fn local_echo(shadow: &mut AppState, key: &KeyEvent) {
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

/// Is this key the client's local DETACH gesture (Ctrl-C)?
///
/// Detaching the client leaves the daemon — and every session — running. Every
/// OTHER key (including Esc, which is meaningful to the remote session's modes) is
/// forwarded to the daemon, so the client never steals a key the session needs.
pub(super) fn is_detach(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// What a key handled inside the mirrored `/quit` overlay tells the render loop to do.
pub(super) enum QuitConfirmKey {
    /// Tear down the client process (the request to act on it was already queued).
    ExitClient,
    /// Stay attached and keep rendering (cancel, or a swallowed stray key).
    Stay,
}

/// Handle a key while the shadow mirrors the daemon's `/quit` confirm overlay
/// (daemon stage 12). The overlay is a navigable horizontal button row —
/// `[close window]` `[minimize]` `[cancel]` (indices 0/1/2) — whose three choices
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
/// activates the CURRENTLY FOCUSED button (`selected`): 0 (close window) → like `k`,
/// 1 (minimize) → like `d`, 2/other → like `Esc`. The direct shortcuts fire regardless
/// of focus:
///   - `[k]` CLOSE THIS WINDOW — close ONLY this client's foreground session via
///     [`ClientRequest::QuitSession`] (the daemon tombstones that one session + repoints
///     every other client off it) then [`ClientRequest::Detach`] to exit THIS client;
///     other windows + their sessions keep running, and the daemon self-exits later only
///     when nothing is left. See [`close_window_and_detach`]. (C4 behaviour change: the
///     old `[k]` sent a daemon-wide `QuitDaemon`, which wrongly killed other windows.)
///   - `[d]` DETACH & keep — reset the daemon's overlay back to Chat (a forwarded `Esc`
///     = the daemon's own cancel, so a later reattach lands in Chat, not the stale
///     overlay), send [`ClientRequest::Detach`] (the daemon passes the controller seat
///     and keeps EVERY session cooking headless), then exit ONLY the client.
///   - `Esc` / `Ctrl-C` cancel — forward an `Esc` so the daemon's `handle_quit_confirm`
///     runs `QuitCancel` and returns to Chat; the resulting snapshot flips the shadow
///     back. The client stays attached.
///
/// Every other key is swallowed (the overlay has no text entry — mirrors the daemon's
/// own `handle_quit_confirm`, which returns `Action::None` for anything else).
///
/// Requests share the ordered outbound queue, so the `[k]` (QuitSession-then-Detach) and
/// `[d]` (Esc-then-Detach) pairs are delivered in sequence, guaranteeing the daemon
/// processes the close/cancel before the client drops.
///
/// `selected` is the shadow's current focus index (mirrored from the daemon), used to
/// resolve what `Enter` activates. `fg_session_id` is this client's foreground session
/// id (from its shadow), the session `[k]` closes.
pub(super) fn handle_quit_confirm_key(
    key: &KeyEvent,
    req_tx: &Sender<ClientRequest>,
    selected: usize,
    fg_session_id: Option<&str>,
) -> QuitConfirmKey {
    // Ctrl-C in the overlay means "cancel", NOT the global detach — match the daemon's
    // `handle_quit_confirm`, which treats Ctrl-C like Esc.
    if is_detach(key) {
        send_overlay_cancel(req_tx);
        return QuitConfirmKey::Stay;
    }
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
                // close this window — like `k`.
                close_window_and_detach(req_tx, fg_session_id)
            }
            1 => {
                // minimize / detach — like `d`: reset the overlay then detach.
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
            send_overlay_cancel(req_tx);
            let _ = req_tx.send(ClientRequest::Detach);
            QuitConfirmKey::ExitClient
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            // Close ONLY this window's foreground session, then detach this client.
            close_window_and_detach(req_tx, fg_session_id)
        }
        KeyCode::Esc => {
            send_overlay_cancel(req_tx);
            QuitConfirmKey::Stay
        }
        // No text entry: swallow every other key (don't forward) so nothing leaks.
        _ => QuitConfirmKey::Stay,
    }
}

/// `[k]` (close this window): close ONLY the acting client's foreground session, then
/// detach THIS client — a PER-WINDOW close, NOT a daemon-wide teardown (C4 behaviour
/// change). With per-client windows, the old `QuitDaemon` wrongly killed every other
/// window's live sessions; now each window's `[k]` only tears down its own.
///
/// Two ordered requests on the shared outbound queue:
///   1. [`ClientRequest::QuitSession`] of this client's foreground session id — the
///      EXACT tombstone path `/quit`-a-single-session uses: the daemon `close()`s that
///      session (aborts its stream + sub-agents, releases its lock, slot stays so no
///      index shifts) and runs `repoint_foreground_off_closed`, which repoints EVERY
///      other client whose pointer named that session onto a still-live one (so no other
///      window is left looking at a tombstone). The closed session's stale `QuitConfirm`
///      mode is never projected again, so no overlay-cancel is needed.
///   2. [`ClientRequest::Detach`] — the SAME clean client-exit `[d]` uses: the daemon
///      deregisters this client and passes the controller seat; every OTHER session +
///      client keeps running headless. The daemon self-exits later, grace-timed, only
///      once NO session remains AND no client is attached — so a lone window closing its
///      one session still ends the daemon (same end state as before), while other live
///      windows keep it up.
///
/// `fg_session_id` is this client's foreground session id (from its shadow). `None`
/// (no foreground to close — shouldn't happen in the overlay) degrades to a plain
/// detach: nothing to tombstone, just leave this window.
fn close_window_and_detach(
    req_tx: &Sender<ClientRequest>,
    fg_session_id: Option<&str>,
) -> QuitConfirmKey {
    if let Some(id) = fg_session_id {
        let _ = req_tx.send(ClientRequest::QuitSession {
            session_id: id.to_string(),
        });
    }
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

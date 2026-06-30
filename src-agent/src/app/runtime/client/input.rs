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
    if !matches!(shadow.mode, Mode::Chat) {
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
        KeyCode::Char('$') if rest.input.is_empty() => {}
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
/// `[quit & kill]` `[minimize]` `[cancel]` (indices 0/1/2) — whose three choices
/// are CLIENT-process-lifecycle decisions. Two classes of key:
///
/// NAVIGATION (`Left`/`Right`, `Tab`/`Shift+Tab`, `h`/`l`) — the daemon owns the focus
/// index (`selected`), so these are FORWARDED verbatim (like the cancel `Esc`). The
/// daemon's `handle_quit_confirm` moves `selected` and the next snapshot flips the
/// shadow, so the highlight tracks. The client never mutates `selected` itself (it
/// would just be overwritten by the snapshot, and could race). Returns `Stay`.
///
/// ACTIVATION — the client acts on the lifecycle choice itself rather than letting it
/// cross to the daemon, because killing/detaching tears down THIS process. `Enter`
/// activates the CURRENTLY FOCUSED button (`selected`): 0 (quit & kill) → like `k`,
/// 1 (minimize) → like `d`, 2/other → like `Esc`. The direct shortcuts fire regardless
/// of focus:
///   - `[k]` KILL ALL & quit — send [`ClientRequest::QuitDaemon`]; the daemon latches
///     its shutdown flag, tombstones every session, releases all locks, unlinks its
///     socket, and self-exits via its graceful teardown. Then exit the client. (No
///     `Esc` first: the daemon is shutting down wholesale.)
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
/// Requests share the ordered outbound queue, so the `[d]` (and `Enter`-on-minimize)
/// pair is delivered Esc-then-Detach in sequence, guaranteeing the daemon leaves the
/// overlay before the client drops.
///
/// `selected` is the shadow's current focus index (mirrored from the daemon), used to
/// resolve what `Enter` activates.
pub(super) fn handle_quit_confirm_key(
    key: &KeyEvent,
    req_tx: &Sender<ClientRequest>,
    selected: usize,
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
                // quit & kill — like `k`.
                let _ = req_tx.send(ClientRequest::QuitDaemon);
                QuitConfirmKey::ExitClient
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
            // Tell the daemon to shut down entirely; it tombstones every session,
            // releases locks, unlinks the socket, and self-exits gracefully.
            let _ = req_tx.send(ClientRequest::QuitDaemon);
            QuitConfirmKey::ExitClient
        }
        KeyCode::Esc => {
            send_overlay_cancel(req_tx);
            QuitConfirmKey::Stay
        }
        // No text entry: swallow every other key (don't forward) so nothing leaks.
        _ => QuitConfirmKey::Stay,
    }
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

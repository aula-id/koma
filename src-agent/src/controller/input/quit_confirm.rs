//! Key handler for the `/quit` confirm overlay (`Mode::QuitConfirm`).
//!
//! The overlay is a navigable horizontal button row — `[quit & kill]`,
//! `[minimize]`, `[cancel]` (indices 0/1/2). Two ways to drive it:
//!
//!   * NAVIGATE then activate: Left/Right (or `h`/`l`) and Tab/Shift+Tab move
//!     focus across the row (mutating `s.selected`); Enter activates the focused
//!     button.
//!   * DIRECT shortcuts: `k` kill all, `d` minimize/detach, `Esc`/`Ctrl+C`
//!     cancel — fire their action immediately regardless of focus.
//!
//! Activation maps the focused index to an action:
//!   `0` → [`Action::QuitKillAll`], `1` → [`Action::QuitDetach`],
//!   `2` → [`Action::QuitCancel`].
//!
//! The overlay carries its own snapshot state (the busy-session count + focused
//! index), so `_rest` is unused — mirroring [`super::handle_session_hub`].

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use crate::app::mode::QuitConfirmState;
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Map a focused button index to its action. Order matches the view + the
/// event-loop click hit-test: `0` = quit & kill, `1` = minimize (detach),
/// `2` = cancel. Out-of-range falls back to the safe cancel.
fn action_for(idx: usize) -> Action {
    match idx {
        0 => Action::QuitKillAll,
        1 => Action::QuitDetach,
        _ => Action::QuitCancel,
    }
}

/// Handle a key press inside the quit-confirm overlay.
///
/// Navigation (Left/Right, `h`/`l`, Tab/Shift+Tab) mutates `s.selected` and
/// returns [`Action::None`]; Enter activates the focused button; the direct
/// `k`/`d`/`Esc`/`Ctrl+C` shortcuts fire immediately. Every other key is
/// swallowed so a stray press can't leak into the chat input underneath or
/// accidentally exit.
pub fn handle_quit_confirm(
    s: &mut QuitConfirmState,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    // Ctrl+C here means "get me out of this overlay", NOT "force quit" — the user
    // already has explicit kill/minimize choices, so treat it like Esc (cancel).
    if is_ctrl(&key, 'c') {
        return Action::QuitCancel;
    }

    match key.code {
        // --- Navigate the button row (focus only; no action) ---
        KeyCode::Left | KeyCode::Char('h') => {
            s.selected = s.selected.saturating_sub(1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') => {
            s.selected = (s.selected + 1).min(2);
            Action::None
        }
        KeyCode::Tab => {
            s.selected = (s.selected + 1) % 3;
            Action::None
        }
        // crossterm reports Shift+Tab as BackTab.
        KeyCode::BackTab => {
            s.selected = (s.selected + 2) % 3;
            Action::None
        }
        // --- Activate the focused button ---
        KeyCode::Enter => action_for(s.selected),
        // --- Direct shortcuts (fire regardless of focus) ---
        KeyCode::Char('k') | KeyCode::Char('K') => Action::QuitKillAll,
        KeyCode::Char('d') | KeyCode::Char('D') => Action::QuitDetach,
        KeyCode::Esc => Action::QuitCancel,
        // No text entry: swallow every other key so nothing leaks through.
        _ => Action::None,
    }
}

//! Key handler for the `/quit` confirm overlay (`Mode::QuitConfirm`).
//!
//! Opened only when a quit is requested while a session is still working. Three
//! keyed choices, no list to navigate:
//!   `k` → kill all & quit  ([`Action::QuitKillAll`])
//!   `d` → detach & quit    ([`Action::QuitDetach`])
//!   `Esc` / `Ctrl+C` → cancel back to Chat ([`Action::QuitCancel`])
//!
//! The overlay carries its own snapshot state (the busy-session count), so
//! `_rest` is unused — mirroring [`super::handle_session_hub`].

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use crate::app::mode::QuitConfirmState;
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Handle a key press inside the quit-confirm overlay.
///
/// `k`/`K` kill all & quit, `d`/`D` detach & quit, `Esc` (or `Ctrl+C`) cancel.
/// Every other key is swallowed so a stray press can't leak into the chat input
/// underneath or accidentally exit.
pub fn handle_quit_confirm(
    _s: &mut QuitConfirmState,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    // Ctrl+C here means "get me out of this overlay", NOT "force quit" — the user
    // already has explicit kill/detach choices, so treat it like Esc (cancel).
    if is_ctrl(&key, 'c') {
        return Action::QuitCancel;
    }

    match key.code {
        KeyCode::Char('k') | KeyCode::Char('K') => Action::QuitKillAll,
        KeyCode::Char('d') | KeyCode::Char('D') => Action::QuitDetach,
        KeyCode::Esc => Action::QuitCancel,
        // No text entry: swallow every other key so nothing leaks through.
        _ => Action::None,
    }
}

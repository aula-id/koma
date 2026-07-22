//! Key handler for the `/extension` installed-extension manager (`Mode::Extensions`).
//!
//! A read-only sibling of [`super::mcp`] with no editor: three sub-modes (deepest first):
//!
//! 0. **UninstallConfirm** – modal y/n; `y` uninstalls (`Action::UninstallExtension`),
//!    `n`/Esc cancels back to Detail.
//! 1. **Detail** – ↑/↓ move the tui-screen cursor; Enter opens the selected extension screen
//!    (`Action::ExtScreenOpen`); `u` arms the uninstall confirm; Esc returns to Browse.
//! 2. **Browse** – ↑/↓ move the LIST cursor; →/Enter open the selected extension's detail;
//!    Esc closes the dashboard (`Action::CloseExtensions`).

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::{ExtSubMode, ExtensionsState};
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key press inside the `/extension` manager.
pub fn handle_extensions(
    s: &mut ExtensionsState,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    match s.sub_mode {
        // --- UninstallConfirm: modal y/n ---
        ExtSubMode::UninstallConfirm => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::UninstallExtension,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                s.sub_mode = ExtSubMode::Detail;
                Action::None
            }
            _ => Action::None,
        },

        // --- Detail: read the selected extension + open its TUI screens ---
        ExtSubMode::Detail => match key.code {
            KeyCode::Esc => {
                s.sub_mode = ExtSubMode::Browse;
                Action::None
            }
            KeyCode::Up => {
                s.screen_up();
                Action::None
            }
            KeyCode::Down => {
                s.screen_down();
                Action::None
            }
            // Enter on a tui-screen row opens the extension-driven screen (no-op when the
            // extension declares none).
            KeyCode::Enter => {
                if s.current_tui_screens_len() > 0 {
                    Action::ExtScreenOpen
                } else {
                    Action::None
                }
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                if s.current().is_some() {
                    s.sub_mode = ExtSubMode::UninstallConfirm;
                }
                Action::None
            }
            _ => Action::None,
        },

        // --- Browse: navigate the installed-extension LIST ---
        ExtSubMode::Browse => match key.code {
            KeyCode::Esc => Action::CloseExtensions,
            KeyCode::Up => {
                s.list_up();
                Action::None
            }
            KeyCode::Down | KeyCode::Tab => {
                s.list_down();
                Action::None
            }
            KeyCode::Enter | KeyCode::Right => {
                if s.current().is_some() {
                    s.enter_detail();
                }
                Action::None
            }
            _ => Action::None,
        },
    }
}

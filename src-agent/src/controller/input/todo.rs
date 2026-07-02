//! Key handler for the `/todo` task-panel (`Mode::Todo`).
//!
//! A read-only master/detail panel — no sub-modes and no editing, so the
//! dispatch is simple: navigate the item list, view the selected item's details,
//! close.
//!
//! Key map:
//! - `Esc`           → `Action::CloseTodo` (return to Chat)
//! - `Ctrl+C`        → `Action::None` (fully inert — koma disables Ctrl+C)
//! - `Up`            → move the LIST cursor up
//! - `Down`          → move the LIST cursor down
//! - `j`/`k`         → vim-style navigation (optional, mirrors chat scroll)

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::TodoState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/todo` task-panel.
///
/// Re-reads the todo list from the session's memory on each key so the panel
/// stays current if the model writes new todos via the todowrite tool.
pub fn handle_todo(s: &mut TodoState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Ctrl+C is fully inert (koma disables it): swallow it here so it can't
    // fall through to any close/quit. Esc still closes the panel.
    if is_ctrl(&key, 'c') {
        return Action::None;
    }

    match key.code {
        KeyCode::Esc => Action::CloseTodo,
        KeyCode::Up | KeyCode::Char('k') => {
            s.move_up();
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            s.move_down();
            Action::None
        }
        // Any other key closes the panel (mirrors the /bash overlay).
        _ => Action::CloseTodo,
    }
}

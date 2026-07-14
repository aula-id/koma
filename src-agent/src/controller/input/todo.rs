//! Key handler for the `/todo` task-panel (`Mode::Todo`).
//!
//! A master/detail panel — navigate the item list, view the selected item's
//! details, reset items to pending via Enter, or close. No sub-modes.
//!
//! Key map:
//! - `Esc`           → `Action::CloseTodo` (return to Chat)
//! - `Enter`         → reset selected item to pending (signals model to redo)
//! - `Ctrl+C`        → `Action::None` (fully inert — koma disables Ctrl+C)
//! - `Up`/`k`        → move the LIST cursor up
//! - `Down`/`j`      → move the LIST cursor down

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::TodoState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/todo` task-panel.
///
/// Re-reads the todo list from the session's memory on each key so the panel
/// stays current if the model writes new todos via the checklist tool.
pub fn handle_todo(s: &mut TodoState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Ctrl+C is fully inert (koma disables it): swallow it here so it can't
    // fall through to any close/quit. Esc still closes the panel.
    if is_ctrl(&key, 'c') {
        return Action::None;
    }

    let action = match key.code {
        KeyCode::Esc => Action::CloseTodo,
        KeyCode::Enter => {
            // Reset the selected item to pending — signals the model to redo it.
            // Locked items (the plan-mode rails: "serve plan to user" / "save plan
            // to file & prompt approval") are system-managed — the model can't
            // touch them either (enforced in the checklist interception) — so a
            // locked selection is a no-op here rather than resetting it.
            if !s.current().map(|item| item.locked).unwrap_or(false) {
                s.reset_to_pending();
            }
            Action::None
        }
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
    };

    // Re-read from disk after every key so the overlay stays live.
    s.refresh_from_disk();

    action
}

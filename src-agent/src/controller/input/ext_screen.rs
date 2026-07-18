//! Key handler for an EXTENSION-DRIVEN TUI screen (`Mode::ExtScreen`).
//!
//! The screen content + its menu come from the extension (TUI SCREEN PROTOCOL v1). koma owns
//! ONLY the menu cursor: ↑/↓ walk the union of every menu node's items LOCALLY (mutating the
//! mode, projected to an attached client); Enter fires the async `tui-select` invoke for the
//! highlighted item (`Action::ExtScreenSelect`); Esc fires a best-effort `tui-close` and pops
//! back to the `/extension` detail view (`Action::ExtScreenClose`). Every other key is
//! swallowed — the screen has no text entry.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::ExtScreenState;
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key press inside an open extension screen.
pub fn handle_ext_screen(s: &mut ExtScreenState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::ExtScreenClose,
        KeyCode::Up => {
            s.menu_up();
            Action::None
        }
        KeyCode::Down => {
            s.menu_down();
            Action::None
        }
        // Fire a select only when an invoke isn't already in flight and there's a selectable
        // menu item under the cursor; otherwise swallow it.
        KeyCode::Enter => {
            if s.waiting || s.selected_menu_item().is_none() {
                Action::None
            } else {
                Action::ExtScreenSelect
            }
        }
        _ => Action::None,
    }
}

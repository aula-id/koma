//! Key handler for the `/security` daemon control panel (`Mode::Security`).
//!
//! A read-only control panel — no sub-modes and no editing, so the dispatch is
//! simple: escape/quit, cursor navigation, and daemon lifecycle keys.
//!
//! Key map:
//! - `Esc`      → `Action::CloseSecurity` (return to Chat)
//! - `Ctrl+C`   → `Action::Quit`
//! - `Up`       → move cursor up in the tool inventory
//! - `Down`     → move cursor down in the tool inventory
//! - `t`        → `Action::SecurityToggle` (enable/disable + start/stop)
//! - `s`        → `Action::SecurityStart`
//! - `x`        → `Action::SecurityStop`
//! - `r`        → `Action::SecurityRestart`

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::SecurityState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/security` control panel.
pub fn handle_security(s: &mut SecurityState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => Action::CloseSecurity,
        KeyCode::Up => {
            s.move_up();
            Action::None
        }
        KeyCode::Down => {
            s.move_down();
            Action::None
        }
        KeyCode::Char('t') | KeyCode::Char('T') => Action::SecurityToggle,
        KeyCode::Char('s') | KeyCode::Char('S') => Action::SecurityStart,
        KeyCode::Char('x') | KeyCode::Char('X') => Action::SecurityStop,
        KeyCode::Char('r') | KeyCode::Char('R') => Action::SecurityRestart,
        _ => Action::None,
    }
}

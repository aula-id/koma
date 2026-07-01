//! Key handler for the `--resume` session picker (`Mode::SessionPicker`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::mode::PickerState;
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Handle a key press inside the `--resume` session picker.
///
/// Typing characters updates the live search query and triggers `refilter`.
/// Typing `/new` + Enter spawns a fresh session and drops into Chat.
/// Esc returns to Chat when an active session exists (opened via /resume), or quits
/// when there is no session (opened by the --resume startup flag). Ctrl+C is inert.
pub fn handle_picker(p: &mut PickerState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Ctrl+C is fully inert (koma disables it); Esc keeps its behavior intact
    // (cancel to Chat when a session exists, else quit the --resume startup picker).
    if is_ctrl(&key, 'c') {
        return Action::None;
    }

    match key.code {
        KeyCode::Esc => {
            if rest.fg().session.is_some() {
                Action::CancelPickerToChat
            } else {
                Action::Quit
            }
        }
        KeyCode::Up => {
            p.move_up();
            Action::None
        }
        KeyCode::Down => {
            p.move_down();
            Action::None
        }
        KeyCode::Enter => {
            if p.query == "/new" {
                p.query.clear();
                p.refilter();
                Action::PickerNewSession
            } else {
                Action::PickerSelect
            }
        }
        KeyCode::Backspace => {
            p.query.pop();
            p.refilter();
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            p.query.push(c);
            p.refilter();
            Action::None
        }
        _ => Action::None,
    }
}

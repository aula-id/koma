//! Key handler for the message-rewind picker (`Mode::MessageRewind`).
//!
//! Opened by a double-Esc while idle in Chat. Up/Down (and PageUp/PageDown)
//! navigate the list of prior user messages (chronological — newest at the bottom,
//! pre-selected); Esc cancels back to Chat unchanged; Enter selects the highlighted
//! message to rewind to.

use super::Action;
use crate::app::mode::RewindState;
use crate::app::state::AppStateRest;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// Handle a key press inside the message-rewind picker.
///
/// `_rest` is accepted for handler-signature consistency with the other mode
/// handlers but is unused here (the picker carries its own state). Esc cancels
/// back to Chat without changing the conversation.
pub fn handle_rewind(rw: &mut RewindState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // How far PageUp / PageDown jump through the list (clamped by move_up/move_down).
    const PAGE: usize = 10;

    match key.code {
        KeyCode::Esc => Action::RewindCancel,
        KeyCode::Up => {
            rw.move_up();
            Action::None
        }
        KeyCode::Down => {
            rw.move_down();
            Action::None
        }
        KeyCode::PageUp => {
            for _ in 0..PAGE {
                rw.move_up();
            }
            Action::None
        }
        KeyCode::PageDown => {
            for _ in 0..PAGE {
                rw.move_down();
            }
            Action::None
        }
        KeyCode::Enter => match rw.selected_entry() {
            // Carry the selected user message's vec index out to the runtime so
            // it can cut the conversation to just before that turn.
            Some(entry) => Action::RewindToMessage(entry.vec_index),
            None => Action::RewindCancel,
        },
        _ => Action::None,
    }
}

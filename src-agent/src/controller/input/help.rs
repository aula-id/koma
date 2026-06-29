//! Key handler for the full-screen, searchable `/help` reference + launcher
//! (`Mode::Help`).
//!
//! A read-only filter/select surface (no editing, nothing to persist), so the
//! dispatch is flat:
//!
//! - Printable char → push to `query`, refilter.
//! - Backspace → pop from `query`, refilter.
//! - Up/Down → move the selection over the filtered list.
//! - Enter → LAUNCH the highlighted entry if it is a COMMAND: close help and
//!   dispatch that command through the normal slash pipeline ([`Action::HelpRun`]
//!   carries the parsed [`Command`]). For a KEYBINDING entry (reference only) and
//!   for an empty list, Enter just closes ([`Action::CloseHelp`]).
//! - Esc → close back to Chat ([`Action::CloseHelp`]).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::{HelpKind, HelpState};
use crate::app::state::AppStateRest;
use crate::controller::command;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/help` reference + launcher.
///
/// `_rest` is accepted for handler-signature consistency with the other modes
/// (and to leave room for status feedback later); it is currently unused because
/// the Help screen mutates only its own [`HelpState`] and routes closes/launches
/// through the returned [`Action`] (the runtime owns `state.mode`).
pub fn handle_help(st: &mut HelpState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => Action::CloseHelp,

        KeyCode::Up => {
            st.move_up();
            Action::None
        }
        KeyCode::Down => {
            st.move_down();
            Action::None
        }

        KeyCode::Enter => match st.selected_entry() {
            // Launch a command: hand the runtime the parsed command so it closes
            // help and runs it through the exact same pipeline as a typed slash.
            // `entry.key` already carries the leading `/`, which `parse` strips.
            Some(entry) if entry.kind == HelpKind::Command => {
                Action::HelpRun(command::parse(&entry.key))
            }
            // Keybinding row (reference only) or empty list: Enter just closes.
            _ => Action::CloseHelp,
        },

        KeyCode::Backspace => {
            st.query.pop();
            st.refilter();
            Action::None
        }

        // Printable input drives the search (ignore Ctrl-modified keys so chords
        // like Ctrl+C above aren't typed into the query).
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            st.query.push(c);
            st.refilter();
            Action::None
        }

        _ => Action::None,
    }
}

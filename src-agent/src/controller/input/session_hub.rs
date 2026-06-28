//! Key handler for the unified session hub (`/resume`, `Mode::SessionHub`).
//!
//! Tab toggles the focused pane (cooking <-> history); Up/Down move the selection
//! within the focused pane; Enter acts on the focused pane's selection; Esc/Ctrl+C
//! close back to Chat without touching any session state.
//!
//! - Enter on a COOKING row -> [`Action::LiveSwitch`] carrying that session's
//!   `sessions` index (REUSES the `/swap` foreground-switch path verbatim).
//! - Enter on a HISTORY row -> [`Action::HubOpenHistory`] carrying that row's
//!   index; the runtime resolves it to a path and loads it into a new tab.
//!
//! The hub carries its own state (a snapshot of live + on-disk sessions), so
//! `_rest` is unused — mirroring [`super::handle_rewind`].

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use crate::app::mode::{HubPane, SessionHub, SessionKind};
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Handle a key press inside the session hub.
///
/// Esc and Ctrl+C both close back to Chat unchanged. Enter resolves against the
/// FOCUSED pane: cooking -> foreground switch (or new session on the synthetic
/// row), history -> load into a new tab. `N` is a global shortcut for `/new`
/// regardless of which pane is focused.
pub fn handle_session_hub(
    hub: &mut SessionHub,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    if is_ctrl(&key, 'c') {
        return Action::CloseSessionHub;
    }

    match key.code {
        KeyCode::Esc => Action::CloseSessionHub,
        KeyCode::Tab | KeyCode::BackTab => {
            hub.toggle_focus();
            Action::None
        }
        KeyCode::Up => {
            hub.move_up();
            Action::None
        }
        KeyCode::Down => {
            hub.move_down();
            Action::None
        }
        KeyCode::Enter => match hub.focus {
            HubPane::Cooking => match hub.selected_cooking() {
                Some(entry) if entry.kind == SessionKind::NewSession => {
                    Action::Slash(crate::controller::command::Command::New)
                }
                Some(entry) => Action::LiveSwitch(entry.idx),
                None => Action::CloseSessionHub,
            },
            HubPane::History => match hub.selected_history() {
                // Carry the history-row index; the runtime reads the path back out
                // of the hub state and loads it (non-destructively) into a new tab.
                Some(_) => Action::HubOpenHistory(hub.history_selected),
                None => Action::CloseSessionHub,
            },
        },
        KeyCode::Char('n') | KeyCode::Char('N') => Action::Slash(crate::controller::command::Command::New),
        _ => Action::None,
    }
}

//! Key handler for the `/mcp` server management dashboard (`Mode::Mcp`).
//!
//! A simpler sibling of [`super::agents`]: there are no pickers and no
//! full-screen body editor, so the dispatch is just three layers (deepest first):
//!
//! 0. **DeleteConfirm** – modal y/n; `y` deletes (`Action::DeleteMcp`),
//!    `n`/Esc cancels back to Browse.
//!
//! 1. **Edit/Create** – ↑/↓ move the field cursor. On a TEXT field, Enter starts
//!    inline editing (Char/Backspace mutate the draft, Enter/Esc commit). On a
//!    TOGGLE field (Enabled / Transport), Space/Enter/←/→ flip the value in place
//!    (never enters text-edit). `s` saves/creates; Esc cancels back to Browse.
//!
//! 2. **Browse** – ↑/↓ move the LIST cursor; →/Enter open the selected server for
//!    editing; `n` starts Create; `d` deletes the selected server; Esc closes the
//!    dashboard (`Action::CloseMcp`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::McpState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/mcp` management dashboard.
pub fn handle_mcp(s: &mut McpState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::McpSubMode;

    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match s.mode {
        // --- DeleteConfirm: modal y/n ---
        McpSubMode::DeleteConfirm => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::DeleteMcp,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                s.mode = McpSubMode::Browse;
                Action::None
            }
            _ => Action::None,
        },

        // --- Edit / Create ---
        McpSubMode::Edit | McpSubMode::Create => {
            if s.editing {
                // Typing into the highlighted draft text field.
                match key.code {
                    // Commit the field; stay in the editor.
                    KeyCode::Esc | KeyCode::Enter => {
                        s.editing = false;
                        Action::None
                    }
                    KeyCode::Backspace => {
                        s.backspace();
                        Action::None
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        s.push_char(c);
                        Action::None
                    }
                    _ => Action::None,
                }
            } else {
                // Navigating the field list.
                match key.code {
                    KeyCode::Esc => {
                        s.cancel();
                        Action::None
                    }
                    KeyCode::Up => {
                        s.field_up();
                        Action::None
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        s.field_down();
                        Action::None
                    }
                    // ←/→ and Space flip a toggle field in place; on a text field
                    // they do nothing (text is entered via Enter → editing).
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                        if s.field.is_toggle() =>
                    {
                        s.toggle_field();
                        Action::None
                    }
                    KeyCode::Enter => {
                        if s.field.is_toggle() {
                            // Enter also flips a toggle (consistent with Space/←/→).
                            s.toggle_field();
                        } else {
                            // Text field → start inline editing.
                            s.editing = true;
                        }
                        Action::None
                    }
                    // Save (Edit) / create (Create). Name + the transport-specific
                    // connection field are required so we never persist a server
                    // that can never connect.
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        if s.draft_name.trim().is_empty() {
                            rest.status = "name required".into();
                            Action::None
                        } else if let Some(missing) = missing_required(s) {
                            rest.status = missing.into();
                            Action::None
                        } else if s.mode == McpSubMode::Create {
                            Action::CreateMcp
                        } else {
                            Action::SaveMcp
                        }
                    }
                    _ => Action::None,
                }
            }
        }

        // --- Browse: navigate the LIST ---
        McpSubMode::Browse => match key.code {
            KeyCode::Esc => Action::CloseMcp,
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
                    s.enter_edit();
                }
                Action::None
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                s.enter_create();
                Action::None
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if s.current().is_some() {
                    s.enter_delete();
                }
                Action::None
            }
            _ => Action::None,
        },
    }
}

/// The transport-specific required field that is still blank, as a status
/// message, or `None` when the draft has everything it needs to connect. Name is
/// checked separately by the caller.
fn missing_required(s: &McpState) -> Option<&'static str> {
    use crate::model::app_config::McpTransport;
    match s.draft_transport {
        McpTransport::Stdio if s.draft_command.trim().is_empty() => Some("command required (stdio)"),
        McpTransport::Http if s.draft_url.trim().is_empty() => Some("url required (http)"),
        _ => None,
    }
}

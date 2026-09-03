//! Key handler for the Ctrl+P attachments panel (`Mode::Attachments`).

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::AttachmentsState;
use crate::app::state::AppStateRest;
use crate::controller::input::is_ctrl;

use super::Action;

/// Handle a key inside the attachments list / nested paste editor.
pub fn handle_attachments(s: &mut AttachmentsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Nested editor takes priority (Agents-style).
    if let Some((_, ref mut ed)) = s.editor.as_mut() {
        return match key.code {
            KeyCode::Esc => {
                // Save to disk then return to list (not Chat).
                if let Some(sess) = rest.fg().session.as_ref() {
                    let dir = sess.path.clone();
                    let _ = s.commit_editor(&dir);
                } else {
                    s.editor = None;
                }
                Action::None
            }
            KeyCode::Enter => {
                ed.newline();
                Action::None
            }
            KeyCode::Backspace => {
                ed.backspace();
                Action::None
            }
            KeyCode::Delete => {
                ed.delete();
                Action::None
            }
            KeyCode::Left => {
                ed.move_left();
                Action::None
            }
            KeyCode::Right => {
                ed.move_right();
                Action::None
            }
            KeyCode::Up => {
                ed.move_up();
                Action::None
            }
            KeyCode::Down => {
                ed.move_down();
                Action::None
            }
            KeyCode::Home => {
                ed.home();
                Action::None
            }
            KeyCode::End => {
                ed.end();
                Action::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(ratatui::crossterm::event::KeyModifiers::CONTROL) => {
                ed.insert_char(c);
                Action::None
            }
            _ => Action::None,
        };
    }

    // List view.
    match key.code {
        KeyCode::Esc => Action::CloseAttachments,
        KeyCode::Up | KeyCode::Char('k') => {
            s.move_up();
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            s.move_down();
            Action::None
        }
        KeyCode::Enter => {
            if let Some(att) = s.current() {
                if att.is_pasted_text() {
                    if let Some(sess) = rest.fg().session.as_ref() {
                        let dir = sess.path.clone();
                        s.open_paste_editor(&dir);
                    }
                }
                // Image: path is shown in the detail pane; no pixel editor.
            }
            Action::None
        }
        KeyCode::Char('d') | KeyCode::Char('D') => Action::AttachmentsRemoveSelected,
        // Ctrl+X also removes (bash-panel convention for destructive list ops).
        _ if is_ctrl(&key, 'x') => Action::AttachmentsRemoveSelected,
        // Any other key closes (mirrors /bash /todo).
        _ => Action::CloseAttachments,
    }
}

//! Keyboard input handler for the `/remote` host manager.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::remote::{HostEditField, RemoteIntent, RemoteState, RemoteView};
use crate::controller::input::is_ctrl;
use crate::controller::input::Action;

pub fn handle_remote(m: &mut RemoteState, key: KeyEvent) -> Action {
    match m.view {
        RemoteView::Browse => handle_browse(m, key),
        RemoteView::SessionHub => handle_session_hub(m, key),
        RemoteView::Edit => handle_editor(m, key),
    }
}

fn handle_browse(m: &mut RemoteState, key: KeyEvent) -> Action {
    // Delete confirm modal
    if let Some(id) = m.pending_delete.clone() {
        return match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                m.pending_delete = None;
                Action::RemoteDeleteHost(id)
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                m.pending_delete = None;
                Action::None
            }
            _ => Action::None,
        };
    }

    match key.code {
        KeyCode::Esc => Action::CloseRemote,
        KeyCode::Up | KeyCode::Char('k') => {
            m.move_up();
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            m.move_down();
            Action::None
        }
        KeyCode::Enter => {
            // Connect to selected host.
            if let Some(host_id) = m.select_current_host() {
                Action::RemoteConnect(host_id)
            } else {
                Action::None
            }
        }
        KeyCode::Char('n' | 'N') if m.intent == RemoteIntent::Manage => Action::RemoteAddHost,
        KeyCode::Char('d' | 'D') if m.intent == RemoteIntent::Manage => {
            if let Some(host) = m.selected_host() {
                m.pending_delete = Some(host.id.clone());
            }
            Action::None
        }
        KeyCode::Char('e' | 'E') if m.intent == RemoteIntent::Manage => {
            if let Some(host) = m.selected_host() {
                Action::RemoteEditHost(host.id.clone())
            } else {
                Action::None
            }
        }
        KeyCode::Char('i') if m.intent == RemoteIntent::Manage => Action::RemoteImportSshConfig,
        KeyCode::Char(c) => {
            // Type to filter.
            m.query.push(c);
            m.refilter();
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_session_hub(m: &mut RemoteState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            m.sessions.clear();
            m.session_selected = 0;
            m.connection_state = None;
            m.password_buf.clear();
            m.view = RemoteView::Browse;
            Action::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
            m.session_selected = m.session_selected.saturating_sub(1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if m.session_selected + 1 < m.sessions.len() {
                m.session_selected += 1;
            }
            Action::None
        }
        KeyCode::Enter => match (
            m.selected_host_id.as_ref(),
            m.sessions.get(m.session_selected),
        ) {
            (Some(host_id), Some(session)) => Action::RemoteConnectSession {
                host_id: host_id.clone(),
                session_id: session.session_id.clone(),
            },
            _ => Action::None,
        },
        _ => Action::None,
    }
}

/// Handle keys in the host editor (create/edit form).
///
/// Two modes:
/// - **Navigation mode** (not editing a field): Up/Down/Tab navigate, Enter starts
///   editing, `s` saves, Esc goes back.
/// - **Edit mode** (typing into a field): Char/Backspace modify, Enter confirms the
///   field, Esc cancels editing without clearing.
fn handle_editor(m: &mut RemoteState, key: KeyEvent) -> Action {
    if m.editing_field {
        // --- Edit mode: typing into a focused field ---
        match key.code {
            KeyCode::Esc => {
                // Cancel editing this field (don't clear it, just stop editing).
                m.editing_field = false;
                Action::None
            }
            KeyCode::Enter => {
                // Confirm the field, move to next.
                let next = m
                    .editor
                    .as_ref()
                    .map(|e| e.focused.next())
                    .unwrap_or(HostEditField::Name);
                if let Some(editor) = &mut m.editor {
                    editor.focused = next;
                }
                m.editing_field = false;
                Action::None
            }
            KeyCode::Backspace => {
                let focused = m.editor.as_ref().map(|e| e.focused);
                if let (Some(field), Some(editor)) = (focused, m.editor.as_mut()) {
                    match field {
                        HostEditField::Name => editor.name.pop(),
                        HostEditField::User => editor.user.pop(),
                        HostEditField::Host => editor.host.pop(),
                        HostEditField::Port => editor.port.pop(),
                        HostEditField::KeyPath => editor.key_path.pop(),
                    };
                }
                Action::None
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+key in edit mode: ignore (don't capture Ctrl+S etc.)
                    return Action::None;
                }
                let focused = m.editor.as_ref().map(|e| e.focused);
                if let (Some(field), Some(editor)) = (focused, m.editor.as_mut()) {
                    match field {
                        HostEditField::Name => editor.name.push(c),
                        HostEditField::User => editor.user.push(c),
                        HostEditField::Host => editor.host.push(c),
                        HostEditField::Port => editor.port.push(c),
                        HostEditField::KeyPath => editor.key_path.push(c),
                    };
                }
                Action::None
            }
            _ => Action::None,
        }
    } else {
        // --- Navigation mode: moving between fields ---
        match key.code {
            KeyCode::Esc => {
                // Go back to browse.
                m.cancel_edit();
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = m
                    .editor
                    .as_ref()
                    .map(|e| e.focused.prev())
                    .unwrap_or(HostEditField::Name);
                if let Some(editor) = &mut m.editor {
                    editor.focused = prev;
                }
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let next = m
                    .editor
                    .as_ref()
                    .map(|e| e.focused.next())
                    .unwrap_or(HostEditField::Name);
                if let Some(editor) = &mut m.editor {
                    editor.focused = next;
                }
                Action::None
            }
            KeyCode::Tab => {
                let next = m
                    .editor
                    .as_ref()
                    .map(|e| e.focused.next())
                    .unwrap_or(HostEditField::Name);
                if let Some(editor) = &mut m.editor {
                    editor.focused = next;
                }
                Action::None
            }
            KeyCode::BackTab => {
                let prev = m
                    .editor
                    .as_ref()
                    .map(|e| e.focused.prev())
                    .unwrap_or(HostEditField::Name);
                if let Some(editor) = &mut m.editor {
                    editor.focused = prev;
                }
                Action::None
            }
            KeyCode::Enter => {
                // Start editing the focused field.
                m.editing_field = true;
                Action::None
            }
            KeyCode::Char('s') | KeyCode::Char('S') if !is_ctrl(&key, 's') => {
                // Save (validate + commit).
                Action::RemoteSaveHost
            }
            _ => Action::None,
        }
    }
}

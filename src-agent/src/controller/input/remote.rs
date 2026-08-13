//! Keyboard input handler for the `/remote` host manager.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::remote::{ConnectionState, HostEditField, RemoteState, RemoteSub};
use crate::app::state::AppStateRest;
use crate::controller::input::is_ctrl;
use crate::controller::input::Action;

pub fn handle_remote(m: &mut RemoteState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match m.sub {
        RemoteSub::Compact => handle_compact(m, rest, key),
        RemoteSub::Fullscreen => handle_fullscreen(m, rest, key),
        RemoteSub::CreateHost | RemoteSub::EditHost => handle_editor(m, key),
    }
}

fn handle_compact(m: &mut RemoteState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
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
            m.enter_fullscreen();
            Action::None
        }
        KeyCode::Backspace => {
            // Arm delete (second Backspace confirms).
            if let Some(id) = m.pending_delete.take() {
                Action::RemoteDeleteHost(id)
            } else if let Some(host) = m.selected_host() {
                m.pending_delete = Some(host.id.clone());
                Action::None
            } else {
                Action::None
            }
        }
        KeyCode::Char(c) => {
            if is_ctrl(&key, 'a') {
                Action::RemoteAddHost
            } else if c == 'i' {
                Action::RemoteImportSshConfig
            } else {
                // Type to filter.
                m.query.push(c);
                m.refilter();
                Action::None
            }
        }
        _ => Action::None,
    }
}

fn handle_fullscreen(m: &mut RemoteState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // If we're in a transient connection state, route keys there instead.
    match &m.connection_state {
        Some(ConnectionState::AuthRequired { .. }) => {
            return handle_password_state(m, key);
        }
        Some(
            ConnectionState::Resolving
            | ConnectionState::Authenticating
            | ConnectionState::Bootstrapping
            | ConnectionState::Connecting,
        ) => {
            return handle_connecting_state(m, key);
        }
        Some(ConnectionState::Error { .. }) => {
            return handle_error_state(m, key);
        }
        Some(ConnectionState::Connected { .. }) | Some(ConnectionState::Disconnected) | None => {
            // Fall through to normal fullscreen handling.
        }
    }

    match key.code {
        KeyCode::Esc => {
            // Back to compact.
            m.sub = RemoteSub::Compact;
            m.detail_host = None;
            Action::None
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            // Connect to the selected host.
            if let Some(host) = m.selected_host() {
                Action::RemoteConnect(host.id.clone())
            } else {
                Action::None
            }
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            // Edit host.
            if let Some(host) = m.selected_host() {
                Action::RemoteEditHost(host.id.clone())
            } else {
                Action::None
            }
        }
        KeyCode::Backspace => {
            if let Some(id) = m.pending_delete.take() {
                Action::RemoteDeleteHost(id)
            } else if let Some(host) = m.selected_host() {
                m.pending_delete = Some(host.id.clone());
                Action::None
            } else {
                Action::None
            }
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
        _ => Action::None,
    }
}

/// Handle keys while the connection is in a transient connecting/resolving state.
fn handle_connecting_state(m: &mut RemoteState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Cancel connection.
            m.connection_state = None;
            Action::None
        }
        _ => Action::None,
    }
}

/// Handle keys while waiting for a password.
fn handle_password_state(m: &mut RemoteState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Cancel password entry.
            m.password_buf.clear();
            m.connection_state = None;
            Action::None
        }
        KeyCode::Enter => {
            let pw = std::mem::take(&mut m.password_buf);
            Action::RemotePasswordSubmit(pw)
        }
        KeyCode::Backspace => {
            m.password_buf.pop();
            Action::None
        }
        KeyCode::Char(c) => {
            if !is_ctrl(&key, 'c') && !is_ctrl(&key, 'd') {
                m.password_buf.push(c);
            }
            Action::None
        }
        _ => Action::None,
    }
}

/// Handle keys while showing an error state.
fn handle_error_state(m: &mut RemoteState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Dismiss error.
            m.connection_state = None;
            Action::None
        }
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
                // Go back to fullscreen.
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

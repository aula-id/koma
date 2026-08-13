//! Keyboard input handler for the `/remote` host manager.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::remote::{RemoteState, RemoteSub};
use crate::app::state::AppStateRest;
use crate::controller::input::is_ctrl;
use crate::controller::input::Action;

pub fn handle_remote(m: &mut RemoteState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match m.sub {
        RemoteSub::Compact => handle_compact(m, rest, key),
        RemoteSub::Fullscreen => handle_fullscreen(m, rest, key),
        RemoteSub::Connecting => handle_connecting(m, key),
        RemoteSub::PasswordInput => handle_password(m, key),
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
            if m.pending_delete.is_some() {
                let id = m.pending_delete.take().unwrap();
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
            if m.pending_delete.is_some() {
                let id = m.pending_delete.take().unwrap();
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

fn handle_connecting(m: &mut RemoteState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Cancel connection — back to fullscreen.
            m.sub = RemoteSub::Fullscreen;
            m.connection_status = None;
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_password(m: &mut RemoteState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Cancel password entry.
            m.sub = RemoteSub::Connecting;
            m.password_buf.clear();
            m.connection_status = None;
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

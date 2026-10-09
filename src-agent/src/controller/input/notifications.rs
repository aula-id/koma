use super::Action;
use crate::{app::mode::notifications::NotificationsState, model::notifications::Operation};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub fn handle(s: &mut NotificationsState, key: KeyEvent) -> Action {
    if s.confirm_clear {
        match key.code {
            KeyCode::Char('y') => {
                s.confirm_clear = false;
                s.request(Operation::Clear);
            }
            _ => s.confirm_clear = false,
        };
        return Action::None;
    }
    match key.code {
        KeyCode::Esc => return Action::CloseHelp,
        KeyCode::Tab => {
            s.app = !s.app;
            s.cursor = 0;
            s.expanded = None;
            s.entries.clear();
            s.request(Operation::List);
        }
        KeyCode::Up => s.cursor = s.cursor.saturating_sub(1),
        KeyCode::Down => s.cursor = (s.cursor + 1).min(s.filtered().len().saturating_sub(1)),
        KeyCode::Enter => {
            if let Some(e) = s.filtered().get(s.cursor) {
                let id = e.id.clone();
                s.expanded = Some(id.clone());
                s.request(Operation::Read { id: Some(id) });
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.request(Operation::Read { id: None })
        }
        KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.confirm_clear = true
        }
        KeyCode::Backspace => {
            s.query.pop();
            s.cursor = 0;
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.query.push(c);
            s.cursor = 0;
        }
        _ => {}
    }
    Action::None
}

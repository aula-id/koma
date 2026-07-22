//! Key handler for the first-run connection chooser (`Mode::Onboard`).
//!
//! Up/`k` and Down/`j` move the highlight (clamped to the three rows); Enter routes
//! the selected row to its setup action; `q`/Esc quit (this is the very first
//! screen — there is no Chat to fall back to, mirroring the KeyInput wizard's
//! `first_run` Esc).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::Action;
use crate::app::mode::OnboardState;
use crate::app::state::AppStateRest;

/// Handle a key press while the first-run connection chooser is active.
pub fn handle_onboard(state: &mut OnboardState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.up();
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.down();
            Action::None
        }
        KeyCode::Enter => match state.cursor {
            0 => Action::SetupKomaFree,
            1 => Action::OnboardProvider,
            // Any other row (2) is the custom endpoint + key wizard.
            _ => Action::OnboardCustom,
        },
        // First-run: no Chat to return to, so both quit (mirrors KeyInput first_run).
        KeyCode::Char('q') if !key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Esc => Action::Quit,
        _ => Action::None,
    }
}

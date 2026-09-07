//! Key handler for the `/skill` hub overlay (`Mode::Skill`).
//!
//! One coherent control stream (no multi-focus mode):
//!
//! | Key | Action |
//! |---|---|
//! | printable / paste | push query, refilter (sticky name) |
//! | Backspace | pop query, refilter |
//! | ↑ / ↓ | move selection |
//! | Tab / → | cycle filter chip right: all → active → inactive |
//! | ← | cycle filter chip left |
//! | Enter / Space | toggle load on selected skill row |
//! | Esc | close → Chat |
//! | other | None (stay open) |
//!
//! Filter chips are a **radio group** — exactly one is checked; ←/→ moves the
//! `[x]` across `all | active | inactive`. Space does **not** insert into the
//! query. Esc-only close is intentional — do not copy bash/todo stray-dismiss.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::SkillCmdState;
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key press inside the `/skill` hub overlay.
pub fn handle_skill_cmd(st: &mut SkillCmdState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::CloseSkill,

        KeyCode::Up => {
            st.move_up();
            Action::None
        }
        KeyCode::Down => {
            st.move_down();
            Action::None
        }

        // Enter / Space toggle the selected skill (load ↔ unload).
        KeyCode::Enter | KeyCode::Char(' ') => match st.selected_name() {
            Some(name) => Action::SkillToggle(name.to_string()),
            None => Action::None,
        },

        // Tab / Right: radio moves right (all → active → inactive → all).
        KeyCode::Tab | KeyCode::Right => {
            st.chip_next();
            Action::None
        }
        // Left: radio moves left.
        KeyCode::Left => {
            st.chip_prev();
            Action::None
        }

        KeyCode::Backspace => {
            st.query.pop();
            st.refilter();
            Action::None
        }

        // Remaining printables (Space already handled) feed the omnisearch.
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            st.query.push(c);
            st.refilter();
            Action::None
        }

        _ => Action::None,
    }
}

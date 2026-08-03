//! Key handler for the `/skill` hub overlay (`Mode::Skill`).
//!
//! An omnisearch filter + chip-select surface:
//!
//! - Printable char → push to `query`, refilter.
//! - Backspace → pop from `query`, refilter.
//! - Up/Down → move the selection over the filtered list.
//! - Enter → toggle the selected skill (load ↔ unload).
//! - Tab/Left/Right → cycle the filter chip (all ↔ active).
//! - Esc → close back to Chat.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::{SkillCmdState, SkillFilterChip};
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key press inside the `/skill` hub overlay.
pub fn handle_skill_cmd(
    st: &mut SkillCmdState,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
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

        KeyCode::Enter => match st.selected_name() {
            Some(name) => Action::SkillToggle(name.to_string()),
            None => Action::None,
        },

        // Tab or Left/Right cycles chip
        KeyCode::Tab | KeyCode::Right => {
            st.chip = match st.chip {
                SkillFilterChip::All => SkillFilterChip::Active,
                SkillFilterChip::Active => SkillFilterChip::All,
            };
            st.refilter();
            Action::None
        }
        KeyCode::Left => {
            st.chip = match st.chip {
                SkillFilterChip::All => SkillFilterChip::Active,
                SkillFilterChip::Active => SkillFilterChip::All,
            };
            st.refilter();
            Action::None
        }

        KeyCode::Backspace => {
            st.query.pop();
            st.refilter();
            Action::None
        }

        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            st.query.push(c);
            st.refilter();
            Action::None
        }

        _ => Action::None,
    }
}

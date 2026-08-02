//! Controller – key handler for the `/model` command overlay.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::{ModelCmdState, ModelCmdSub};
use crate::controller::input::action::Action;

/// Handle a key press inside the `/model` command overlay.
///
/// Up/Down move the selection; Enter confirms the highlighted option (meaning
/// depends on submode); Esc cancels. `_rest` is accepted for handler-signature
/// consistency but unused here.
pub(crate) fn handle_model_cmd(
    m: &mut ModelCmdState,
    _rest: &mut crate::app::state::AppStateRest,
    key: KeyEvent,
) -> Action {
    match key.code {
        KeyCode::Esc => match &m.sub {
            ModelCmdSub::AgentPick { .. } => {
                // Esc in AgentPick → back to AgentList
                Action::ModelBackToAgentList
            }
            _ => Action::ModelCancel,
        },
        KeyCode::Up => {
            m.up();
            Action::None
        }
        KeyCode::Down => {
            m.down();
            Action::None
        }
        KeyCode::Enter => match &m.sub {
            ModelCmdSub::Help { .. } => Action::ModelCancel,
            ModelCmdSub::RolePick { role } => {
                let uuid = m.selected_uuid();
                Action::ModelRoleSwap {
                    role: *role,
                    model_uuid: uuid,
                }
            }
            ModelCmdSub::AgentList => {
                // Enter on agent list → open AgentPick for that agent
                if let Some((Some(name), _)) = m.selected() {
                    Action::ModelOpenAgentPick {
                        agent_name: name.clone(),
                    }
                } else {
                    Action::None
                }
            }
            ModelCmdSub::AgentPick { agent_name, .. } => {
                let uuid = m.selected_uuid();
                Action::ModelAgentSwap {
                    agent_name: agent_name.clone(),
                    model_uuid: uuid,
                }
            }
        },
        _ => Action::None,
    }
}

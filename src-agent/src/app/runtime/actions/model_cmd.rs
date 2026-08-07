//! Action handlers for the `/model` command: role swap, agent swap, cancel.

use anyhow::Result;

use crate::app::mode::{ModelCmdState, ModelCmdSub};
use crate::app::state::AppState;
use crate::model::app_config::ModelRole;

/// Handle `Action::ModelRoleSwap`: swap the session's role to the chosen model
/// (or inherit / drop override when `model_uuid` is `None`).
pub(crate) fn handle_model_role_swap(
    role: ModelRole,
    model_uuid: Option<String>,
    state: &mut AppState,
) -> Result<()> {
    let before_main = state.rest.main_identity_now();

    // Snapshot the chosen entry from global catalogue BEFORE the mutable borrow.
    let chosen = model_uuid.as_ref().and_then(|uuid| {
        state
            .rest
            .config
            .models
            .iter()
            .find(|m| &m.uuid == uuid)
            .cloned()
    });

    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        if model_uuid.is_none() {
            // Inherit: drop any local override for this role.
            sess.settings
                .session_models
                .retain(|e| !e.effective_roles().contains(&role));
            sess.save()?;
        } else if let Some(chosen) = chosen {
            // Check if already a local override with the same source_uuid.
            let already = sess.settings.session_models.iter().any(|e| {
                e.effective_roles().contains(&role)
                    && e.source_uuid.as_deref() == Some(chosen.uuid.as_str())
            });

            if !already {
                use crate::model::app_config::{new_uuid, ModelEntry};

                // Strip this role from all other entries.
                sess.settings
                    .session_models
                    .retain(|e| !e.effective_roles().contains(&role));

                // Push cloned global entry as the new local override.
                sess.settings.session_models.push(ModelEntry {
                    uuid: new_uuid(),
                    name: chosen.name.clone(),
                    model_id: chosen.model_id.clone(),
                    provider_uuid: chosen.provider_uuid.clone(),
                    route: chosen.route.clone(),
                    roles: vec![role],
                    role: None,
                    source_uuid: Some(chosen.uuid.clone()),
                });
            }
            sess.save()?;
        }
        // Unknown uuid → no-op (leave overrides as-is).
    }

    // Reset effort if Main was the role that changed.
    state.rest.reset_effort_if_main_changed(before_main);

    // Show confirmation toast.
    let role_label = match role {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Planner => "planner",
        ModelRole::Compactor => "compactor",
        ModelRole::Safeguard => "safeguard",
    };
    // Resolve the model label for the toast before any mutable borrows.
    let toast_msg = if model_uuid.is_none() {
        format!("{role_label}: inherited")
    } else {
        let label = model_uuid
            .as_ref()
            .and_then(|u| {
                state
                    .rest
                    .config
                    .models
                    .iter()
                    .find(|m| &m.uuid == u)
                    .map(|m| m.model_id.as_str())
            })
            .unwrap_or("set");
        format!("{role_label}: {label}")
    };
    state.rest.fg_mut().set_toast_info(toast_msg);

    *state.mode_mut() = crate::app::mode::Mode::Chat;
    Ok(())
}

/// Handle `Action::ModelAgentSwap`: set the agent's model_uuid.
pub(crate) fn handle_model_agent_swap(
    agent_name: String,
    model_uuid: Option<String>,
    state: &mut AppState,
) -> Result<()> {
    use crate::model::agent_def::{save_agent, AgentScope as DefScope, AgentSource};

    let session_path = state.rest.fg().session.as_ref().map(|s| s.path.clone());

    let registry =
        crate::model::agent_def::load_registry(session_path.as_deref().and_then(|p| p.parent()));
    let Some(agent) = registry.get(&agent_name).cloned() else {
        state.rest.fg_mut().status = format!("unknown agent: {agent_name}");
        return Ok(());
    };

    // Refuse extension agents.
    if agent.source == AgentSource::Extension {
        state.rest.fg_mut().status = "cannot change extension agent model".to_string();
        return Ok(());
    }

    let mut def = agent.clone();
    def.model_uuid = model_uuid.clone();

    let scope = match agent.source {
        AgentSource::Global => DefScope::Global,
        AgentSource::Session | AgentSource::Builtin | AgentSource::Extension => {
            let dir = session_path
                .as_deref()
                .and_then(|p| p.parent())
                .unwrap_or_else(|| std::path::Path::new("."));
            DefScope::Session(dir)
        }
    };

    if let Err(e) = save_agent(scope, &def) {
        state.rest.fg_mut().status = format!("save failed: {e}");
        return Ok(());
    }

    // Rebuild system prompt so the sub-agent roster reflects the change.
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        sess.rebuild_system();
    }

    // Toast — resolve the label before the mutable borrow.
    let toast_msg = {
        let label = model_uuid
            .as_ref()
            .and_then(|u| {
                state
                    .rest
                    .config
                    .models
                    .iter()
                    .find(|m| &m.uuid == u)
                    .map(|m| m.model_id.as_str())
            })
            .unwrap_or("inherited");
        format!("{agent_name}: {label}")
    };
    state.rest.fg_mut().set_toast_info(toast_msg);

    *state.mode_mut() = crate::app::mode::Mode::Chat;
    Ok(())
}

/// Handle `Action::ModelBackToAgentList`: pop AgentPick back to AgentList.
pub(crate) fn handle_model_back_to_agent_list(state: &mut AppState) {
    let session_path = state.rest.fg().session.as_ref().map(|s| s.path.clone());

    let registry =
        crate::model::agent_def::load_registry(session_path.as_deref().and_then(|p| p.parent()));
    let agents: Vec<String> = registry
        .list(true)
        .into_iter()
        .filter(|a| !matches!(a.source, crate::model::agent_def::AgentSource::Extension))
        .map(|a| a.name.clone())
        .collect();
    let options: Vec<(Option<String>, String)> = agents
        .into_iter()
        .map(|name| (Some(name.clone()), name))
        .collect();
    *state.mode_mut() = crate::app::mode::Mode::Model(Box::new(ModelCmdState {
        sub: ModelCmdSub::AgentList,
        options,
        cursor: 0,
        note: String::new(),
    }));
}

/// Handle `Action::ModelOpenAgentPick`: open AgentPick for a named agent.
pub(crate) fn handle_model_open_agent_pick(agent_name: String, state: &mut AppState) {
    let session_path = state.rest.fg().session.as_ref().map(|s| s.path.clone());

    let registry =
        crate::model::agent_def::load_registry(session_path.as_deref().and_then(|p| p.parent()));

    let Some(agent) = registry.get(&agent_name).cloned() else {
        state.rest.fg_mut().status = format!("unknown agent: {agent_name}");
        return;
    };

    // Build model options: inherit main + session models + global models.
    let mut options: Vec<(Option<String>, String)> = vec![(None, "(inherit main)".to_string())];

    let sess_settings = state.rest.fg().session.as_ref().map(|s| &s.settings);
    let config = &state.rest.config;
    if let Some(settings) = sess_settings {
        for entry in settings.session_models.iter().chain(config.models.iter()) {
            let label = entry_label(config, entry);
            options.push((Some(entry.uuid.clone()), label));
        }
    }

    // Cursor on the agent's current model_uuid or 0 (inherit).
    let cursor = match &agent.model_uuid {
        Some(uuid) => options
            .iter()
            .position(|(u, _)| u.as_deref() == Some(uuid.as_str()))
            .unwrap_or(0),
        None => 0,
    };

    *state.mode_mut() = crate::app::mode::Mode::Model(Box::new(ModelCmdState {
        sub: ModelCmdSub::AgentPick {
            agent_name,
            current_model: agent.model_uuid,
        },
        options,
        cursor,
        note: "pick a model for this agent".to_string(),
    }));
}

/// One-line label for a model entry: `"name — model_id @ provider"`.
fn entry_label(
    config: &crate::model::app_config::AppConfig,
    entry: &crate::model::app_config::ModelEntry,
) -> String {
    let provider_name = config
        .providers
        .iter()
        .find(|p| p.uuid == entry.provider_uuid)
        .and_then(|p| {
            if !p.name.trim().is_empty() {
                Some(p.name.as_str())
            } else if !p.endpoint.trim().is_empty() {
                Some(p.endpoint.as_str())
            } else {
                None
            }
        })
        .or_else(|| {
            config
                .oauth_conns
                .iter()
                .find(|c| c.uuid == entry.provider_uuid)
                .and_then(|c| {
                    if !c.name.trim().is_empty() {
                        Some(c.name.as_str())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or("?");
    format!("{} — {} @ {}", entry.name, entry.model_id, provider_name)
}

#[cfg(test)]
#[path = "model_cmd_action_test.rs"]
mod tests;

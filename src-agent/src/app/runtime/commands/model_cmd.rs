//! Model command: `/model` — session model switcher + agent model picker.
//!
//! Subcommand dispatch:
//! - `/model` or `/model help` or `/model ?` → open help submode
//! - `/model <role>` → open RolePick picker for that role
//! - `/model agent` → open AgentList picker
//! - `/model agent <name>` → open AgentPick for named agent
//!
//! Role tokens (case-insensitive): `main`, `awareness`, `planner`, `compactor`,
//! `safeguard`.

use anyhow::Result;

use crate::app::mode::{ModelCmdState, ModelCmdSub};
use crate::app::state::AppState;
use crate::model::app_config::ModelRole;

/// Parse a role token (case-insensitive) into a [`ModelRole`].
pub(crate) fn parse_role(s: &str) -> Option<ModelRole> {
    match s.to_lowercase().as_str() {
        "main" => Some(ModelRole::Main),
        "awareness" => Some(ModelRole::Awareness),
        "planner" => Some(ModelRole::Planner),
        "compactor" => Some(ModelRole::Compactor),
        "safeguard" => Some(ModelRole::Safeguard),
        _ => None,
    }
}

/// Handle the `/model` command. Parses the remainder and dispatches.
pub(super) fn handle_model(args: String, state: &mut AppState) -> Result<()> {
    let rest = args.trim().to_string();

    if rest.is_empty() || rest.eq_ignore_ascii_case("help") || rest == "?" {
        open_help(state);
        return Ok(());
    }

    let first_token = rest.split_whitespace().next().unwrap_or("");

    // Role tokens
    if let Some(role) = parse_role(first_token) {
        open_role_pick(role, state);
        return Ok(());
    }

    // Agent subcommand
    if first_token.eq_ignore_ascii_case("agent") {
        let agent_name = rest[first_token.len()..].trim();
        if agent_name.is_empty() {
            open_agent_list(state);
        } else {
            open_agent_pick_by_name(agent_name, state);
        }
        return Ok(());
    }

    // Unknown subcommand — open help with error note
    let mut lines = help_lines(state);
    lines.insert(0, String::new());
    lines.insert(0, format!("unknown subcommand: {first_token}"));
    *state.mode_mut() = crate::app::mode::Mode::Model(Box::new(ModelCmdState {
        sub: ModelCmdSub::Help { lines },
        options: Vec::new(),
        cursor: 0,
        note: String::new(),
    }));
    Ok(())
}

/// Open the Help submode with live role bindings.
fn open_help(state: &mut AppState) {
    let lines = help_lines(state);
    *state.mode_mut() = crate::app::mode::Mode::Model(Box::new(ModelCmdState {
        sub: ModelCmdSub::Help { lines },
        options: Vec::new(),
        cursor: 0,
        note: String::new(),
    }));
}

/// Build the help text lines with current role bindings.
fn help_lines(state: &AppState) -> Vec<String> {
    let mut lines = vec![
        "/model — session model switcher".to_string(),
        String::new(),
    ];

    if let Some(sess) = state.rest.fg().session.as_ref() {
        let cfg = &state.rest.config;
        let settings = &sess.settings;
        let roles = [
            (ModelRole::Main, "main"),
            (ModelRole::Awareness, "awareness"),
            (ModelRole::Planner, "planner"),
            (ModelRole::Compactor, "compactor"),
            (ModelRole::Safeguard, "safeguard"),
        ];
        for (role, label) in roles {
            let resolved = crate::app::resolve::resolve_role(cfg, settings, role);
            let model_label = resolved
                .as_ref()
                .map(|r| r.model_id.as_str())
                .unwrap_or("(unset)");
            lines.push(format!("  {label:<12} {model_label}"));
        }
    } else {
        lines.push("  no active session".to_string());
    }

    lines.push(String::new());
    lines.push("  /model <role>            swap role model".to_string());
    lines.push("  /model agent             pick agent, then model".to_string());
    lines.push("  /model agent <name>      swap model for agent".to_string());

    // List available agents.
    if let Some(sess) = state.rest.fg().session.as_ref() {
        let session_dir = sess.path.parent();
        let registry = crate::model::agent_def::load_registry(session_dir);
        let agent_names: Vec<String> = registry
            .list(true)
            .into_iter()
            .filter(|a| {
                !matches!(
                    a.source,
                    crate::model::agent_def::AgentSource::Extension
                )
            })
            .map(|a| a.name.clone())
            .collect();
        if !agent_names.is_empty() {
            lines.push(String::new());
            lines.push(format!("  agents: {}", agent_names.join(", ")));
        }
    }

    lines
}

/// Open the RolePick picker for a session role.
fn open_role_pick(role: ModelRole, state: &mut AppState) {
    let mut options: Vec<(Option<String>, String)> = Vec::new();

    // Row 0: inherit (drop the local override for this role).
    options.push((None, "(inherit global)".to_string()));

    // List models from the GLOBAL catalogue only.
    if let Some(sess) = state.rest.fg().session.as_ref() {
        let config = &state.rest.config;
        for entry in &config.models {
            let label = entry_label(config, entry);
            options.push((Some(entry.uuid.clone()), label));
        }

        // If role is Main, add the synthetic koma-free row.
        if role == ModelRole::Main {
            use crate::service::koma_free::KOMA_FREE_SENTINEL;
            options.insert(
                1,
                (
                    Some(KOMA_FREE_SENTINEL.to_string()),
                    "(koma free — keyless)".to_string(),
                ),
            );
        }

        // Cursor: find the current local override's source_uuid in the global
        // catalogue, or 0 (inherit).
        let current_source = sess
            .settings
            .session_models
            .iter()
            .find(|e| e.effective_roles().contains(&role))
            .and_then(|e| e.source_uuid.clone());

        let cursor = match &current_source {
            Some(uuid) => options
                .iter()
                .position(|(u, _)| u.as_deref() == Some(uuid.as_str()))
                .unwrap_or(0),
            None => 0,
        };

        let role_label = match role {
            ModelRole::Main => "main",
            ModelRole::Awareness => "awareness",
            ModelRole::Planner => "planner",
            ModelRole::Compactor => "compactor",
            ModelRole::Safeguard => "safeguard",
        };

        *state.mode_mut() = crate::app::mode::Mode::Model(Box::new(ModelCmdState {
            sub: ModelCmdSub::RolePick { role },
            options,
            cursor,
            note: format!("pick a model for the {role_label} role"),
        }));
    }
}

/// Open the AgentList picker.
fn open_agent_list(state: &mut AppState) {
    let session_path = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.path.clone());

    let registry = crate::model::agent_def::load_registry(
        session_path.as_deref().and_then(|p| p.parent()),
    );

    let config = &state.rest.config;

    let agents: Vec<String> = registry
        .list(true)
        .into_iter()
        .filter(|a| {
            !matches!(
                a.source,
                crate::model::agent_def::AgentSource::Extension
            )
        })
        .map(|a| a.name.clone())
        .collect();

    let options: Vec<(Option<String>, String)> = agents
        .into_iter()
        .map(|name| {
            // Show current model if set.
            let label = match registry.get(&name).and_then(|d| d.model_uuid.as_ref()) {
                Some(uuid) => {
                    let model_label = config
                        .models
                        .iter()
                        .find(|m| &m.uuid == uuid)
                        .map(|m| m.model_id.as_str())
                        .unwrap_or("?");
                    format!("{name} — {model_label}")
                }
                None => name.clone(),
            };
            (Some(name), label)
        })
        .collect();

    *state.mode_mut() = crate::app::mode::Mode::Model(Box::new(ModelCmdState {
        sub: ModelCmdSub::AgentList,
        options,
        cursor: 0,
        note: "select an agent to change its model".to_string(),
    }));
}

/// Open AgentPick for a named agent (from `/model agent <name>`).
fn open_agent_pick_by_name(agent_name: &str, state: &mut AppState) {
    use crate::model::agent_def::AgentSource;

    let session_path = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.path.clone());

    let registry = crate::model::agent_def::load_registry(
        session_path.as_deref().and_then(|p| p.parent()),
    );
    let Some(agent) = registry.get(agent_name).cloned() else {
        state.rest.fg_mut().status = format!("unknown agent: {agent_name}");
        return;
    };

    if agent.source == AgentSource::Extension {
        state.rest.fg_mut().status =
            "cannot change extension agent model".to_string();
        return;
    }

    crate::app::runtime::actions::model_cmd::handle_model_open_agent_pick(
        agent_name.to_string(),
        state,
    );
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
#[path = "model_cmd_test.rs"]
mod tests;

//! Action handlers for the agent dashboard: CreateAgent, SaveAgent,
//! DeleteAgent, CloseAgents.

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;

/// Handle `Action::CloseAgents`: discard any in-flight drafts and return to Chat.
pub(super) fn handle_close_agents(state: &mut AppState) -> Result<()> {
    // Discard any in-flight drafts; the dashboard never wrote them.
    state.mode = Mode::Chat;
    state.rest.status = "ready".into();
    Ok(())
}

/// Handle `Action::CreateAgent`: build an [`AgentDef`] from the Create drafts,
/// write it to the chosen scope, then reload the in-mode registry snapshot.
///
/// The name is re-validated by the data layer ([`agent_def::save_agent`]) so a
/// path-traversal name can never reach the filesystem. On success the dashboard
/// returns to Browse with the new agent in the list; on error the status line
/// reports it and the editor stays open so the draft isn't lost.
pub(super) fn handle_create_agent(state: &mut AppState) -> Result<()> {
    use crate::model::agent_def::{save_agent, AgentScope as DefScope};

    let Mode::Agents(a) = &state.mode else {
        return Ok(());
    };
    let scope_session = matches!(a.create_scope, crate::app::mode::AgentScope::Session);
    let def = a.to_agent_def();
    let session_dir = a.session_dir.clone();

    let scope = if scope_session {
        DefScope::Session(&session_dir)
    } else {
        DefScope::Global
    };
    let result = save_agent(scope, &def);

    match result {
        Ok(_) => {
            // Reload from disk so the new agent appears with its real source.
            if let Some(sess) = state.rest.fg().session.as_ref() {
                if let Mode::Agents(a) = &mut state.mode {
                    a.reload(sess);
                    // Select the freshly-created agent if we can find it.
                    if let Some(i) = a.agents.iter().position(|x| x.name == def.name) {
                        a.list_sel = i;
                    }
                    a.cancel();
                }
            }
            // Rebuild the system prompt so the sub-agent roster reflects the new agent.
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
            }
            state.rest.status = format!("agent created: {}", def.name);
        }
        Err(e) => {
            state.rest.status = format!("create failed: {e}");
        }
    }
    Ok(())
}

/// Handle `Action::SaveAgent`: overwrite the selected file-backed agent with the
/// Edit drafts, writing to the agent's own scope, then reload.
///
/// The target scope is the selected agent's [`AgentSource`] — a session agent is
/// re-saved into the session dir, a global agent into the global dir. A built-in
/// can never reach this path (the input handler blocks Edit on built-ins), so an
/// unexpected built-in source is treated as a no-op error.
pub(super) fn handle_save_agent(state: &mut AppState) -> Result<()> {
    use crate::model::agent_def::{save_agent, AgentScope as DefScope, AgentSource};

    let Mode::Agents(a) = &state.mode else {
        return Ok(());
    };
    let Some(agent) = a.current_agent() else {
        return Ok(());
    };
    let source = agent.source;
    let def = a.to_agent_def();
    let session_dir = a.session_dir.clone();

    let scope = match source {
        AgentSource::Global => DefScope::Global,
        AgentSource::Session => DefScope::Session(&session_dir),
        AgentSource::Builtin => {
            // Defensive: the UI never offers Edit on a built-in.
            state.rest.status = "built-in agents are read-only".into();
            return Ok(());
        }
    };
    let result = save_agent(scope, &def);

    match result {
        Ok(_) => {
            if let Some(sess) = state.rest.fg().session.as_ref() {
                if let Mode::Agents(a) = &mut state.mode {
                    a.reload(sess);
                    if let Some(i) = a.agents.iter().position(|x| x.name == def.name) {
                        a.list_sel = i;
                    }
                    a.cancel();
                }
            }
            // Rebuild the system prompt so the sub-agent roster reflects the change.
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
            }
            state.rest.status = format!("agent updated: {}", def.name);
        }
        Err(e) => {
            state.rest.status = format!("save failed: {e}");
        }
    }
    Ok(())
}

/// Handle `Action::DeleteAgent`: remove the selected file-backed agent from its
/// own scope's directory, then reload.
///
/// Built-ins are never deletable: they have no `file_path` and the input handler
/// blocks the delete prompt for them, so this only ever sees Global/Session
/// file agents. Deleting a session/global override that shadowed a built-in
/// simply re-exposes the built-in on the next reload.
pub(super) fn handle_delete_agent(state: &mut AppState) -> Result<()> {
    use crate::model::agent_def::{delete_agent, AgentScope as DefScope, AgentSource};

    let Mode::Agents(a) = &state.mode else {
        return Ok(());
    };
    let Some(agent) = a.current_agent() else {
        return Ok(());
    };
    let name = agent.name.clone();
    let source = agent.source;
    let session_dir = a.session_dir.clone();

    let scope = match source {
        AgentSource::Global => DefScope::Global,
        AgentSource::Session => DefScope::Session(&session_dir),
        AgentSource::Builtin => {
            state.rest.status = "cannot delete a built-in agent".into();
            if let Mode::Agents(a) = &mut state.mode {
                a.cancel();
            }
            return Ok(());
        }
    };
    let result = delete_agent(scope, &name);

    match result {
        Ok(()) => {
            if let Some(sess) = state.rest.fg().session.as_ref() {
                if let Mode::Agents(a) = &mut state.mode {
                    a.reload(sess);
                    a.cancel();
                }
            }
            // Rebuild the system prompt so the sub-agent roster reflects the deletion.
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
            }
            state.rest.status = format!("agent deleted: {name}");
        }
        Err(e) => {
            state.rest.status = format!("delete failed: {e}");
            if let Mode::Agents(a) = &mut state.mode {
                a.cancel();
            }
        }
    }
    Ok(())
}

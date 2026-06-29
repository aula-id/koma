use super::modes::mode_snapshot;
use super::tokens::theme_token;

use crate::app::resolve::resolve_role;
use crate::app::state::AppState;
use crate::app::subagent::SubAgentStatus;
use crate::model::app_config::{AppConfig, ModelRole};

use crate::ipc::proto::{
    GlobalSnapshot, PendingSubagentSnapshot, SessionSnapshot, StateSnapshot, SubAgentSnapshot,
};

/// Build a complete, frozen [`StateSnapshot`] from the live [`AppState`].
pub fn build_snapshot(state: &AppState) -> StateSnapshot {
    let config = &state.rest.config;
    let sessions: Vec<SessionSnapshot> = state
        .rest
        .sessions
        .iter()
        .map(|rt| session_snapshot(rt, config))
        .collect();

    let foreground_id = state
        .rest
        .sessions
        .get(state.rest.foreground)
        .map(|s| s.id.clone());

    StateSnapshot {
        foreground_id,
        sessions,
        global: global_snapshot(state),
    }
}

pub fn session_snapshot(
    rt: &crate::app::state::SessionRuntime,
    config: &AppConfig,
) -> SessionSnapshot {
    let messages = rt
        .session
        .as_ref()
        .map(|s| s.conversation.messages().to_vec())
        .unwrap_or_default();
    let committed_reasoning: Vec<Option<String>> =
        if messages.iter().any(|m| m.reasoning.is_some()) {
            messages.iter().map(|m| m.reasoning.clone()).collect()
        } else {
            Vec::new()
        };
    let name = rt
        .session
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_default();

    let resolved_model_id = rt
        .session
        .as_ref()
        .and_then(|s| resolve_role(config, &s.settings, ModelRole::Main))
        .map(|r| r.model_id)
        .or_else(|| rt.session.as_ref().map(|s| s.settings.model.clone()))
        .unwrap_or_default();

    SessionSnapshot {
        id: rt.id.clone(),
        name,
        cwd: rt
            .session
            .as_ref()
            .map(|_| rt.effective_cwd().display().to_string())
            .unwrap_or_default(),
        messages,
        committed_reasoning,
        streaming: rt.streaming.clone(),
        stream_reasoning: rt.stream_reasoning.clone(),
        tokens_in: rt.tokens_in,
        tokens_out: rt.tokens_out,
        cost: rt.cost,
        tokens_cached: rt.tokens_cached,
        waiting: rt.waiting,
        awaiting_approval: rt.awaiting_approval,
        approval_reason: rt.approval_reason.clone(),
        pending_tool_calls: rt.pending_tool_calls.clone(),
        tool_idx: rt.tool_idx,
        working: rt.is_working(),
        finished_unseen: rt.finished_unseen,
        subagents: rt.subagents.iter().map(subagent_snapshot).collect(),
        pending_subagents: rt
            .pending_subagents
            .iter()
            .map(pending_subagent_snapshot)
            .collect(),
        resolved_model_id,
    }
}

fn subagent_snapshot(sa: &crate::app::subagent::SubAgent) -> SubAgentSnapshot {
    let status = match &sa.status {
        SubAgentStatus::Running => "running".to_string(),
        SubAgentStatus::Done(_) => "done".to_string(),
        SubAgentStatus::Killed => "killed".to_string(),
        SubAgentStatus::Error(e) => format!("error: {e}"),
    };

    SubAgentSnapshot {
        id: sa.id,
        name: sa.agent_name.clone(),
        label: sa.label.clone(),
        status,
        steps: sa.transcript.len(),
        transcript: sa.transcript.clone(),
        messages: sa.messages.clone(),
    }
}

fn pending_subagent_snapshot(
    p: &crate::app::subagent::PendingSubagent,
) -> PendingSubagentSnapshot {
    PendingSubagentSnapshot {
        id: p.id,
        agent_name: p.agent_name.clone(),
        prompt: p.prompt.clone(),
    }
}

pub fn global_snapshot(state: &AppState) -> GlobalSnapshot {
    GlobalSnapshot {
        input: state.rest.input.clone(),
        cursor: state.rest.cursor,
        scroll: state.rest.scroll,
        follow: state.rest.follow,
        status: state.rest.status.clone(),
        work_elapsed_ms: state
            .rest
            .work_since
            .map(|since| since.elapsed().as_millis() as u64),
        theme: theme_token(&state.rest.config.theme).to_string(),
        accent: state.rest.config.accent.clone(),
        mode: mode_snapshot(state),
        toast: state.rest.toast.as_ref().map(|(msg, _until, kind)| {
            let kind = match kind {
                crate::app::state::ToastKind::Error => "error".to_string(),
                crate::app::state::ToastKind::Info => "info".to_string(),
            };
            (kind, msg.clone())
        }),
        models_cache: state.rest.models_cache.clone(),
        models_cache_endpoint: state.rest.models_cache_endpoint.clone(),
        agent_viewer: state.rest.agent_viewer,
        agent_viewer_scroll: state.rest.agent_viewer_scroll,
        agent_viewer_follow: state.rest.agent_viewer_follow,
        subagents_open: state.rest.subagents_open,
        subagent_sel: state.rest.subagent_sel,
        palette_sel: state.rest.palette_sel,
        pending_attachments: state.rest.pending_attachments.clone(),
        file_palette: file_palette_matches(state),
        agent_mode: match state.rest.agent_mode {
            crate::app::state::AgentMode::Auto => "auto",
            crate::app::state::AgentMode::Normal => "normal",
        }
        .to_string(),
    }
}

const FILE_PAL_MAX: usize = 10;

fn file_palette_matches(state: &AppState) -> Option<Vec<String>> {
    let partial = crate::controller::input::file_ref_partial(&state.rest.input)?;
    let matches = state
        .rest
        .fg()
        .dir_cache
        .read()
        .map(|c| c.search(partial, FILE_PAL_MAX))
        .unwrap_or_default();
    Some(matches)
}

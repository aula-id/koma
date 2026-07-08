use super::modes::mode_snapshot;
use super::tokens::theme_token;

use crate::app::resolve::resolve_role;
use crate::app::state::AppState;
use crate::app::subagent::SubAgentStatus;
use crate::model::app_config::{AppConfig, ModelRole};

use crate::ipc::proto::{
    GlobalSnapshot, ModeSnapshot, PendingSubagentSnapshot, SessionSnapshot, StateSnapshot,
    SubAgentSnapshot,
};

/// Build a complete, frozen [`StateSnapshot`] from the live [`AppState`].
///
/// Builds the mode payload via the pure [`mode_snapshot`] projection, then defers to
/// [`build_snapshot_with_mode`]. The daemon's hot streaming path bypasses this and
/// supplies a CACHED [`ModeSnapshot`] directly (the mode payload is the expensive part
/// for heavy modes like `/usage` / `/agents` / `/mcp`), so this default entry point is
/// used only by the rarer attach/resync paths and tests.
pub fn build_snapshot(state: &AppState) -> StateSnapshot {
    build_snapshot_with_mode(state, mode_snapshot(state))
}

/// Build a [`StateSnapshot`] using a PRE-BUILT [`ModeSnapshot`] instead of projecting
/// the mode fresh. Identical to [`build_snapshot`] except the (expensive) mode payload
/// is supplied by the caller — the daemon streams this with a throttled/discriminant-
/// cached mode so the per-tick rebuild stays cheap. Everything else (sessions +
/// foreground + the rest of [`global_snapshot_with_mode`]) is still projected from live
/// `state`.
pub fn build_snapshot_with_mode(state: &AppState, mode: ModeSnapshot) -> StateSnapshot {
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
        global: global_snapshot_with_mode(state, mode),
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
        working: rt.is_ui_busy(),
        finished_unseen: rt.finished_unseen,
        subagents: rt.subagents.iter().map(subagent_snapshot).collect(),
        pending_subagents: rt
            .pending_subagents
            .iter()
            .map(pending_subagent_snapshot)
            .collect(),
        resolved_model_id,
        pending_steer: rt
            .pending_steer
            .iter()
            .map(|s| {
                let one_line = s.replace('\n', " ");
                let t = one_line.trim().to_string();
                if t.chars().count() > 48 {
                    format!("{}…", t.chars().take(48).collect::<String>())
                } else {
                    t
                }
            })
            .collect(),
        // Background-bash jobs (identity + raw status; no output) so the GUI sidepanel's
        // `bash[]` shows them. Reads the LIVE registry the jobs actually land in — the
        // same `bash_jobs` vec `bash_output`/`bash_kill` mutate — so finished/failed jobs
        // (which are retained, never removed) are included.
        bash_jobs: rt
            .bash_jobs
            .iter()
            .map(|j| crate::ipc::proto::BashJobSnapshot {
                id: j.id,
                command: j.command.clone(),
                status: j.snapshot_status(),
            })
            .collect(),
        // Cumulative file-change log (#24) so the GUI Explore "File changed" panel
        // renders it. Sourced from the in-memory mirror (refreshed at finish_tool_round
        // + on load from the per-session store).
        file_changes: rt
            .file_changes
            .iter()
            .map(|c| crate::ipc::proto::FileChangeSnapshot {
                path: c.path.clone(),
                status: c.status.clone(),
            })
            .collect(),
        // Plan-mode todo checklist so the GUI Explore "PLAN" section renders it.
        // Drop the two locked workflow rails here — they're internal bookkeeping,
        // not user-facing plan content (mirrors the `plan_ready` digest's own
        // `!it.locked` filter) — so the wire carries exactly the model's steps.
        plan_todos: rt
            .plan_todos
            .iter()
            .filter(|it| !it.locked)
            .map(|it| crate::ipc::proto::PlanTodoSnapshot {
                content: it.content.clone(),
                status: it.status.clone(),
            })
            .collect(),
    }
}

fn subagent_snapshot(sa: &crate::app::subagent::SubAgent) -> SubAgentSnapshot {
    let status = match &sa.status {
        SubAgentStatus::Running => "running".to_string(),
        SubAgentStatus::Done(_) => "done".to_string(),
        SubAgentStatus::Killed => "killed".to_string(),
        SubAgentStatus::Error(e) => format!("error: {e}"),
    };

    // Carry reasoning out-of-band (index-aligned with `messages`) since
    // `ChatMessage::reasoning` is `#[serde(skip)]`. Ship an empty vec in the
    // common no-reasoning case so the wire stays small.
    let committed_reasoning: Vec<Option<String>> =
        if sa.messages.iter().any(|m| m.reasoning.is_some()) {
            sa.messages.iter().map(|m| m.reasoning.clone()).collect()
        } else {
            Vec::new()
        };

    SubAgentSnapshot {
        id: sa.id,
        name: sa.agent_name.clone(),
        label: sa.label.clone(),
        status,
        detached: sa.detached,
        steps: sa.transcript.len(),
        transcript: sa.transcript.clone(),
        messages: sa.messages.clone(),
        live_text: sa.live_text.clone(),
        committed_reasoning,
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

/// Build a [`GlobalSnapshot`] from a PRE-BUILT [`ModeSnapshot`] instead of projecting
/// the mode fresh. See [`build_snapshot_with_mode`] for why the daemon supplies a cached
/// mode here; the default mode-projecting path is folded into [`build_snapshot`] via
/// [`mode_snapshot`], so there is no separate `global_snapshot` wrapper.
pub fn global_snapshot_with_mode(state: &AppState, mode: ModeSnapshot) -> GlobalSnapshot {
    GlobalSnapshot {
        input: state.rest.fg().input.clone(),
        cursor: state.rest.fg().cursor,
        scroll: state.rest.fg().scroll,
        follow: state.rest.fg().follow,
        status: state.rest.fg().status.clone(),
        work_elapsed_ms: state
            .rest
            .work_since
            .map(|since| since.elapsed().as_millis() as u64),
        theme: theme_token(&state.rest.config.theme).to_string(),
        accent: state.rest.config.accent.clone(),
        // Opaque registry key (like `accent`) — copied verbatim so the thin client
        // rebuilds the chosen palette instead of silently defaulting to `dark`.
        palette: state.rest.config.palette.clone(),
        mode,
        toast: state.rest.fg().toast.as_ref().map(|(msg, _until, kind)| {
            let kind = match kind {
                crate::app::state::ToastKind::Error => "error".to_string(),
                crate::app::state::ToastKind::Info => "info".to_string(),
            };
            (kind, msg.clone())
        }),
        models_cache: state.rest.models_cache.clone(),
        models_cache_endpoint: state.rest.models_cache_endpoint.clone(),
        models_cache_failed: state.rest.models_cache_failed.clone(),
        // Authoritative GLOBAL config catalogue for the native-React GUI's Connector +
        // MCP panels. `session_models` is the foreground session's per-session override
        // layer (the "local" scope); the rest mirror `AppConfig` directly.
        providers: state.rest.config.providers.clone(),
        config_models: state.rest.config.models.clone(),
        session_models: state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| s.settings.session_models.clone())
            .unwrap_or_default(),
        mcp_servers: state.rest.config.mcp_servers.clone(),
        agent_viewer: state.rest.agent_viewer,
        agent_viewer_scroll: state.rest.agent_viewer_scroll,
        agent_viewer_follow: state.rest.agent_viewer_follow,
        subagents_open: state.rest.subagents_open,
        subagent_sel: state.rest.subagent_sel,
        palette_sel: state.rest.palette_sel,
        pending_attachments: state.rest.fg().pending_attachments.clone(),
        file_palette: file_palette_matches(state),
        agent_mode: match state.rest.agent_mode {
            crate::app::state::AgentMode::Auto => "auto",
            crate::app::state::AgentMode::Normal => "normal",
            crate::app::state::AgentMode::Plan => "plan",
            crate::app::state::AgentMode::Yolo => "yolo",
        }
        .to_string(),
        latest_version: state.rest.latest_version.as_ref().map(|v| v.version.clone()),
    }
}

// Result cap for the @-file palette (how many matches are navigable/scrollable).
// The visible window is smaller (overlays.rs MAX_VIS); search is memoized so a
// larger cap is cheap. Lets a long match set scroll instead of truncating at 10.
const FILE_PAL_MAX: usize = 200;

fn file_palette_matches(state: &AppState) -> Option<Vec<String>> {
    let partial = crate::controller::input::file_ref_partial(&state.rest.fg().input)?;
    let matches = state
        .rest
        .fg()
        .dir_cache
        .read()
        .map(|c| c.search(partial, FILE_PAL_MAX))
        .unwrap_or_default();
    Some(matches)
}

//! Task command: `/task <agent> <task>` — spawn a sub-agent.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

/// Handle the `/task <agent> <task>` command: spawn a named sub-agent.
///
/// Guards against missing session/client, empty args, and the concurrency cap.
/// Uses the shared `stream::spawn_task` path so bookkeeping never diverges from
/// the `task` tool.
pub(super) fn handle_task(
    args: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // `/task` with no args is a convenient, always-reliable way to open the
    // sub-agents overlay (same panel as the `$` key, which only fires on an empty
    // composer). With args, fall through to the normal spawn path below.
    if args.trim().is_empty() {
        state.rest.subagents_open = true;
        state.rest.subagent_sel = state
            .rest
            .subagent_sel
            .min(state.rest.fg().subagents.len().saturating_sub(1));
        return Ok(());
    }
    // Guard: needs an active client + session.
    if client.is_none() || state.rest.fg().session.is_none() {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    }
    // Split the first whitespace token as the agent name; the rest is
    // the task text (original casing preserved).
    let mut tokens = args.splitn(2, char::is_whitespace);
    let agent_name = tokens.next().unwrap_or("").trim().to_string();
    let task_text = tokens.next().unwrap_or("").trim().to_string();
    if agent_name.is_empty() || task_text.is_empty() {
        state.rest.fg_mut().status = "usage: /task <agent> <task>".into();
        return Ok(());
    }
    // Spawn now if a slot is free, else ENQUEUE (unlimited pending; at most
    // MAX_SUBAGENTS run at once). The `/task` path never parks the main turn
    // (tool_call_id == None), so it just reports started vs queued. Uses the
    // shared `spawn_or_queue` helper so the ctx/registry/awareness/memory inputs
    // + bookkeeping never diverge from the `task` tool.
    let fgi = state.rest.foreground;
    match super::super::stream::spawn_or_queue(state, fgi, client, handle, &agent_name, &task_text, None)
    {
        super::super::stream::SpawnOutcome::Spawned(id) => {
            state
                .rest
                .fg_mut()
                .set_toast_info(format!("started sub-agent #{id} ({agent_name})"));
            state.rest.fg_mut().status = format!("started sub-agent #{id} ({agent_name})");
        }
        super::super::stream::SpawnOutcome::Queued(id) => {
            state
                .rest
                .fg_mut()
                .set_toast_info(format!("queued sub-agent #{id} ({agent_name})"));
            state.rest.fg_mut().status = format!("queued sub-agent #{id} ({agent_name})");
        }
        super::super::stream::SpawnOutcome::Failed => {
            state.rest.fg_mut().status = format!("unknown agent: {agent_name}");
        }
    }
    Ok(())
}

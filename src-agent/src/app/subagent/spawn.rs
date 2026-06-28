//! Sub-agent spawning: turn an agent name + task into a running [`SubAgent`].
//!
//! [`spawn_subagent`] looks the agent up in the registry, resolves its route,
//! builds the isolated seed conversation, and launches [`run_agent_loop`] on the
//! provided tokio runtime handle. It returns a [`SubAgent`] carrying the abort
//! handle + the event receiver the orchestrator drains (wired up in a later
//! stage). `None` when the named agent doesn't exist.

// Inert in Stage 1: the spawn entry point is fully implemented but not yet called
// from the chat loop / `task` tool, so its items are unreferenced until a later
// stage.
#![allow(dead_code)]

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::model::agent_def::AgentRegistry;
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;
use crate::tool::ToolCtx;

use super::context;
use super::engine::run_agent_loop;
use super::{SubAgent, SubAgentStatus};

/// Max characters of the task kept in the sub-agent's display label.
const LABEL_LEN: usize = 60;

/// Truncate `s` to at most `max` characters (char-boundary safe), appending an
/// ellipsis when it was cut. Used for the compact display label.
fn truncate_label(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// Spawn a sub-agent for `agent_name` with `task`, returning a handle to it.
///
/// Resolution:
/// - the agent is looked up in `registry` (case-insensitive); `None` if absent;
/// - its route is resolved via [`crate::app::resolve::resolve_agent`] (the
///   agent's own model + provider, falling back to the Main route when it has
///   none);
/// - its tool allow-list is [`AgentDef::effective_tools`] (never includes the
///   `task` recursion guard);
/// - its seed conversation is built by [`context::build_seed`] from the agent
///   persona + `memory_md` + `awareness` + `task` (fully isolated from the main
///   session history);
/// - its step budget is the agent's `steps` (`None` when absent = unbounded;
///   the loop terminates naturally when the model produces no tool calls).
///
/// The loop is spawned on `handle`; the returned [`SubAgent`] owns its
/// [`AbortHandle`](tokio::task::AbortHandle) and the receiver end of the event
/// channel. `config` / `settings` are cloned into the task (owned, no borrow
/// escapes); `ctx` is moved in whole.
#[allow(clippy::too_many_arguments)]
pub fn spawn_subagent(
    client: &Arc<OpenRouterClient>,
    handle: &tokio::runtime::Handle,
    registry: &AgentRegistry,
    config: &AppConfig,
    settings: &Settings,
    ctx: ToolCtx,
    awareness: &str,
    memory_md: &str,
    id: usize,
    agent_name: &str,
    task: &str,
    tool_call_id: Option<String>,
) -> Option<SubAgent> {
    // Look the agent up; a missing name is a no-op for the caller.
    let agent = registry.get(agent_name)?;

    // Resolve the agent's route (its own model+provider, else inherit Main).
    let resolved = crate::app::resolve::resolve_agent(config, settings, agent)?;

    // The effective allow-list + isolated seed conversation + step budget.
    let tools = agent.effective_tools();
    let convo = context::build_seed(agent, awareness, memory_md, task);
    // None = unbounded (natural termination when model returns no tool calls).
    // Some(n) = explicit per-agent cap from the agent-def `steps` field.
    let max_steps: Option<usize> = agent.steps.map(|s| s as usize);

    // Owned clones moved into the task so it borrows nothing from the caller.
    let client_arc = Arc::clone(client);
    let config = config.clone();
    let settings = settings.clone();
    let task_intent = task.to_string();
    let label = truncate_label(task, LABEL_LEN);
    let agent_name = agent.name.clone();

    // Save the model_id before `resolved` is moved into run_agent_loop.
    let spawned_model_id = resolved.model_id.clone();
    let (tx, rx) = mpsc::unbounded_channel();
    let jh = handle.spawn(run_agent_loop(
        client_arc,
        resolved,
        config,
        settings,
        tools,
        ctx,
        convo,
        task_intent,
        max_steps,
        tx,
    ));

    Some(SubAgent {
        id,
        agent_name,
        label,
        model_id: spawned_model_id,
        status: SubAgentStatus::Running,
        abort: jh.abort_handle(),
        rx,
        transcript: Vec::new(),
        messages: Vec::new(),
        tool_call_id,
        usage_tokens_in: 0,
        usage_tokens_out: 0,
        usage_cost: 0.0,
    })
}

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

use crate::app::state::AgentMode;
use crate::model::agent_def::AgentRegistry;
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;
use crate::tool::ToolCtx;

use super::context;
use super::engine::run_agent_loop;
use super::{SpawnOverrides, SubAgent, SubAgentStatus};

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
///
/// `overrides`, when `Some`, steers ONLY the route resolution below: a
/// `Some(model)`/`Some(effort)` field is applied to a throwaway CLONE of the
/// registry's [`AgentDef`] before resolving — the registry entry itself, and
/// every other input derived from it below (tools, prompt, steps), is
/// untouched. `None` (the overwhelming common case — every non-extension spawn
/// path passes it) resolves the agent exactly as before.
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
    detached: bool,
    ext_owned: bool,
    mode: AgentMode,
    overrides: Option<SpawnOverrides>,
) -> Option<SubAgent> {
    // Look the agent up; a missing name is a no-op for the caller.
    let agent = registry.get(agent_name)?;

    // Resolve the agent's route (its own model+provider, else inherit Main).
    // An override, when present, is applied to a CLONE used only for this
    // resolution — the registry's `agent` (used below for tools/prompt/steps)
    // is never mutated.
    let resolved = if let Some(ov) = &overrides {
        let mut overridden = agent.clone();
        if let Some(model) = &ov.model {
            overridden.model = Some(model.clone());
            overridden.model_uuid = None;
            overridden.provider_uuid = None;
        }
        if let Some(effort) = &ov.effort {
            overridden.effort = Some(effort.clone());
        }
        crate::app::resolve::resolve_agent(config, settings, &overridden)?
    } else {
        crate::app::resolve::resolve_agent(config, settings, agent)?
    };

    // The effective allow-list + isolated seed conversation + step budget. While
    // the PARENT session is in Plan mode, the delegated sub-agent must stay
    // read-only too — fold its allow-list down through the same whitelist used
    // by the main advertise fold (`tool_allowed_in_plan`), then strip two tools
    // that whitelist alone would let through: `seqthink` (main-agent-only — a
    // sub-agent has no user to ask "enter plan mode" on its behalf) and
    // `checklist` (its plan-mode interception lives in the main event loop's
    // `process_tools`; a sub-agent that ran the generic `Checklist::run` instead
    // would write the real per-directory `memory/TODO.md`, breaking plan-mode
    // read-only). This only ever NARROWS whatever the agent declared.
    let mut tools = agent.effective_tools();
    // Inherit connected MCP tools automatically, exactly like the main agent does
    // (no per-agent MCP picker): snapshot the shared manager's discovered tools —
    // the wire `ToolDef`s to advertise, plus their namespaced names appended to
    // this agent's allow-list so the execution gate in `run_agent_loop` keeps the
    // model's calls to them. Mirrors `app::runtime::stream::run` (run.rs:447-456).
    // With no MCP servers (or none connected yet) both are empty and the spawn is
    // byte-identical to the pre-MCP path.
    let mut mcp_tools: Vec<crate::dto::openrouter::ToolDef> = Vec::new();
    if let Some(mgr) = ctx.mcp_manager.as_ref() {
        let (defs, names) = mgr.advertise_cached();
        tools.extend(names);
        mcp_tools = defs;
    }
    if mode == AgentMode::Plan {
        // MCP tools ride through untouched — same precedent as the main advertise
        // fold at run.rs:487 (the user explicitly wired those servers, so they own
        // that risk), otherwise `tool_allowed_in_plan` would strip every mcp__*
        // name since it knows nothing about them.
        tools.retain(|n| {
            (crate::tool::tool_allowed_in_plan(n) && !matches!(n.as_str(), "seqthink" | "checklist"))
                || n.starts_with("mcp__")
        });
    }
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
        mcp_tools,
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
        live_text: String::new(),
        tool_call_id,
        detached,
        nudged: false,
        ext_owned,
        usage_tokens_in: 0,
        usage_tokens_out: 0,
        usage_cost: 0.0,
    })
}

//! Sub-agent (`task`) interceptor blocks (`task`, `task_output`, `task_kill`)
//! — split out of `intercepts.rs` for file size (pure code motion, no
//! behaviour change; see the parent module doc for the `InterceptFlow`
//! control-flow contract every `intercept_*` fn here follows).

use std::sync::Arc;

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::stream::tools::approval::parse_subagent_id;
use super::InterceptFlow;

pub(in crate::app::runtime::stream::tools) fn intercept_task(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("").trim();
    let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("").trim();
    // `run_in_background: true` makes the sub-agent DETACHED: the call is
    // answered IMMEDIATELY with its id (no park), mirroring bg-bash. The
    // model then polls it with `task_output` / stops it with `task_kill`.
    let background = args
        .get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if agent.is_empty() || prompt.is_empty() {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: task requires non-empty 'agent' and 'prompt'".to_string(),
        ));
    } else if background {
        // DETACHED branch (mirrors the bg-bash interception): spawn/queue
        // the agent NOT tied to this call (tool_call_id = None) and marked
        // detached, DO NOT record the call id in `pending_subagent_calls`
        // (so the round never parks on it), and push an IMMEDIATE result.
        // On terminal the detached agent fires a completion nudge (see
        // `drain_subagents` + `deferred.rs`) so the model knows to poll it.
        let agent = agent.to_string();
        let prompt = prompt.to_string();
        let result = match crate::app::runtime::stream::spawn::spawn_or_queue(
            state,
            sess_idx,
            client,
            handle,
            &agent,
            &prompt,
            None,  // detached: not tied to a blocking call
            true,  // detached = true
            None,  // no spawn overrides for a model-initiated task
        ) {
            crate::app::runtime::stream::spawn::SpawnOutcome::Spawned(id) => format!(
                "started background sub-agent #{id} ({agent}). It runs on its own and its \
                 full report is delivered to you automatically when it finishes — no need to \
                 poll or wait. Don't re-announce or repeat this to the user; just continue the \
                 conversation naturally, and you'll be woken with the result when it lands. \
                 (task_kill({{\"id\": {id}}}) to abort.)"
            ),
            crate::app::runtime::stream::spawn::SpawnOutcome::Queued(id) => format!(
                "queued background sub-agent #{id} ({agent}) — all {} slots busy; it \
                 starts when one frees, runs on its own, and delivers its full \
                 report to you automatically — no need to poll. Don't re-announce it to the \
                 user; just carry on naturally, and you'll be woken when it lands. \
                 (task_kill({{\"id\": {id}}}) to abort.)",
                crate::app::subagent::MAX_SUBAGENTS
            ),
            crate::app::runtime::stream::spawn::SpawnOutcome::Failed => {
                format!("error: unknown agent '{agent}'")
            }
        };
        state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
    } else {
        let agent = agent.to_string();
        let prompt = prompt.to_string();
        // Spawn now if a slot is free, else ENQUEUE (unlimited pending; at
        // most MAX_SUBAGENTS run at once). In BOTH the spawned and queued
        // cases DEFER the result by recording the call id in
        // `pending_subagent_calls`, so the parked round waits for the
        // delegation whether it runs now or later — its result fills when
        // the agent (eventually) finishes.
        match crate::app::runtime::stream::spawn::spawn_or_queue(
            state,
            sess_idx,
            client,
            handle,
            &agent,
            &prompt,
            Some(call.id.clone()),
            false, // blocking delegation (parks the round)
            None,  // no spawn overrides for a model-initiated task
        ) {
            crate::app::runtime::stream::spawn::SpawnOutcome::Spawned(_)
            | crate::app::runtime::stream::spawn::SpawnOutcome::Queued(_) => {
                state.rest.sessions[sess_idx].pending_subagent_calls.push(call.id.clone());
                // Park the round on this delegation IMMEDIATELY (mirrors
                // `dispatch_deferred` setting `awaiting_tool_tasks` inline at
                // dispatch): if a LATER call in this round early-returns
                // before the loop bottom — e.g. a risky call parks on the
                // classifier — the bottom-of-loop `awaiting_subagents = true`
                // reconciliation is skipped, and the sub-agent drain would
                // then not treat the round as parked. Setting it here keeps
                // the park flag correct mid-round; the bottom-of-loop set
                // stays as a (now-idempotent) backstop.
                state.rest.sessions[sess_idx].awaiting_subagents = true;
            }
            // Nothing started or queued (no client/session or unknown
            // agent) → answer the call now so it isn't left dangling.
            crate::app::runtime::stream::spawn::SpawnOutcome::Failed => state.rest.sessions[sess_idx]
                .tool_results
                .push((call.id.clone(), format!("error: unknown agent '{agent}'"))),
        }
    }
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(in crate::app::runtime::stream::tools) fn intercept_task_output(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let id_arg = args.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let result = match parse_subagent_id(&id_arg)
        .and_then(|n| state.rest.sessions[sess_idx].subagents.iter().find(|s| s.id == n))
    {
        Some(sa) => {
            use crate::app::subagent::SubAgentStatus::*;
            match &sa.status {
                Running => format!(
                    "[running] sub-agent #{} ({}) — still working. No need to poll again: \
                     its full report is delivered to you automatically when it finishes, \
                     and you'll be woken with the result. Just continue the conversation \
                     with the user meanwhile.",
                    sa.id, sa.agent_name
                ),
                Done(report) => {
                    format!("[done] sub-agent #{} ({})\n{report}", sa.id, sa.agent_name)
                }
                Error(e) => format!("[error] sub-agent #{} ({}): {e}", sa.id, sa.agent_name),
                Killed => format!("[killed] sub-agent #{} ({})", sa.id, sa.agent_name),
            }
        }
        None => {
            // No guessing: with up to MAX_SUBAGENTS running, returning the wrong
            // agent's report is worse than asking. Require an explicit id.
            if id_arg.is_null() {
                "error: task_output needs a numeric id, e.g. task_output({\"id\": 0}). \
                 This does NOT mean the sub-agent finished — a background sub-agent \
                 delivers its full report to you automatically when it is done, so there's no \
                 need to re-delegate or poll. Just continue with the user; you'll be woken when it lands."
                    .to_string()
            } else {
                format!(
                    "error: no sub-agent with id {id_arg}. Call task_output with a valid \
                     numeric id, e.g. task_output({{\"id\": 0}})."
                )
            }
        }
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(in crate::app::runtime::stream::tools) fn intercept_task_kill(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let id_arg = args.get("id").cloned().unwrap_or(serde_json::Value::Null);
    // Resolve the target id first (immutable borrow), then mutate by id.
    let explicit_id = parse_subagent_id(&id_arg)
        .filter(|&n| state.rest.sessions[sess_idx].subagents.iter().any(|s| s.id == n));
    let resolved_id: Result<usize, String> = if let Some(n) = explicit_id {
        Ok(n)
    } else {
        // No valid explicit id — try to infer a safe target.
        use crate::app::subagent::SubAgentStatus;
        let running: Vec<usize> = state.rest.sessions[sess_idx].subagents.iter()
            .filter(|s| matches!(s.status, SubAgentStatus::Running))
            .map(|s| s.id)
            .collect();
        match running.len() {
            0 => Err("error: no running sub-agent to kill.".to_string()),
            1 => Ok(running[0]),
            _ => {
                let list = running.iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(format!(
                    "error: task_kill needs an id — running sub-agents: {list}. \
                     e.g. task_kill({{\"id\": {}}})",
                    running[0]
                ))
            }
        }
    };
    let result = match resolved_id {
        Ok(target_id) => {
            use crate::app::subagent::SubAgentStatus;
            // Drop the immutable borrow before taking a mutable one.
            let sa = state.rest.sessions[sess_idx].subagents.iter_mut()
                .find(|s| s.id == target_id)
                .expect("id was validated above");
            // Abort the tokio task (best effort) and flip a still-Running
            // status to Killed so the $ panel + a later task_output reflect
            // it immediately (a terminal status is left untouched).
            sa.abort.abort();
            if matches!(sa.status, SubAgentStatus::Running) {
                sa.status = SubAgentStatus::Killed;
            }
            format!("sub-agent #{} killed", sa.id)
        }
        Err(msg) => msg,
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

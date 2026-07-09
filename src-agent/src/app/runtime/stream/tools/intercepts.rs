//! Sequential interceptor blocks for [`super::approval::process_tools`]'s tool
//! round — split out of `approval.rs` for file size (pure code motion, no
//! behaviour change). Each `intercept_*` fn is exactly one `process_tools` block
//! (gated on the call's tool name / mode in the CALLER, unchanged), taking the
//! same locals the block used and returning an [`InterceptFlow`] that replicates
//! the block's original control flow one-to-one:
//!
//! - every bare `continue;` in the original block becomes `return
//!   InterceptFlow::Continue;` (advance to the next `tool_idx`, same as before),
//! - every bare `return;` becomes `return InterceptFlow::Return;` (park the round
//!   — `process_tools` itself returns, unchanged),
//! - a block that could fall through past its own `if` (no continue/return on
//!   every path) ends with a trailing `InterceptFlow::Fallthrough`, and the
//!   caller falls through to the NEXT block in the same loop iteration exactly
//!   as the original code did.
//!
//! [`build_convo_context`] is the "plan-classifier preamble": the recent-history
//! plus approved-plan context string `process_tools` builds ONCE before the
//! loop, fed to the TAC classifier so it's intent-aware.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::app::state::AgentMode;
use crate::dto::chat::ToolCall;
use crate::service::openrouter::OpenRouterClient;

use super::approval::{
    bash_status_line, file_known_in_history, filter_bash_output, parse_bash_id,
    parse_subagent_id, spawn_classify_park, tac_inputs,
};

/// What an `intercept_*` block resolved to, mirroring the three ways the
/// original inline `if` block could end: keep looping (`Continue`), park the
/// round (`Return`), or fall through to the next block / the generic dispatch
/// path in THIS SAME iteration (`Fallthrough`).
pub(super) enum InterceptFlow {
    /// `continue` the `tool_idx` while-loop in `process_tools`.
    Continue,
    /// `return` from `process_tools` entirely (the round parked).
    Return,
    /// Not handled by this intercept — fall through to whatever comes next.
    Fallthrough,
}

/// Build the recent-history context string fed to the TAC classifier so it's
/// intent-aware (sees the last few turns, not just a terse confirmation like
/// "ok go!"). When a plan was just approved and is executing, prepends a
/// preamble instructing the classifier to ALLOW the calls that carry it out
/// and only flag off-plan/destructive actions — the classifier still runs, this
/// only enriches its context. Identical behaviour when no plan is stashed.
pub(super) fn build_convo_context(state: &AppState, sess_idx: usize) -> String {
    let base = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|sess| sess.conversation.recent_context(6, 600))
        .unwrap_or_default();
    match &state.rest.sessions[sess_idx].approved_plan {
        Some(plan) => format!(
            "[The user has APPROVED the following plan and asked to execute it now. ALLOW tool calls that carry out this plan — file writes/edits and shell commands needed to implement it are authorized. Only flag calls that are clearly OFF-PLAN, destructive beyond the plan's scope, or dangerous.]\n\nAPPROVED PLAN:\n{plan}\n\n--- recent conversation ---\n{base}"
        ),
        None => base,
    }
}

pub(super) fn intercept_plan_enter(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
) -> InterceptFlow {
    let result = super::dispatch::run_tool(state, sess_idx, call);
    let final_result = if result == crate::tool::plan::PLAN_ENTER_SENTINEL {
        if mode == AgentMode::Plan {
            "already in plan mode".to_string()
        } else {
            state.rest.set_agent_mode(AgentMode::Plan);
            "entered plan mode - tools are read-only; explore, structure your \
             reasoning with seqthink, build the checklist with todowrite, and \
             call plan_ready with highlights + the full plan when confident"
                .to_string()
        }
    } else {
        // Not expected (plan_enter has no failure path), but pass through
        // unchanged rather than swallow it.
        result
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_plan_ready(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
) -> InterceptFlow {
    if mode != AgentMode::Plan {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "plan_ready is only available in plan mode".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    // Settled-session guard: plan_ready is only accepted once ALL of this
    // session's background work has finished. A still-running background bash
    // job or sub-agent (typically a detached one — a blocking one would have
    // parked this round) means the plan may rest on incomplete results, so
    // REJECT (do NOT park) and tell the model to collect the outputs first.
    let mut pending: Vec<String> = Vec::new();
    for j in &state.rest.sessions[sess_idx].bash_jobs {
        if matches!(j.snapshot_status(), crate::app::bgbash::BashJobStatus::Running) {
            pending.push(format!("bash-{}", j.id));
        }
    }
    for sa in &state.rest.sessions[sess_idx].subagents {
        if matches!(sa.status, crate::app::subagent::SubAgentStatus::Running) {
            pending.push(format!("#{} ({})", sa.id, sa.agent_name));
        }
    }
    if !pending.is_empty() {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!(
                "plan_ready rejected: background work is still running ({}). Collect the \
                 results with bash_output/task_output, incorporate them into the plan, \
                 then call plan_ready again.",
                pending.join(", ")
            ),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let (highlights, plan) = match crate::tool::plan::parse_plan_ready_args(&args) {
        Ok(pair) => pair,
        Err(e) => {
            state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), e));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };
    // Persist the full plan to `<session>/plan.md` (atomic tmp + rename).
    // On IO failure (or no active session) answer the call with an error
    // and continue — do NOT park on an unwritten plan.
    let plan_path = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.plan_path());
    match plan_path {
        Some(path) => {
            if let Err(e) =
                crate::model::memory::atomic_write(&path, plan.as_bytes())
            {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!("error: could not write plan.md: {e}"),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
        }
        None => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: could not write plan.md: no active session".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    }
    // Compose the USER-FACING plan digest and swap it into the stored
    // plan_ready tool-call args, so the transcript renders the digest
    // (checklist + full plan when short, else checklist + highlights)
    // instead of the raw model arg. The tool call is already committed to
    // history (turn.rs commits the assistant message before this runs), so
    // rewriting its args needs no view/projection change — the next snapshot
    // projects the mutated conversation. Best-effort display shaping: with no
    // active session we skip and the raw arg render is the fallback.
    {
        use crate::app::mode::todo::{self, TodoStatus};
        if let Some(tpath) = state.rest.sessions[sess_idx]
            .session
            .as_ref()
            .map(|s| s.plan_todos_path())
        {
            let mut items = todo::load_todos_from(&tpath);
            // Checklist = the model's step items only; the two locked rails
            // are internal workflow, not user-facing plan content.
            let steps: Vec<String> = items
                .iter()
                .filter(|it| !it.locked)
                .map(|it| it.content.clone())
                .collect();
            let mut checklist = if steps.is_empty() {
                "Plan:".to_string()
            } else {
                format!(
                    "Plan ({} step{}):",
                    steps.len(),
                    if steps.len() == 1 { "" } else { "s" }
                )
            };
            for (i, s) in steps.iter().enumerate() {
                checklist.push_str(&format!("\n  {}. {}", i + 1, s));
            }
            // "short" = the full plan body is <= 2000 bytes (equals plan.md,
            // just written): small enough to show inline. Longer plans render
            // the checklist + the model's highlights digest, not the whole text.
            let composed = if plan.len() <= 2000 {
                format!("{checklist}\n\n{plan}")
            } else {
                format!("{checklist}\n\n{highlights}")
            };
            // Overwrite only the `highlights` field (preserve `plan`), keeping
            // valid JSON so the transcript parses it unchanged.
            let mut new_args = args.clone();
            if let Some(obj) = new_args.as_object_mut() {
                obj.insert(
                    "highlights".to_string(),
                    serde_json::Value::String(composed),
                );
            }
            let new_args_str = new_args.to_string();
            // Mark the two locked rails Completed now that the plan is served
            // + saved, so `/todo` reads as a finished workflow while parked.
            for it in items.iter_mut() {
                if it.locked {
                    it.status = TodoStatus::Completed;
                }
            }
            let _ = todo::save_todos_to(&tpath, &items);
            // Refresh the in-memory mirror so the GUI Explore "PLAN" section
            // reflects the rails' Completed flip the instant the plan parks
            // (the projection filters the locked rails back out on the wire).
            state.rest.sessions[sess_idx].plan_todos = items;
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.conversation.set_tool_call_args(&call.id, new_args_str);
                let _ = sess.save();
            }
        }
    }
    // Park for the user's decision. Do NOT advance `tool_idx` — the resume
    // handlers answer this exact plan_ready call. The digest renders in the
    // transcript (via the rewritten tool-call args), so `approval_reason` is
    // unused here — the plan-ready overlay shows only the y/a/n footer.
    state.rest.sessions[sess_idx].awaiting_approval = true;
    state.rest.sessions[sess_idx].approval_reason = None;
    state.rest.sessions[sess_idx].status =
        "plan ready - [y] approve  [a] approve & compact  [n] chat more".to_string();
    InterceptFlow::Return
}

pub(super) fn intercept_todowrite_plan(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    use crate::app::mode::todo::{
        self, TodoItem, TodoPriority, TodoStatus, PLAN_RAIL_SAVE, PLAN_RAIL_SERVE,
    };
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let plan_todos_path = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.plan_todos_path());
    let result = match plan_todos_path {
        Some(path) => {
            // Rails are ALWAYS reset to Pending on an active todowrite — a
            // model call means planning is still in progress, so
            // "serve"/"save" cannot legitimately be done yet. This also
            // un-sticks the rails after a plan_ready → deny → re-plan cycle
            // (plan_ready marks them Completed; without this reset they'd
            // stay Completed while the model reworks the plan). Only
            // `plan_ready` marks them Completed.
            // The model's items: drop any whose content case-insensitively
            // equals a rail (the model must not fabricate/move the rails) or
            // is blank (blank content never survives a parse round-trip), and
            // force `locked:false` on everything it sends.
            let model_items: Vec<TodoItem> = args
                .get("todos")
                .and_then(serde_json::Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|it| {
                            let content = it
                                .get("content")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if content.is_empty()
                                || content.eq_ignore_ascii_case(PLAN_RAIL_SERVE)
                                || content.eq_ignore_ascii_case(PLAN_RAIL_SAVE)
                            {
                                return None;
                            }
                            let status = it
                                .get("status")
                                .and_then(serde_json::Value::as_str)
                                .map(TodoStatus::from_str)
                                .unwrap_or(TodoStatus::Pending);
                            let priority = it
                                .get("priority")
                                .and_then(serde_json::Value::as_str)
                                .map(TodoPriority::from_str)
                                .unwrap_or(TodoPriority::Medium);
                            Some(TodoItem { content, status, priority, locked: false })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let n = model_items.len();
            // Model steps first, then ALWAYS the two locked rails as the last
            // two entries (status carried over from the existing file).
            let mut merged = model_items;
            merged.push(TodoItem {
                content: PLAN_RAIL_SERVE.to_string(),
                status: TodoStatus::Pending,
                priority: TodoPriority::Low,
                locked: true,
            });
            merged.push(TodoItem {
                content: PLAN_RAIL_SAVE.to_string(),
                status: TodoStatus::Pending,
                priority: TodoPriority::Low,
                locked: true,
            });
            let saved = todo::save_todos_to(&path, &merged);
            if saved.is_ok() {
                // Refresh the in-memory mirror so the GUI Explore "PLAN"
                // section reflects this todowrite immediately (the
                // projection filters the locked rails back out on the wire).
                state.rest.sessions[sess_idx].plan_todos = merged;
            }
            match saved {
                Ok(()) => format!("Updated plan: {n} step(s) + 2 rails"),
                Err(e) => format!("error: could not write plan todos: {e}"),
            }
        }
        None => "error: no active session — cannot write plan todos".to_string(),
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_plan_readonly_gate(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    if call.function.name == "git_operator" {
        let sanitized =
            crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
        let args: serde_json::Value =
            serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
        let subcmd = args
            .get("args")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !crate::tool::plan_git_subcommand_allowed(subcmd) {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!(
                    "plan mode is read-only: git {subcmd} is not allowed (read-only git only)"
                ),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
        // An allowed read-only git subcommand falls through to the normal
        // gate flow below — git_operator is risky, so Auto/Normal/Yolo
        // handling applies unchanged (Plan is treated like Auto there; see
        // the comments at the classifier verdict branches).
    } else if !crate::tool::tool_allowed_in_plan(&call.function.name) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!(
                "plan mode is read-only: {} is unavailable until the plan is approved",
                call.function.name
            ),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    InterceptFlow::Fallthrough
}

pub(super) fn intercept_task(
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
        let result = match super::super::spawn::spawn_or_queue(
            state,
            sess_idx,
            client,
            handle,
            &agent,
            &prompt,
            None,  // detached: not tied to a blocking call
            true,  // detached = true
        ) {
            super::super::spawn::SpawnOutcome::Spawned(id) => format!(
                "started background sub-agent #{id} ({agent}). It runs on its own and its \
                 full report is delivered to you automatically when it finishes — no need to \
                 poll or wait. Don't re-announce or repeat this to the user; just continue the \
                 conversation naturally, and you'll be woken with the result when it lands. \
                 (task_kill({{\"id\": {id}}}) to abort.)"
            ),
            super::super::spawn::SpawnOutcome::Queued(id) => format!(
                "queued background sub-agent #{id} ({agent}) — all {} slots busy; it \
                 starts when one frees, runs on its own, and delivers its full \
                 report to you automatically — no need to poll. Don't re-announce it to the \
                 user; just carry on naturally, and you'll be woken when it lands. \
                 (task_kill({{\"id\": {id}}}) to abort.)",
                crate::app::subagent::MAX_SUBAGENTS
            ),
            super::super::spawn::SpawnOutcome::Failed => {
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
        match super::super::spawn::spawn_or_queue(
            state,
            sess_idx,
            client,
            handle,
            &agent,
            &prompt,
            Some(call.id.clone()),
            false, // blocking delegation (parks the round)
        ) {
            super::super::spawn::SpawnOutcome::Spawned(_)
            | super::super::spawn::SpawnOutcome::Queued(_) => {
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
            super::super::spawn::SpawnOutcome::Failed => state.rest.sessions[sess_idx]
                .tool_results
                .push((call.id.clone(), format!("error: unknown agent '{agent}'"))),
        }
    }
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_bash_background(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let background = args
        .get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if background {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let result = if command.trim().is_empty() {
            "error: bash requires a non-empty 'command'".to_string()
        } else {
            // Lazily create THIS session's completion channel once, then
            // reuse it (mirrors the deferred tool-task channel). The worker
            // fires the finished job id over `bash_done_tx`; the event-loop
            // deferred drain reads `bash_done_rx` to pop a toast.
            if state.rest.sessions[sess_idx].bash_done_tx.is_none() {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                state.rest.sessions[sess_idx].bash_done_tx = Some(tx);
                state.rest.sessions[sess_idx].bash_done_rx = Some(rx);
            }
            let id = state.rest.sessions[sess_idx].next_bash_id();
            // Same effective cwd the inline `bash` runs in (the `cd`
            // override, else the configured workdir).
            let cwd = state.rest.sessions[sess_idx].effective_cwd();
            let done_tx = state.rest.sessions[sess_idx].bash_done_tx.clone();
            let job = crate::app::bgbash::spawn_bash_job(id, command, cwd, done_tx);
            state.rest.sessions[sess_idx].bash_jobs.push(job);
            // Persist the new job record so it survives close/reopen (#25).
            crate::app::runtime::bg_persist::persist_bash_jobs(&state.rest.sessions[sess_idx]);
            format!(
                "started background job bash-{id} (running). Poll with \
                 bash_output{{\"job_id\":\"bash-{id}\"}}, stop with \
                 bash_kill{{\"job_id\":\"bash-{id}\"}}."
            )
        };
        state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    // Not a background bash — fall through to the normal path below.
    InterceptFlow::Fallthrough
}

pub(super) fn intercept_bash_output(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let job_id = args.get("job_id").and_then(|v| v.as_str()).unwrap_or("").trim();
    let tail_lines = args.get("tail_lines").and_then(|v| v.as_u64()).map(|n| n as usize);
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let result = match parse_bash_id(job_id)
        .and_then(|n| state.rest.sessions[sess_idx].bash_jobs.iter().find(|j| j.id == n))
    {
        Some(job) => {
            let status = job.snapshot_status();
            let line = bash_status_line(&status);
            let out = job.output_snapshot();
            // Only reshape a FINISHED job's output when the model gave
            // no explicit pattern/tail_lines shaping of its own — an
            // explicit ask is deliberate and must be respected as-is.
            // Running/Killed/Error, and any poll with pattern or
            // tail_lines set, take the exact original path below.
            let finished_code = match status {
                crate::app::bgbash::BashJobStatus::Done(code) => Some(code),
                _ => None,
            };
            let saving = state.rest.sessions[sess_idx]
                .session
                .as_ref()
                .map(|s| s.settings.bash_saving)
                .unwrap_or(true);
            let qualifies =
                finished_code.is_some() && tail_lines.is_none() && pattern.is_none() && saving;

            if out.is_empty() {
                format!("{line}\n(no output yet)")
            } else if qualifies {
                // Mirror `tool::shell::finalize_output`'s "saving" path
                // (filter + tee) for a finished background job, same as
                // synchronous bash/git_operator.
                let code = finished_code.expect("qualifies implies finished_code is Some");
                let (text, should_tee) =
                    crate::app::bgbash::render_finished_output(&job.command, &out, code, saving);
                let mut body = format!("{line}\n{text}");
                if should_tee {
                    let log_dir = state.rest.sessions[sess_idx]
                        .session
                        .as_ref()
                        .map(|s| s.path.join("opt"));
                    if let Some(dir) = log_dir {
                        if let Some(path) = job.ensure_tee_log(&dir, &out) {
                            body.push_str(&format!("\nfull-output: {}", path.display()));
                        }
                    }
                }
                body
            } else {
                match filter_bash_output(&out, tail_lines, pattern) {
                    Ok(filtered) if filtered.is_empty() => format!("{line}\n(no matching lines)"),
                    Ok(filtered) => format!("{line}\n{filtered}"),
                    Err(e) => format!("{line}\n[{e}]"),
                }
            }
        }
        None => format!("error: no such job: {job_id}"),
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_bash_kill(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let job_id = args.get("job_id").and_then(|v| v.as_str()).unwrap_or("").trim();
    let result = match parse_bash_id(job_id)
        .and_then(|n| state.rest.sessions[sess_idx].bash_jobs.iter().find(|j| j.id == n))
    {
        Some(job) => {
            crate::app::bgbash::kill_bash_job(job);
            format!("job bash-{} killed", job.id)
        }
        None => format!("error: no such job: {job_id}"),
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_task_output(
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

pub(super) fn intercept_task_kill(
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

pub(super) fn intercept_cd(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> InterceptFlow {
    let result = super::dispatch::run_tool(state, sess_idx, call);
    let final_result = if let Some(target) = result.strip_prefix(crate::tool::cd::CWD_CHANGE_PREFIX) {
        let new_cwd = std::path::PathBuf::from(target);
        super::super::spawn::apply_workspace_change(state, sess_idx, new_cwd, client, handle);
        format!("changed working directory to {target}")
    } else {
        // Already an `error:`/refusal line — pass it through unchanged.
        result
    };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_git_cred(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let result = super::dispatch::run_tool(state, sess_idx, call);
    let final_result =
        if let Some(key) = result.strip_prefix(crate::tool::git_cred::GIT_CRED_SELECT_PREFIX) {
            // Apply the selection: write into settings and persist.
            let key = key.to_string();
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.settings.git_ssh_key = Some(key.clone());
                let _ = sess.save();
            }
            format!("selected ssh key: {key}")
        } else {
            // list output or error: — pass through unchanged.
            result
        };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_git_worktree(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    convo_context: &str,
) -> InterceptFlow {
    // Gate the destructive `remove` action behind the approval classifier —
    // it deletes a worktree (hard to undo). The other actions (create /
    // enter / exit / list) only move cwd/roots and are cheap to reverse, so
    // they skip the gate. On the resume pass after the user approves,
    // `approved_worktree_call` holds this call's id → skip re-gating and run
    // the interception for real. Mirrors the generic risky gate below
    // (~line 622+) but lives here because git_worktree is intercepted before
    // that gate and can't reach it.
    let wt_args: serde_json::Value = serde_json::from_str(
        &crate::dto::chat::sanitize_tool_arguments(&call.function.arguments),
    )
    .unwrap_or_default();
    let is_remove =
        wt_args.get("action").and_then(|a| a.as_str()) == Some("remove");
    let pre_approved = state.rest.sessions[sess_idx]
        .approved_worktree_call
        .as_deref()
        == Some(call.id.as_str());
    if pre_approved {
        // Consume the one-shot approval so a later un-approved remove re-gates.
        state.rest.sessions[sess_idx].approved_worktree_call = None;
    } else if is_remove && mode != AgentMode::Yolo {
        match tac_inputs(state, sess_idx, client) {
            Some((c, config, settings)) => {
                // Async TAC gate (mirrors the generic risky gate below):
                // take a drain-staged verdict for THIS call, else spawn the
                // classifier off-thread and PARK — the round re-enters this
                // arm with the verdict once it lands (`pre_approved` stays
                // false, `is_remove` stays true, so it lands back here). A
                // stale staged id is dropped and re-classified. The three-way
                // branch below is UNCHANGED.
                let verdict = match state.rest.sessions[sess_idx]
                    .pending_classify_verdict
                    .take()
                {
                    Some((vid, v)) if vid == call.id => v,
                    _ => {
                        spawn_classify_park(
                            state, sess_idx, handle, c, config, settings,
                            convo_context, call,
                        );
                        return InterceptFlow::Return;
                    }
                };
                if verdict.available && verdict.allow {
                    // Definite allow. Auto runs inline; Normal still asks.
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].approval_reason =
                            Some(format!("classifier: ok — {}", verdict.reason));
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status =
                            format!("approve {}? [y/n]", call.function.name);
                        return InterceptFlow::Return;
                    }
                    // Auto + allow → fall through and run it inline.
                } else if verdict.available {
                    // Definite block. Auto records + continues; Normal asks.
                    // Plan never reaches this `is_remove` classifier flow at
                    // all — `git_worktree` isn't in `tool_allowed_in_plan`, so
                    // the read-only gate above already denied it before this
                    // point, leaving only Auto/Normal/Yolo here.
                    if mode == AgentMode::Auto {
                        state.rest.sessions[sess_idx].tool_results.push((
                            call.id.clone(),
                            format!("blocked by harness: {}", verdict.reason),
                        ));
                        state.rest.sessions[sess_idx].tool_idx += 1;
                        return InterceptFlow::Continue;
                    }
                    state.rest.sessions[sess_idx].approval_reason =
                        Some(verdict.reason);
                    state.rest.sessions[sess_idx].awaiting_approval = true;
                    state.rest.sessions[sess_idx].status =
                        format!("approve {}? [y/n]", call.function.name);
                    return InterceptFlow::Return;
                } else {
                    // Classifier unavailable. Normal → human y/n; Auto →
                    // fail-CLOSED (never delete a worktree unverified).
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].approval_reason =
                            Some(verdict.reason.clone());
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status =
                            format!("approve {}? [y/n]", call.function.name);
                        return InterceptFlow::Return;
                    }
                    state.rest.sessions[sess_idx].tool_results.push((
                        call.id.clone(),
                        format!(
                            "not executed: classifier unavailable — {}. The \
                             safety classifier could not verify this \
                             git_worktree remove, so it was NOT run.",
                            verdict.reason
                        ),
                    ));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            }
            // Classifier disabled → Normal asks, Auto runs.
            None => {
                if mode == AgentMode::Normal {
                    state.rest.sessions[sess_idx].awaiting_approval = true;
                    state.rest.sessions[sess_idx].status =
                        format!("approve {}? [y/n]", call.function.name);
                    return InterceptFlow::Return;
                }
                // Auto + classifier disabled → fall through and run inline.
            }
        }
    }
    let result = super::dispatch::run_tool(state, sess_idx, call);
    let final_result =
        if let Some(target) =
            result.strip_prefix(crate::tool::git_worktree::GIT_WT_CREATE_PREFIX)
        {
            // `create` succeeded: target is the shadow path string.
            // Same state work as enter: register the path + persist + switch cwd.
            let new_cwd = std::path::PathBuf::from(target);
            let target_str = target.to_string();
            {
                if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                    sess.settings.enter_worktree(target_str.clone());
                    let _ = sess.save();
                }
            }
            super::super::spawn::apply_workspace_change(
                state, sess_idx, new_cwd.clone(), client, handle,
            );
            // Emit a clear "created + entered" confirmation so no model
            // misreads this as a failure (unlike the bare "entered worktree"
            // string the old enter sentinel would have produced).
            let name = std::path::Path::new(target)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(target);
            format!(
                "created worktree '{name}' at {target} and switched into it \
                 — you are now working inside the new worktree. \
                 Use git_worktree({{\"action\":\"exit\"}}) to return to the repo root."
            )
        } else if let Some(target) =
            result.strip_prefix(crate::tool::git_worktree::GIT_WT_ENTER_PREFIX)
        {
            // `enter` succeeded: target is the canonical path string.
            let new_cwd = std::path::PathBuf::from(target);
            let target_str = target.to_string();
            // Swap slot [0] to the worktree root (stashing the current
            // primary root for restore on exit), then persist. Scoped so
            // the mutable sess borrow ends before we call
            // apply_workspace_change (which also borrows state mut).
            {
                if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                    sess.settings.enter_worktree(target_str.clone());
                    let _ = sess.save();
                }
            }
            super::super::spawn::apply_workspace_change(
                state, sess_idx, new_cwd.clone(), client, handle,
            );
            format!("entered worktree: {}", new_cwd.display())
        } else if result.starts_with(crate::tool::git_worktree::GIT_WT_EXIT_PREFIX) {
            // `exit`: restore the base primary root (swap slot [0] back) and return
            // to it. Extra roots in workdir[1..] are preserved. Mutate + save in a
            // scoped borrow, then call apply_workspace_change outside it.
            //
            // Capture whether we were ACTUALLY inside an entered worktree BEFORE the
            // swap: `workdir_saved.is_some()` means a real worktree is active and
            // exit_worktree() will restore the base; `is_none()` means there is
            // nothing to exit (e.g. the session was launched FROM a worktree). We
            // must report these distinctly or the model can't tell a no-op from a
            // real exit and retries `exit` in a loop.
            let (primary, was_active) = {
                if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                    let was_active = sess.settings.workdir_saved.is_some();
                    sess.settings.exit_worktree();
                    let _ = sess.save();
                    (sess.workdir(), was_active)
                } else {
                    (std::path::PathBuf::from("."), false)
                }
            };
            super::super::spawn::apply_workspace_change(
                state, sess_idx, primary.clone(), client, handle,
            );
            if was_active {
                format!("exited worktree — now at {}", primary.display())
            } else {
                format!(
                    "no active worktree to exit — already at {} (this session started here); nothing to do",
                    primary.display()
                )
            }
        } else if let Some(removed) =
            result.strip_prefix(crate::tool::git_worktree::GIT_WT_REMOVE_PREFIX)
        {
            // `remove` succeeded: the worktree is already deleted (git ran
            // from the repo root). Two cleanups:
            // (1) de-register the path from settings.workdir; (2) if the
            // session's live cwd was inside the removed worktree it now
            // points at a dead dir — snap it back to the primary workdir
            // (repo root). Capture the primary path in the same scoped
            // borrow, then apply outside it (apply_workspace_change also
            // borrows state mutably).
            let removed = removed.to_string();
            let primary;
            {
                if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                    // Removing the worktree we're standing in → restore the base
                    // root (swap slot [0] back). Removing a different worktree/dir
                    // by name → just drop it wherever it sits in the list.
                    let in_removed = sess
                        .settings
                        .workdir
                        .first()
                        .map(|p| p == &removed)
                        .unwrap_or(false);
                    if in_removed {
                        sess.settings.exit_worktree();
                    } else {
                        sess.settings.workdir.retain(|p| p != &removed);
                    }
                    let _ = sess.save();
                    primary = sess.workdir();
                } else {
                    primary = std::path::PathBuf::from(".");
                }
            }
            let stale = state.rest.sessions[sess_idx]
                .active_cwd
                .as_ref()
                .is_some_and(|c| !c.is_dir());
            if stale {
                super::super::spawn::apply_workspace_change(
                    state, sess_idx, primary.clone(), client, handle,
                );
            }
            format!("worktree removed: {removed}")
        } else {
            // list output, or an error: — pass through.
            result
        };
    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(super) fn intercept_read_before_edit_guard(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized =
        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
        let path_str = path_str.to_string();
        // Build workspaces the same way the tools do.
        let ctx = super::super::spawn::build_tool_ctx(state, sess_idx);
        if let Ok(target_abs) = crate::tool::resolve(&ctx.workspaces, &path_str) {
            let is_edit = call.function.name == "edit";
            // write only guards when OVERWRITING an existing file; new file is exempt.
            let must_check = is_edit || target_abs.exists();
            if must_check {
                // Scope the immutable borrow of session so it ends before
                // we mutate state below (push result / advance tool_idx).
                let known = {
                    let msgs = state.rest.sessions[sess_idx]
                        .session
                        .as_ref()
                        .map(|s| s.conversation.messages())
                        .unwrap_or(&[]);
                    file_known_in_history(msgs, &ctx.workspaces, &target_abs)
                };
                if !known {
                    let verb = if is_edit { "editing" } else { "overwriting" };
                    let nudge = format!(
                        "error: read '{path_str}' before {verb} it — call \
                         read({{\"path\":\"{path_str}\"}}) first so you're working \
                         against the current file, then retry. \
                         (Creating a brand-new file needs no prior read.)"
                    );
                    // Mirror exactly how the TAC classifier DENIES a call in
                    // Auto mode (definite block): push a synthetic result for
                    // this call id, advance tool_idx, and continue the loop
                    // without running the tool.
                    state.rest.sessions[sess_idx]
                        .tool_results
                        .push((call.id.clone(), nudge));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            }
        }
    }
    InterceptFlow::Fallthrough
}


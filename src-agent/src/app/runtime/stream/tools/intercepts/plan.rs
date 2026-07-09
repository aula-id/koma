//! Plan-mode interceptor blocks (`plan_enter`, `plan_ready`, `todowrite` while
//! in Plan, and the Plan read-only enforcement gate), plus the shared
//! `build_convo_context` preamble — split out of `intercepts.rs` for file size
//! (pure code motion, no behaviour change; see the parent module doc for the
//! `InterceptFlow` control-flow contract every `intercept_*` fn here follows).

use crate::app::state::AppState;
use crate::app::state::AgentMode;
use crate::dto::chat::ToolCall;
use super::InterceptFlow;

/// Build the recent-history context string fed to the TAC classifier so it's
/// intent-aware (sees the last few turns, not just a terse confirmation like
/// "ok go!"). When a plan was just approved and is executing, prepends a
/// preamble instructing the classifier to ALLOW the calls that carry it out
/// and only flag off-plan/destructive actions — the classifier still runs, this
/// only enriches its context. Identical behaviour when no plan is stashed.
pub(in crate::app::runtime::stream::tools) fn build_convo_context(state: &AppState, sess_idx: usize) -> String {
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

pub(in crate::app::runtime::stream::tools) fn intercept_plan_enter(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
) -> InterceptFlow {
    let result = crate::app::runtime::stream::tools::dispatch::run_tool(state, sess_idx, call);
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

pub(in crate::app::runtime::stream::tools) fn intercept_plan_ready(
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

pub(in crate::app::runtime::stream::tools) fn intercept_todowrite_plan(
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

pub(in crate::app::runtime::stream::tools) fn intercept_plan_readonly_gate(
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

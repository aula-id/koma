//! Tool-approval state machine: classify, run, deny, finish tool rounds.
//! Includes risky-tool detection, TAC (tool-call classifier) inputs, and the
//! main `process_tools` loop that drives approval/dispatch for each tool call.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use crate::service::openrouter::OpenRouterClient;
use crate::app::state::AgentMode;

/// True for tools that mutate the workspace (or run arbitrary shell commands)
/// and therefore require approval in Normal mode. Thin re-export of the
/// canonical definition in [`crate::tool::tool_is_risky`] so callers within
/// this module use the local name unchanged.
fn tool_is_risky(name: &str) -> bool {
    crate::tool::tool_is_risky(name)
}

/// Parse a background-bash job handle (`bash-<n>`, or a bare `<n>`) into its
/// numeric id. Returns `None` for anything that doesn't end in a number, so an
/// unknown/garbage handle surfaces a clean "no such job" error to the model.
fn parse_bash_id(job_id: &str) -> Option<usize> {
    job_id
        .strip_prefix("bash-")
        .unwrap_or(job_id)
        .trim()
        .parse::<usize>()
        .ok()
}

/// Parse a detached sub-agent id from a `task_output` / `task_kill` `id` arg,
/// which the model may send as a JSON integer OR a string (`3` or `"3"`, and
/// tolerating a stray `#`/`sub-`/`agent-` prefix). Returns `None` for anything
/// that doesn't resolve to a number, so an unknown handle surfaces a clean "no
/// such sub-agent" error. Mirrors [`parse_bash_id`], generalised to both JSON
/// shapes because the `task_*` tools declare `id` as `integer | string`.
fn parse_subagent_id(v: &serde_json::Value) -> Option<usize> {
    if let Some(n) = v.as_u64() {
        return usize::try_from(n).ok();
    }
    let s = v.as_str()?.trim();
    s.strip_prefix('#')
        .unwrap_or(s)
        .strip_prefix("sub-")
        .or_else(|| s.strip_prefix("agent-"))
        .unwrap_or(s)
        .trim()
        .parse::<usize>()
        .ok()
}

/// Render a one-line status banner for a background bash job, shown FIRST in a
/// `bash_output` result so the model sees the lifecycle at a glance: `[running]`,
/// `[exit <code>]`, `[killed]`, or `[error: <msg>]`.
fn bash_status_line(status: &crate::app::bgbash::BashJobStatus) -> String {
    use crate::app::bgbash::BashJobStatus::*;
    match status {
        Running => "[running]".to_string(),
        Done(code) => format!("[exit {code}]"),
        Killed => "[killed]".to_string(),
        Error(msg) => format!("[error: {msg}]"),
    }
}

/// Filter a background job's captured output for `bash_output`: keep only lines
/// matching `pattern` (a regex, grep-style) when set, then the LAST `tail_lines`
/// of what remains when set (>0). Returns `Err` with a clean message on an invalid
/// regex so the model sees why its filter was rejected. An empty `Ok` string means
/// "no lines left" (the caller maps it to a friendly note).
fn filter_bash_output(
    out: &str,
    tail_lines: Option<usize>,
    pattern: Option<&str>,
) -> Result<String, String> {
    let mut lines: Vec<&str> = out.lines().collect();
    if let Some(pat) = pattern {
        let re = regex::Regex::new(pat).map_err(|e| format!("invalid pattern: {e}"))?;
        lines.retain(|l| re.is_match(l));
    }
    if let Some(n) = tail_lines {
        if n > 0 && lines.len() > n {
            lines = lines.split_off(lines.len() - n);
        }
    }
    Ok(lines.join("\n"))
}

/// Inputs for a tool-call-classifier (TAC) call, or `None` when TAC should not
/// run: the harness is disabled, or there's no client/session. `None` makes the
/// caller fall back to the ORIGINAL approval behaviour (Normal prompts a risky
/// call, Auto runs it) — the unchanged path when the harness is off. The
/// `Settings` and client `Arc` are cloned out so the caller's `block_on` doesn't
/// hold a borrow of `state`.
pub(super) fn tac_inputs(
    state: &AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
) -> Option<(
    Arc<OpenRouterClient>,
    crate::model::app_config::AppConfig,
    crate::model::settings::Settings,
)> {
    match (client.as_ref(), state.rest.sessions[sess_idx].session.as_ref()) {
        (Some(c), Some(sess)) if sess.settings.classifier_enabled => Some((
            Arc::clone(c),
            state.rest.config.clone(),
            sess.settings.clone(),
        )),
        _ => None,
    }
}

/// True if `target_abs` was read/written/edited earlier in the conversation
/// (i.e. the model has seen or authored its current content). Scans history for
/// read/write/edit tool_calls whose "path" arg resolves to the same abs path.
fn file_known_in_history(
    messages: &[crate::dto::chat::ChatMessage],
    workspaces: &[std::path::PathBuf],
    target_abs: &std::path::Path,
) -> bool {
    for msg in messages {
        let Some(tcs) = msg.tool_calls.as_ref() else { continue };
        for tc in tcs {
            if matches!(tc.function.name.as_str(), "read" | "write" | "edit") {
                let sanitized = crate::dto::chat::sanitize_tool_arguments(&tc.function.arguments);
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sanitized) {
                    if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                        if let Ok(abs) = crate::tool::resolve(workspaces, p) {
                            if abs == target_abs { return true; }
                        }
                    }
                }
            }
        }
    }
    false
}

/// Drive the tool-approval state machine for the current round.
///
/// Walks `pending_tool_calls` from `tool_idx`, running each call and collecting
/// its `(id, result)` into `tool_results`. Non-risky calls always run inline. A
/// risky call (write/edit/delete/bash) is the decision point, and the policy
/// depends on whether the tool-call classifier (TAC) is enabled:
///
/// **Classifier enabled** ([`tac_inputs`] is `Some`) — TAC runs in BOTH modes,
/// intent-aware (it sees the last user message). Per verdict:
/// - available + allow → run the call inline (both modes).
/// - available + block → Auto records a `blocked by harness: <reason>` result
///   and continues the loop WITHOUT a prompt; Normal pauses for `y/n` with the
///   reason.
/// - unavailable (error/timeout) → BOTH modes pause for `y/n` ("classifier
///   unavailable"), degrading to a human decision rather than freezing.
///
/// **Classifier disabled** (`tac_inputs` is `None`) — original behaviour: Normal
/// pauses a risky call for `y/n`; Auto runs it inline.
///
/// A pause sets `awaiting_approval` and returns; the turn is resumed later by
/// [`Action::ApproveTool`] / [`Action::DenyTool`] (which run/deny that one call,
/// advance `tool_idx`, and call back in here). Once every call in the round has
/// resolved it calls [`super::dispatch::finish_tool_round`].
///
/// **Deferred tools.** A call cleared to run whose name is in
/// [`crate::tool::DEFERRED_TOOLS`] (the heavy/blocking ones — read/write/edit/
/// delete/bash/grep/glob/remember/web_fetch/web_search) is NOT run inline:
/// [`super::dispatch::dispatch_deferred`] hands it to a background `std::thread` and PARKS the
/// round. The round's deferred tools run ONE AT A TIME — after dispatching a
/// deferred call we `return` immediately rather than looping, so the next call
/// isn't dispatched until this one's result lands (correctness: two writes to the
/// same file in one round must not race). The event-loop drain folds the result in
/// and the resume gate RE-ENTERS this function at the advanced `tool_idx`, so the
/// loop simply continues. The classifier/approval gate above still runs on the UI
/// thread BEFORE a deferred risky tool is dispatched — deferral happens only after
/// the call is allowed. Instant tools (pong / dir_list / dir_cache_update) still
/// run inline.
///
/// Each call/string is cloned out of `state.rest` before `run_tool` (which
/// borrows `state` mutably) so there's no overlapping borrow of the vec. Reached
/// only from the sync loop, so the `block_on` TAC call is safe.
pub(crate) fn process_tools(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // Recent conversation tail, used to make TAC intent-aware. Cloned out once
    // (empty when there's no session) so the per-call `block_on` below holds no
    // borrow of `state`. We feed the last few User/Assistant turns — NOT just the
    // most-recent user line — because in multi-turn chats that last line is often a
    // terse confirmation ("ok go!", "yes") whose intent only resolves against the
    // earlier request and the agent's proposed plan. 6 messages × 600 chars keeps
    // the safeguard call cheap.
    let convo_context = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|sess| sess.conversation.recent_context(6, 600))
        .unwrap_or_default();
    while state.rest.sessions[sess_idx].tool_idx < state.rest.sessions[sess_idx].pending_tool_calls.len() {
        // re-read every iteration: plan_enter can flip the mode mid-round
        let mode = state.rest.agent_mode;
        let call = state.rest.sessions[sess_idx].pending_tool_calls
            [state.rest.sessions[sess_idx].tool_idx]
            .clone();
        // Intercept the model-callable `plan_enter` tool BEFORE the generic
        // dispatch path (mirrors `cd`): the tool's `run` is pure validation (it
        // always succeeds, no arguments), so the actual mode switch is applied
        // HERE via `set_agent_mode` — the single choke-point shared with `/mode`
        // and Shift+Tab, so `plan_return_mode` + the system-prompt nudge never
        // drift out of sync between entry points. Already-Plan is a no-op (the
        // model gets a friendly "already in plan mode" instead of a spurious
        // re-entry into the same mode).
        if call.function.name == "plan_enter" {
            let result = super::dispatch::run_tool(state, sess_idx, &call);
            let final_result = if result == crate::tool::plan::PLAN_ENTER_SENTINEL {
                if mode == AgentMode::Plan {
                    "already in plan mode".to_string()
                } else {
                    state.rest.set_agent_mode(AgentMode::Plan);
                    "entered plan mode - tools are read-only; explore, structure your \
                     reasoning with seqthink, and call plan_ready with a summary + full \
                     plan when confident"
                        .to_string()
                }
            } else {
                // Not expected (plan_enter has no failure path), but pass through
                // unchanged rather than swallow it.
                result
            };
            state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), final_result));
            state.rest.sessions[sess_idx].tool_idx += 1;
            continue;
        }
        // Intercept the model-callable `plan_ready` tool BEFORE the generic
        // dispatch path (mirrors the `bash_output` FULL interception). Its args
        // carry the entire plan text — too big to round-trip through a sentinel —
        // so they're parsed HERE and `Tool::run` is never called (it's a stub).
        // Outside Plan mode it's a no-op error. In Plan mode we persist the plan
        // to `<session>/plan.md` and PARK the round for the user's y/a/n decision,
        // mirroring the risky-gate pause below: set `awaiting_approval` +
        // `approval_reason` (the summary) + a status line and `return` WITHOUT
        // advancing `tool_idx`, so the resume handlers (`ApprovePlan` /
        // `ApprovePlanCompact` / `DenyPlan`) answer THIS exact call.
        if call.function.name == "plan_ready" {
            if mode != AgentMode::Plan {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    "plan_ready is only available in plan mode".to_string(),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                continue;
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
                continue;
            }
            let sanitized =
                crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
            let args: serde_json::Value =
                serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
            let (summary, plan) = match crate::tool::plan::parse_plan_ready_args(&args) {
                Ok(pair) => pair,
                Err(e) => {
                    state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), e));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    continue;
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
                        continue;
                    }
                }
                None => {
                    state.rest.sessions[sess_idx].tool_results.push((
                        call.id.clone(),
                        "error: could not write plan.md: no active session".to_string(),
                    ));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    continue;
                }
            }
            // Park for the user's decision. Do NOT advance `tool_idx` — the resume
            // handlers answer this exact plan_ready call.
            state.rest.sessions[sess_idx].awaiting_approval = true;
            state.rest.sessions[sess_idx].approval_reason = Some(summary);
            state.rest.sessions[sess_idx].status =
                "plan ready - [y] approve  [a] approve & compact  [n] chat more".to_string();
            return;
        }
        // Plan-mode read-only enforcement (defense-in-depth). The advertise fold
        // (`app::runtime::stream::run`) and the sub-agent allow-list fold
        // (`app::subagent::spawn`) already hide non-whitelisted tools from the
        // model while Plan is active, but a stale/fabricated call name must still
        // be rejected HERE so a model that ignores its own tool list can never
        // mutate anything. `git_operator` is whitelisted at the tool-name level
        // (it's read-only-safe in principle) but additionally filtered by
        // subcommand — Plan may run read git (`log`, `diff`, `status`, …) but not
        // `commit`/`push`/`checkout`/etc. `mcp__*` tools are exempt (the user
        // explicitly wired those servers, so they own that risk — same precedent
        // as `sec_*`'s harness exemption).
        if mode == AgentMode::Plan && !call.function.name.starts_with("mcp__") {
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
                    continue;
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
                continue;
            }
        }
        // Intercept the model-callable `task` tool BEFORE the generic
        // classify/dispatch path: spawn a background sub-agent (never classify it
        // as risky, never await it inline). UNLIKE the generic path, a SUCCESSFUL
        // spawn does NOT push a tool result here — instead it DEFERS, recording the
        // call id in `pending_subagent_calls` so the round parks (below) and the
        // event-loop drain delivers the sub-agent's FULL report as the tool result
        // once it finishes. The main agent then reacts to the real report rather
        // than a fire-and-forget "started" line. A parse error / unknown agent
        // spawns nothing, so it still pushes an IMMEDIATE error result for that call
        // id (keeping the conversation API-valid). Either way `tool_idx` advances so
        // the remaining calls in the round still process.
        if call.function.name == "task" {
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
                        state.rest.sessions[sess_idx].pending_subagent_calls.push(call.id.clone())
                    }
                    // Nothing started or queued (no client/session or unknown
                    // agent) → answer the call now so it isn't left dangling.
                    super::super::spawn::SpawnOutcome::Failed => state.rest.sessions[sess_idx]
                        .tool_results
                        .push((call.id.clone(), format!("error: unknown agent '{agent}'"))),
                }
            }
            state.rest.sessions[sess_idx].tool_idx += 1;
            continue;
        }
        // Intercept a model-callable `bash` call with `run_in_background: true`
        // BEFORE the generic classify/dispatch path: register a background job,
        // spawn it DETACHED, and answer the call IMMEDIATELY with its job id (do
        // NOT park the round). The model then polls the captured output with
        // `bash_output` and stops it with `bash_kill` (both intercepted below).
        // A plain `bash` (no `run_in_background`, or `false`) is left UNTOUCHED —
        // it falls through to the normal approval gate + deferred lane exactly as
        // before, so default behaviour is unchanged.
        if call.function.name == "bash" {
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
                    format!(
                        "started background job bash-{id} (running). Poll with \
                         bash_output{{\"job_id\":\"bash-{id}\"}}, stop with \
                         bash_kill{{\"job_id\":\"bash-{id}\"}}."
                    )
                };
                state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
                state.rest.sessions[sess_idx].tool_idx += 1;
                continue;
            }
            // Not a background bash — fall through to the normal path below.
        }
        // Intercept the model-callable `bash_output` tool: look up the background
        // job in this session's registry and answer synchronously with a status
        // line followed by the captured output so far. Never parks. An unknown id
        // returns an `error:` line surfaced to the model verbatim.
        if call.function.name == "bash_output" {
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
            continue;
        }
        // Intercept the model-callable `bash_kill` tool: find the job and SIGTERM
        // it (best effort), flipping its status to `Killed`. Never parks. An
        // unknown id returns an `error:` line.
        if call.function.name == "bash_kill" {
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
            continue;
        }
        // Intercept the model-callable `task_output` tool (mirrors `bash_output`):
        // look up the detached sub-agent in this session's registry and answer
        // synchronously with a status line + its transcript tail (Running) or its
        // full report (Done) / error / killed note. Never parks. An unknown or
        // missing id returns an `error:` line surfaced to the model verbatim —
        // no guessing fallback, because with up to MAX_SUBAGENTS running in
        // parallel, returning the wrong agent's report is worse than asking.
        if call.function.name == "task_output" {
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
            continue;
        }
        // Intercept the model-callable `task_kill` tool (mirrors `bash_kill`): find
        // the detached sub-agent and abort its tokio task, flipping a still-Running
        // status to `Killed`. Never parks. An unknown id returns an `error:` line.
        if call.function.name == "task_kill" {
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
            continue;
        }
        // Intercept the model-callable `cd` tool BEFORE the generic dispatch path.
        // `cd` must MUTATE session state (the live cwd + dir cache + awareness),
        // which a read-only `ToolCtx` can't do — so the tool's `run` only RESOLVES
        // + validates the target (allow-list-checked) and returns it tagged with
        // `CWD_CHANGE_PREFIX` on success; here we apply the repoint via the shared
        // `apply_workspace_change` primitive and answer the call with a
        // human-readable confirmation. A resolution/validation failure returns a
        // plain `error:`/refusal string, which is surfaced to the model verbatim
        // (the cwd is left unchanged). The path resolution is INSTANT (canonicalize
        // + stat), so running it inline here — not via the deferred lane — is fine.
        // `tool_idx` advances either way so the rest of the round still processes.
        if call.function.name == "cd" {
            let result = super::dispatch::run_tool(state, sess_idx, &call);
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
            continue;
        }
        // Intercept the model-callable `git_cred` tool BEFORE the generic
        // dispatch path. A `select` result tagged with `GIT_CRED_SELECT_PREFIX`
        // must be applied to session settings (persisted) here on the main thread
        // rather than in a side-effect-free `ToolCtx`; a `list` result (or any
        // `error:`) has no such prefix and is surfaced to the model verbatim.
        // `git_cred` is INSTANT (only stat calls) so it runs inline, never via
        // the deferred lane.
        if call.function.name == "git_cred" {
            let result = super::dispatch::run_tool(state, sess_idx, &call);
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
            continue;
        }
        // Intercept the model-callable `git_worktree` tool BEFORE the generic
        // dispatch path. `create`, `remove`, `enter`, and `exit` mutate session
        // state (cwd + allowed roots), which a read-only `ToolCtx` can't do. The
        // tool's `run` does the pure validation/resolution and returns a
        // sentinel-tagged string; here we apply the state change via
        // `apply_workspace_change` (same primitive as `cd`).
        //
        // `create` result: starts with `GIT_WT_CREATE_PREFIX` + shadow path.
        //   → same state work as enter (push path into `settings.workdir`, persist,
        //     apply_workspace_change), but returns a create-specific confirmation so
        //     no model misreads it as a failure.
        // `enter` result: starts with `GIT_WT_ENTER_PREFIX` + shadow path.
        //   → push the path into `settings.workdir` (if not already present),
        //     persist, then call `apply_workspace_change`.
        // `exit` result: exactly `GIT_WT_EXIT_PREFIX`.
        //   → resolve the primary workdir (first `settings.workdir` entry) and
        //     call `apply_workspace_change` to return there.
        // `remove` result: starts with `GIT_WT_REMOVE_PREFIX` + removed shadow path
        // (remove AUTO-EXITS: the worktree is already gone, git ran from the repo root).
        //   → de-register the path from `settings.workdir`; if the live cwd was
        //     inside the removed worktree, snap it back to the primary workdir.
        // Anything else (list output, or an `error:` string):
        //   → pass through to the model verbatim.
        //
        // Borrow structure mirrors the `cd` arm: extract the path string + run
        // `sess.save()` in a scoped block so the `state` borrow is fully released
        // before calling `apply_workspace_change` (which also borrows `state` mutably).
        if call.function.name == "git_worktree" {
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
                        let verdict = handle.block_on(
                            crate::app::harness::classify_toolcall(
                                &c,
                                &config,
                                &settings,
                                &convo_context,
                                &call.function.name,
                                &call.function.arguments,
                            ),
                        );
                        if verdict.available && verdict.allow {
                            // Definite allow. Auto runs inline; Normal still asks.
                            if mode == AgentMode::Normal {
                                state.rest.sessions[sess_idx].approval_reason =
                                    Some(format!("classifier: ok — {}", verdict.reason));
                                state.rest.sessions[sess_idx].awaiting_approval = true;
                                state.rest.sessions[sess_idx].status =
                                    format!("approve {}? [y/n]", call.function.name);
                                return;
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
                                continue;
                            }
                            state.rest.sessions[sess_idx].approval_reason =
                                Some(verdict.reason);
                            state.rest.sessions[sess_idx].awaiting_approval = true;
                            state.rest.sessions[sess_idx].status =
                                format!("approve {}? [y/n]", call.function.name);
                            return;
                        } else {
                            // Classifier unavailable. Normal → human y/n; Auto →
                            // fail-CLOSED (never delete a worktree unverified).
                            if mode == AgentMode::Normal {
                                state.rest.sessions[sess_idx].approval_reason =
                                    Some(verdict.reason.clone());
                                state.rest.sessions[sess_idx].awaiting_approval = true;
                                state.rest.sessions[sess_idx].status =
                                    format!("approve {}? [y/n]", call.function.name);
                                return;
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
                            continue;
                        }
                    }
                    // Classifier disabled → Normal asks, Auto runs.
                    None => {
                        if mode == AgentMode::Normal {
                            state.rest.sessions[sess_idx].awaiting_approval = true;
                            state.rest.sessions[sess_idx].status =
                                format!("approve {}? [y/n]", call.function.name);
                            return;
                        }
                        // Auto + classifier disabled → fall through and run inline.
                    }
                }
            }
            let result = super::dispatch::run_tool(state, sess_idx, &call);
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
            continue;
        }
        // Read-before-edit/overwrite guard: the model must have READ (or written/
        // edited) a file earlier in this conversation before it can `edit` it, or
        // `write`-overwrite an existing file. A write to a brand-new path is always
        // allowed. If the path can't be parsed or resolved we skip the guard and let
        // the tool fail on its own terms.
        if matches!(call.function.name.as_str(), "edit" | "write") {
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
                            continue;
                        }
                    }
                }
            }
        }
        // `sec_*` tools are harness-EXEMPT: security mode is explicit user
        // authorization to test their own target, so the TAC classifier (built to
        // block unrequested mutations) would only block legit offensive steps. The
        // tool's `risk` metadata is still shown in the /security panel as a label.
        //
        // YOLO mode bypasses the harness ENTIRELY: a risky call skips the classifier
        // network call AND every `y/n` prompt, running inline exactly as Auto does on
        // a definite-allow. Only the classifier + prompts are bypassed — the
        // deterministic workspace path guard (WC) inside the tools still applies, so
        // writes stay in the project dir. Gate it FIRST so no `tac_inputs` /
        // `block_on(classify_toolcall)` ever fires in Yolo; clear any stale approval
        // reason and fall through to the dispatch block below (which advances
        // `tool_idx`). The classifier/approval branches below are reached only for the
        // Auto/Normal modes.
        // Yolo + risky: explicit no-op gate. Clear any stale approval reason and fall
        // straight through to dispatch (no classifier, no prompt). Kept as its own
        // branch so the bypass is unmistakable in the control flow.
        if tool_is_risky(&call.function.name) && mode == AgentMode::Yolo {
            state.rest.sessions[sess_idx].approval_reason = None;
        }
        if tool_is_risky(&call.function.name) && mode != AgentMode::Yolo {
            match tac_inputs(state, sess_idx, client) {
                // Classifier enabled → run TAC in both modes and act on its verdict.
                Some((c, config, settings)) => {
                    let verdict = handle.block_on(crate::app::harness::classify_toolcall(
                        &c,
                        &config,
                        &settings,
                        &convo_context,
                        &call.function.name,
                        &call.function.arguments,
                    ));
                    if verdict.available && verdict.allow {
                        // Definite allow. Auto runs it inline (no prompt — the user
                        // delegated decisions); Normal still asks, because in Normal
                        // mode the USER approves every risky op and the classifier
                        // only informs. The allowed reason is surfaced so the prompt
                        // shows the verdict was "ok".
                        // Plan reaches this generic risky gate ONLY for `git_operator`
                        // with an allowed READ subcommand — every other risky tool
                        // (write/edit/delete/bash/web_download, and any git_operator
                        // subcommand outside the read-only list) was already denied
                        // by the Plan read-only gate above, before this point.
                        // Deliberately treated like Auto: TAC-approved read-only git
                        // is harmless, so no prompt is needed.
                        if mode == AgentMode::Auto || mode == AgentMode::Plan {
                            // Fall through and run it inline (no prompt).
                            state.rest.sessions[sess_idx].approval_reason = None;
                        } else {
                            state.rest.sessions[sess_idx].approval_reason =
                                Some(format!("classifier: ok — {}", verdict.reason));
                            state.rest.sessions[sess_idx].awaiting_approval = true;
                            state.rest.sessions[sess_idx].status =
                                format!("approve {}? [y/n]", call.function.name);
                            return;
                        }
                    } else if verdict.available {
                        // Definite block. Auto records it and continues; Normal asks.
                        // Same Plan caveat as the allow-branch above: only a
                        // classifier-blocked git_operator READ subcommand reaches
                        // here in Plan mode, so it is recorded + continued exactly
                        // like Auto rather than prompting.
                        if mode == AgentMode::Auto || mode == AgentMode::Plan {
                            state.rest.sessions[sess_idx].tool_results.push((
                                call.id.clone(),
                                format!("blocked by harness: {}", verdict.reason),
                            ));
                            state.rest.sessions[sess_idx].tool_idx += 1;
                            continue;
                        }
                        state.rest.sessions[sess_idx].approval_reason = Some(verdict.reason);
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status = format!("approve {}? [y/n]", call.function.name);
                        return;
                    } else {
                        // Classifier unavailable. `verdict.reason` carries the REAL
                        // cause (e.g. "classifier not configured …", "classifier
                        // error: 402 …", "classifier timeout") — surface it so the
                        // user sees the actual diagnostic, not a generic string.
                        // Normal: degrade to a human y/n prompt (human decides).
                        // Auto: fail-CLOSED — do NOT run the tool. Push a synthetic
                        //       "not executed" result and bounce to the model, which
                        //       re-decides with the outage surfaced. A classifier
                        //       outage must never let an unverified mutation run.
                        if mode == AgentMode::Normal {
                            state.rest.sessions[sess_idx].approval_reason =
                                Some(verdict.reason.clone());
                            state.rest.sessions[sess_idx].awaiting_approval = true;
                            state.rest.sessions[sess_idx].status =
                                format!("approve {}? [y/n]", call.function.name);
                            return;
                        }
                        // Auto + unavailable → bounce to model (fail-closed).
                        // Do NOT run the tool. Push a synthetic "not executed"
                        // result so finish_tool_round re-streams it to the model,
                        // which re-decides with full context. The mutation never
                        // silently runs when the classifier is down.
                        state.rest.sessions[sess_idx].tool_results.push((
                            call.id.clone(),
                            format!(
                                "not executed: classifier unavailable — {}. \
                                The safety classifier could not verify this call, \
                                so it was NOT run. If the user explicitly requested \
                                this change, tell them to configure or fix the \
                                safeguard classifier in /settings or switch agent \
                                mode; otherwise do not retry.",
                                verdict.reason
                            ),
                        ));
                        state.rest.sessions[sess_idx].tool_idx += 1;
                        state.rest.sessions[sess_idx].set_toast(
                            "harness: classifier unavailable — not run, bounced to model"
                                .to_string(),
                        );
                        continue;
                    }
                }
                // Classifier disabled → original behaviour: Normal asks, Auto runs.
                None => {
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status = format!("approve {}? [y/n]", call.function.name);
                        return;
                    }
                    // Auto + classifier disabled → fall through and run inline.
                }
            }
        }
        // The call has cleared the approval/classifier gate (or was non-risky):
        // dispatch it. Heavy/blocking tools (see `DEFERRED_TOOLS`) run OFF the
        // UI/event-loop thread so the comet keeps sweeping; truly-instant tools run
        // inline. `dispatch_deferred` advances `tool_idx` past this call and
        // registers its id in `pending_tool_tasks`; we then PARK the round
        // IMMEDIATELY by returning (do NOT keep looping). The deferred tools of a
        // round therefore run ONE AT A TIME, in order: the event-loop drain delivers
        // this tool's result, the resume gate re-enters `process_tools`, and the
        // loop continues at the next call. This sequencing is REQUIRED for
        // correctness — two writes/edits to the same file in one round would
        // otherwise race and lose a write.
        // MCP tools (`mcp__<server>__<tool>`) have DYNAMIC names so they can't be
        // listed in `DEFERRED_TOOLS`, but their dispatch blocks the calling thread
        // on a `call_tool` round-trip for up to `CALL_TIMEOUT` (60s) — running that
        // inline would freeze the UI. Route any `mcp__*` call through the SAME
        // off-thread deferred lane as bash/read/web_fetch.
        if crate::tool::DEFERRED_TOOLS.contains(&call.function.name.as_str())
            || call.function.name.starts_with("mcp__")
            || call.function.name.starts_with("sec_")
        {
            super::dispatch::dispatch_deferred(state, sess_idx, &call);
            return;
        }
        // Instant tool: name the tool for the comet phase label and run it inline.
        state.rest.sessions[sess_idx].status = format!("running {}", call.function.name);
        let result = super::dispatch::run_tool(state, sess_idx, &call);
        state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), result));
        state.rest.sessions[sess_idx].tool_idx += 1;
    }
    // Loop exhausted. PARK if there's still deferred work outstanding from this
    // round's `task`-tool sub-agent delegations (`pending_subagent_calls`). A
    // deferred HEAVY tool (`pending_tool_tasks`) parks INSIDE the loop instead —
    // `dispatch_deferred` + an immediate `return` — so it runs sequentially and
    // doesn't reach here; the `has_tool_tasks` arm below is kept only as defensive
    // belt-and-braces. If anything is still in flight, DON'T finish the round — the
    // conversation would have dangling tool_call ids. Mark the round parked and
    // return; the event-loop drains fill each pending result into `tool_results` as
    // it lands, and once BOTH pending lists empty the resume gate re-enters
    // `process_tools` (which eventually reaches `finish_tool_round`). `waiting`
    // stays true and `awaiting_approval` stays false, so the comet keeps shimmering.
    let has_subagents = !state.rest.sessions[sess_idx].pending_subagent_calls.is_empty();
    let has_tool_tasks = !state.rest.sessions[sess_idx].pending_tool_tasks.is_empty();
    if has_subagents || has_tool_tasks {
        if has_subagents {
            state.rest.sessions[sess_idx].awaiting_subagents = true;
        }
        if has_tool_tasks {
            state.rest.sessions[sess_idx].awaiting_tool_tasks = true;
        }
        // Status: prefer the delegation message when sub-agents are pending (its
        // existing wording is unchanged); otherwise show the fetch is in flight.
        if has_subagents {
            let n = state.rest.sessions[sess_idx].pending_subagent_calls.len();
            state.rest.sessions[sess_idx].status = if n == 1 {
                "delegating… (1 sub-agent)".into()
            } else {
                format!("delegating… ({n} sub-agents)")
            };
        } else {
            state.rest.sessions[sess_idx].status = "fetching…".into();
        }
        return;
    }
    super::dispatch::finish_tool_round(state, sess_idx, client, handle);
}

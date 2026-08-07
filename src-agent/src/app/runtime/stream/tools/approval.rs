//! Tool-approval state machine: classify, run, deny, finish tool rounds.
//! Includes risky-tool detection, TAC (tool-call classifier) inputs, and the
//! main `process_tools` loop that drives approval/dispatch for each tool call.

use std::sync::Arc;

use crate::app::state::AgentMode;
use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use crate::service::openrouter::OpenRouterClient;

use super::intercepts::{self, InterceptFlow};

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
pub(super) fn parse_bash_id(job_id: &str) -> Option<usize> {
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
pub(super) fn parse_subagent_id(v: &serde_json::Value) -> Option<usize> {
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
pub(super) fn bash_status_line(status: &crate::app::bgbash::BashJobStatus) -> String {
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
pub(super) fn filter_bash_output(
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
/// `Settings` and client `Arc` are cloned out so the caller can move them into
/// the off-thread classify task ([`spawn_classify_park`]) without borrowing `state`.
pub(super) fn tac_inputs(
    state: &AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
) -> Option<(
    Arc<OpenRouterClient>,
    crate::model::app_config::AppConfig,
    crate::model::settings::Settings,
)> {
    match (
        client.as_ref(),
        state.rest.sessions[sess_idx].session.as_ref(),
    ) {
        (Some(c), Some(sess)) if sess.settings.classifier_enabled => Some((
            Arc::clone(c),
            state.rest.config.clone(),
            sess.settings.clone(),
        )),
        _ => None,
    }
}

/// Spawn the TAC classifier for `call` OFF the event-loop thread and PARK the
/// round on it. Lazily opens this session's verdict channel, fires the classify
/// task (which sends `(call_id, verdict)` back over it once done), latches
/// `awaiting_classify`, and sets the "classifying …" status. The caller MUST
/// `return` right after so the round stays parked — the event-loop drain stages
/// the verdict and re-enters `process_tools`, which then acts on it via the
/// unchanged three-way verdict branch. This replaces the old synchronous
/// `handle.block_on(classify_toolcall(..))`, which froze the whole event loop
/// (every session, in the daemon) for the 1-12s of the classify call.
///
/// The inputs are cheap-cloned owned values (Arc client / AppConfig / Settings /
/// Strings), so the spawned future is `Send + 'static` — mirroring the advisory
/// prompt-classifier spawn in `actions::chat::handle_submit`. `classify_toolcall`
/// is pure async HTTP with its own internal timeout, so `handle.spawn` (a tokio
/// task) is correct here (NOT a `std::thread` — there is no `reqwest::blocking`).
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_classify_park(
    state: &mut AppState,
    sess_idx: usize,
    handle: &tokio::runtime::Handle,
    client: Arc<OpenRouterClient>,
    config: crate::model::app_config::AppConfig,
    settings: crate::model::settings::Settings,
    convo_context: &str,
    call: &ToolCall,
) {
    // Lazily create THIS session's verdict channel once, then reuse it (mirrors
    // the deferred tool-task channel). Both ends live on the session; the task
    // fires over `classify_tx`, the event-loop drain reads `classify_rx`.
    if state.rest.sessions[sess_idx].classify_tx.is_none() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.rest.sessions[sess_idx].classify_tx = Some(tx);
        state.rest.sessions[sess_idx].classify_rx = Some(rx);
    }
    let Some(tx) = state.rest.sessions[sess_idx].classify_tx.as_ref().cloned() else {
        crate::model::store::append_global_error_log("approval", "BUG: classify_tx missing");
        return;
    };
    let convo = convo_context.to_string();
    let name = call.function.name.clone();
    let args = call.function.arguments.clone();
    let call_id = call.id.clone();
    handle.spawn(async move {
        let verdict = crate::app::harness::classify_toolcall(
            &client, &config, &settings, &convo, &name, &args,
        )
        .await;
        // A dropped receiver (turn interrupted / session closed) makes this a
        // no-op — same contract as the streaming + PC channels.
        let _ = tx.send((call_id, verdict));
    });
    state.rest.sessions[sess_idx].awaiting_classify = true;
    state.rest.sessions[sess_idx].status = format!("classifying {}…", call.function.name);
}

/// True if `target_abs` was read/written/edited earlier in the conversation
/// (i.e. the model has seen or authored its current content). Scans history for
/// read/write/edit tool_calls whose "path" arg resolves to the same abs path.
pub(super) fn file_known_in_history(
    messages: &[crate::dto::chat::ChatMessage],
    workspaces: &[std::path::PathBuf],
    target_abs: &std::path::Path,
) -> bool {
    for msg in messages {
        let Some(tcs) = msg.tool_calls.as_ref() else {
            continue;
        };
        for tc in tcs {
            if matches!(tc.function.name.as_str(), "read" | "write" | "edit") {
                let sanitized = crate::dto::chat::sanitize_tool_arguments(&tc.function.arguments);
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sanitized) {
                    if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                        if let Ok(abs) = crate::tool::resolve(workspaces, p) {
                            if abs == target_abs {
                                return true;
                            }
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
pub(crate) fn process_tools(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // Recent conversation tail, used to make TAC intent-aware — see
    // `intercepts::build_convo_context` for the plan-aware preamble.
    let convo_context = intercepts::build_convo_context(state, sess_idx);
    while state.rest.sessions[sess_idx].tool_idx
        < state.rest.sessions[sess_idx].pending_tool_calls.len()
    {
        // re-read every iteration: plan_enter can flip the mode mid-round
        let mode = state.rest.sessions[sess_idx].agent_mode;
        let call = state.rest.sessions[sess_idx].pending_tool_calls
            [state.rest.sessions[sess_idx].tool_idx]
            .clone();
        if call.function.name == "plan_enter" {
            match intercepts::intercept_plan_enter(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "plan_ready" {
            match intercepts::intercept_plan_ready(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if mode == AgentMode::Plan && call.function.name == "checklist" {
            match intercepts::intercept_checklist_plan(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if mode == AgentMode::Plan && !call.function.name.starts_with("mcp__") {
            match intercepts::intercept_plan_readonly_gate(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // SDLC assess: deny filesystem-mutating workspace tools at runtime
        // (same pattern as Plan's readonly gate). mission_ready / checklist /
        // read-search remain available so the contract can be prepared.
        if mode == AgentMode::Sdlc
            && state.rest.sessions[sess_idx].sdlc_phase.as_deref() == Some("assess")
            && !call.function.name.starts_with("mcp__")
        {
            match intercepts::intercept_sdlc_assess_gate(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Never force-push in ANY SDLC phase (including done/paused).
        if mode == AgentMode::Sdlc && call.function.name == "git_operator" {
            let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
            let args: serde_json::Value =
                serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
            let git_args: Vec<&str> = args
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect::<Vec<&str>>())
                .unwrap_or_default();
            if let Some(reason) = crate::tool::sdlc_git_force_push_denied(&git_args) {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!("error: Never force-push in SDLC ({reason})."),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                continue;
            }
        }
        // SDLC execute/integrate: confine git_operator to the frozen bound
        // worktree (no cwd override, no checkout/switch, binding must be live).
        if mode == AgentMode::Sdlc
            && matches!(
                state.rest.sessions[sess_idx].sdlc_phase.as_deref(),
                Some("execute") | Some("integrate")
            )
            && call.function.name == "git_operator"
        {
            match intercepts::intercept_sdlc_execute_git_gate(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept `mission_ready` BEFORE the generic dispatch path. Only when mode == Sdlc.
        if call.function.name == "mission_ready" {
            match intercepts::intercept_mission_ready(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept `mission_verify` BEFORE the generic dispatch path. Only when mode == Sdlc.
        if call.function.name == "mission_verify" {
            match intercepts::intercept_mission_verify(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept `mission_integrate` BEFORE the generic dispatch path. Only when mode == Sdlc.
        if call.function.name == "mission_integrate" {
            match intercepts::intercept_mission_integrate(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept `checklist` WHILE IN SDLC MODE before the generic dispatch path.
        if mode == AgentMode::Sdlc && call.function.name == "checklist" {
            match intercepts::intercept_checklist_sdlc(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "task" {
            match intercepts::intercept_task(state, sess_idx, &call, client, handle) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "bash" {
            match intercepts::intercept_bash_background(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "bash_output" {
            match intercepts::intercept_bash_output(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "bash_kill" {
            match intercepts::intercept_bash_kill(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "task_output" {
            match intercepts::intercept_task_output(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "task_kill" {
            match intercepts::intercept_task_kill(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "task_send" {
            match intercepts::intercept_task_send(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "cd" {
            match intercepts::intercept_cd(state, sess_idx, &call, client, handle) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "git_cred" {
            match intercepts::intercept_git_cred(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "skill" {
            match intercepts::intercept_skill(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if call.function.name == "git_worktree" {
            match intercepts::intercept_git_worktree(
                state,
                sess_idx,
                &call,
                mode,
                client,
                handle,
                &convo_context,
            ) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // SDLC execute/integrate: reject write/edit/delete to paths owned by a
        // DIFFERENT active node (hard path-ownership enforcement via glob matching).
        if mode == AgentMode::Sdlc
            && matches!(
                state.rest.sessions[sess_idx].sdlc_phase.as_deref(),
                Some("execute") | Some("integrate")
            )
            && matches!(call.function.name.as_str(), "write" | "edit" | "delete")
        {
            match intercepts::intercept_sdlc_path_ownership_gate(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if matches!(call.function.name.as_str(), "edit" | "write") {
            match intercepts::intercept_read_before_edit_guard(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        if tool_is_risky(&call.function.name) && mode == AgentMode::Yolo {
            state.rest.sessions[sess_idx].approval_reason = None;
        }
        if tool_is_risky(&call.function.name) && mode != AgentMode::Yolo {
            match tac_inputs(state, sess_idx, client) {
                Some((c, config, settings)) => {
                    let verdict = match state.rest.sessions[sess_idx]
                        .pending_classify_verdict
                        .take()
                    {
                        Some((vid, v)) if vid == call.id => v,
                        _ => {
                            spawn_classify_park(
                                state,
                                sess_idx,
                                handle,
                                c,
                                config,
                                settings,
                                &convo_context,
                                &call,
                            );
                            return;
                        }
                    };
                    if verdict.available && verdict.allow {
                        if mode == AgentMode::Auto
                            || mode == AgentMode::Plan
                            || mode == AgentMode::Sdlc
                        {
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
                        if mode == AgentMode::Auto
                            || mode == AgentMode::Plan
                            || mode == AgentMode::Sdlc
                        {
                            state.rest.sessions[sess_idx].tool_results.push((
                                call.id.clone(),
                                format!("blocked by harness: {}", verdict.reason),
                            ));
                            state.rest.sessions[sess_idx].tool_idx += 1;
                            continue;
                        }
                        state.rest.sessions[sess_idx].approval_reason = Some(verdict.reason);
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status =
                            format!("approve {}? [y/n]", call.function.name);
                        return;
                    } else {
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
                None => {
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status =
                            format!("approve {}? [y/n]", call.function.name);
                        return;
                    }
                }
            }
        }
        if crate::tool::DEFERRED_TOOLS.contains(&call.function.name.as_str())
            || call.function.name.starts_with("mcp__")
            || call.function.name.starts_with("sec_")
        {
            super::dispatch::dispatch_deferred(state, sess_idx, &call);
            return;
        }
        state.rest.sessions[sess_idx].status = format!("running {}", call.function.name);
        let result = super::dispatch::run_tool(state, sess_idx, &call);
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), result));
        state.rest.sessions[sess_idx].tool_idx += 1;
    }
    let has_subagents = !state.rest.sessions[sess_idx]
        .pending_subagent_calls
        .is_empty();
    let has_tool_tasks = !state.rest.sessions[sess_idx].pending_tool_tasks.is_empty();
    if has_subagents || has_tool_tasks {
        if has_subagents {
            state.rest.sessions[sess_idx].awaiting_subagents = true;
        }
        if has_tool_tasks {
            state.rest.sessions[sess_idx].awaiting_tool_tasks = true;
        }
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

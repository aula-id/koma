//! Tool-approval state machine: classify, run, deny, finish tool rounds.
//! Includes risky-tool detection, TAC (tool-call classifier) inputs, and the
//! main `process_tools` loop that drives approval/dispatch for each tool call.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::app::state::AgentMode;
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
    match (client.as_ref(), state.rest.sessions[sess_idx].session.as_ref()) {
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
    let tx = state.rest.sessions[sess_idx]
        .classify_tx
        .as_ref()
        .unwrap()
        .clone();
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
/// loop simply continues. The classifier/approval gate above decides BEFORE a
/// deferred risky tool is dispatched — deferral happens only after the call is
/// allowed. Instant tools (pong / dir_list / dir_cache_update) still run inline.
///
/// **Async classifier gate.** The TAC classify call is NOT run inline: it would
/// freeze the event loop 1-12s per risky call. Instead the gate spawns the
/// classify off-thread ([`spawn_classify_park`]) and PARKS the round on
/// `awaiting_classify`; the event-loop drain stages the verdict into
/// `pending_classify_verdict` and RE-ENTERS this function, which consumes it and
/// acts on the unchanged three-way branch. So a risky-tool round can park twice —
/// first on the classifier, then (in Normal, or on a deferred allow) on approval /
/// the deferred lane.
///
/// Each call/string is cloned out of `state.rest` before `run_tool` (which
/// borrows `state` mutably) so there's no overlapping borrow of the vec.
pub(crate) fn process_tools(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // Recent conversation tail, used to make TAC intent-aware — see
    // `intercepts::build_convo_context` for the plan-aware preamble.
    let convo_context = intercepts::build_convo_context(state, sess_idx);
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
            match intercepts::intercept_plan_enter(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept the model-callable `plan_ready` tool BEFORE the generic
        // dispatch path (mirrors the `bash_output` FULL interception). Its args
        // carry the entire plan text — too big to round-trip through a sentinel —
        // so they're parsed HERE and `Tool::run` is never called (it's a stub).
        // Outside Plan mode it's a no-op error. In Plan mode we persist the plan
        // to `<session>/plan.md`, COMPOSE the user-facing digest (checklist + full
        // plan when short, else checklist + highlights) and swap it into the stored
        // tool-call args so the transcript renders it, then PARK the round for the
        // user's y/a/n decision, mirroring the risky-gate pause below: set
        // `awaiting_approval` + a status line and `return` WITHOUT advancing
        // `tool_idx`, so the resume handlers (`ApprovePlan` / `ApprovePlanCompact` /
        // `DenyPlan`) answer THIS exact call.
        if call.function.name == "plan_ready" {
            match intercepts::intercept_plan_ready(state, sess_idx, &call, mode) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept `checklist` WHILE IN PLAN MODE before the generic dispatch path
        // (mirrors the `plan_ready` full interception): the model manages the plan
        // checklist, but the two locked rails are auto-appended and the per-directory
        // `memory/TODO.md` is NOT touched. Outside Plan mode `checklist` is left
        // UNTOUCHED — it falls through to its normal writer. Session-scoped: this
        // resolves THIS call's session (`sess_idx`), never a foreground assumption.
        if mode == AgentMode::Plan && call.function.name == "checklist" {
            match intercepts::intercept_checklist_plan(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
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
            match intercepts::intercept_plan_readonly_gate(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
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
            match intercepts::intercept_task(state, sess_idx, &call, client, handle) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
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
            match intercepts::intercept_bash_background(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept the model-callable `bash_output` tool: look up the background
        // job in this session's registry and answer synchronously with a status
        // line followed by the captured output so far. Never parks. An unknown id
        // returns an `error:` line surfaced to the model verbatim.
        if call.function.name == "bash_output" {
            match intercepts::intercept_bash_output(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept the model-callable `bash_kill` tool: find the job and SIGTERM
        // it (best effort), flipping its status to `Killed`. Never parks. An
        // unknown id returns an `error:` line.
        if call.function.name == "bash_kill" {
            match intercepts::intercept_bash_kill(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept the model-callable `task_output` tool (mirrors `bash_output`):
        // look up the detached sub-agent in this session's registry and answer
        // synchronously with a status line + its transcript tail (Running) or its
        // full report (Done) / error / killed note. Never parks. An unknown or
        // missing id returns an `error:` line surfaced to the model verbatim —
        // no guessing fallback, because with up to MAX_SUBAGENTS running in
        // parallel, returning the wrong agent's report is worse than asking.
        if call.function.name == "task_output" {
            match intercepts::intercept_task_output(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept the model-callable `task_kill` tool (mirrors `bash_kill`): find
        // the detached sub-agent and abort its tokio task, flipping a still-Running
        // status to `Killed`. Never parks. An unknown id returns an `error:` line.
        if call.function.name == "task_kill" {
            match intercepts::intercept_task_kill(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
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
            match intercepts::intercept_cd(state, sess_idx, &call, client, handle) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Intercept the model-callable `git_cred` tool BEFORE the generic
        // dispatch path. A `select` result tagged with `GIT_CRED_SELECT_PREFIX`
        // must be applied to session settings (persisted) here on the main thread
        // rather than in a side-effect-free `ToolCtx`; a `list` result (or any
        // `error:`) has no such prefix and is surfaced to the model verbatim.
        // `git_cred` is INSTANT (only stat calls) so it runs inline, never via
        // the deferred lane.
        if call.function.name == "git_cred" {
            match intercepts::intercept_git_cred(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
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
            match intercepts::intercept_git_worktree(state, sess_idx, &call, mode, client, handle, &convo_context) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
            }
        }
        // Read-before-edit/overwrite guard: the model must have READ (or written/
        // edited) a file earlier in this conversation before it can `edit` it, or
        // `write`-overwrite an existing file. A write to a brand-new path is always
        // allowed. If the path can't be parsed or resolved we skip the guard and let
        // the tool fail on its own terms.
        if matches!(call.function.name.as_str(), "edit" | "write") {
            match intercepts::intercept_read_before_edit_guard(state, sess_idx, &call) {
                InterceptFlow::Continue => continue,
                InterceptFlow::Return => return,
                InterceptFlow::Fallthrough => {}
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
        // `spawn_classify_park` ever fires in Yolo; clear any stale approval
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
                    // Async TAC gate: consume a verdict the event-loop drain already
                    // staged for THIS call, else spawn the classifier off-thread and
                    // PARK the round (returning immediately) — the drain re-enters
                    // here with the verdict once it lands. A staged verdict for a
                    // DIFFERENT id is stale (interrupted/superseded turn): drop it and
                    // classify fresh. The three-way branch below is UNCHANGED.
                    let verdict = match state.rest.sessions[sess_idx]
                        .pending_classify_verdict
                        .take()
                    {
                        Some((vid, v)) if vid == call.id => v,
                        _ => {
                            spawn_classify_park(
                                state, sess_idx, handle, c, config, settings,
                                &convo_context, &call,
                            );
                            return;
                        }
                    };
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

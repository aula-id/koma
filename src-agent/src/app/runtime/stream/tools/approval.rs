//! Tool-approval state machine: classify, run, deny, finish tool rounds.
//! Includes risky-tool detection, TAC (tool-call classifier) inputs, and the
//! main `process_tools` loop that drives approval/dispatch for each tool call.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use crate::service::openrouter::OpenRouterClient;
use crate::app::state::AgentMode;

/// True for tools that mutate the workspace (or run arbitrary shell commands)
/// and therefore require approval in Normal mode. Deterministic, name-based —
/// no classifier / network call.
pub(super) fn tool_is_risky(name: &str) -> bool {
    matches!(name, "write" | "delete" | "edit" | "bash")
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
    let mode = state.rest.agent_mode;
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
        let call = state.rest.sessions[sess_idx].pending_tool_calls
            [state.rest.sessions[sess_idx].tool_idx]
            .clone();
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
            if agent.is_empty() || prompt.is_empty() {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    "error: task requires non-empty 'agent' and 'prompt'".to_string(),
                ));
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
        // dispatch path. `enter` and `exit` mutate session state (cwd + allowed
        // roots), which a read-only `ToolCtx` can't do. The tool's `run` does the
        // pure validation/resolution and returns a sentinel-tagged string; here we
        // apply the state change via `apply_workspace_change` (same primitive as `cd`).
        //
        // `enter` result: starts with `GIT_WT_ENTER_PREFIX` + canonical path.
        //   → push the path into `settings.workdir` (if not already present),
        //     persist, then call `apply_workspace_change`.
        // `exit` result: exactly `GIT_WT_EXIT_PREFIX`.
        //   → resolve the primary workdir (first `settings.workdir` entry) and
        //     call `apply_workspace_change` to return there.
        // Anything else (list/create/remove output, or an `error:` string):
        //   → pass through to the model verbatim.
        //
        // Borrow structure mirrors the `cd` arm: extract the path string + run
        // `sess.save()` in a scoped block so the `state` borrow is fully released
        // before calling `apply_workspace_change` (which also borrows `state` mutably).
        if call.function.name == "git_worktree" {
            let result = super::dispatch::run_tool(state, sess_idx, &call);
            let final_result =
                if let Some(target) =
                    result.strip_prefix(crate::tool::git_worktree::GIT_WT_ENTER_PREFIX)
                {
                    // `enter` succeeded: target is the canonical path string.
                    let new_cwd = std::path::PathBuf::from(target);
                    let target_str = target.to_string();
                    // Push the new root into settings.workdir if not already there,
                    // then persist. Scoped so the mutable sess borrow ends before
                    // we call apply_workspace_change (which also borrows state mut).
                    {
                        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                            if !sess.settings.workdir.contains(&target_str) {
                                sess.settings.workdir.push(target_str.clone());
                            }
                            let _ = sess.save();
                        }
                    }
                    super::super::spawn::apply_workspace_change(
                        state, sess_idx, new_cwd.clone(), client, handle,
                    );
                    format!("entered worktree: {}", new_cwd.display())
                } else if result.starts_with(crate::tool::git_worktree::GIT_WT_EXIT_PREFIX) {
                    // `exit`: return to the primary workdir (first workdir entry).
                    // Extract the primary path in a scoped borrow, then call
                    // apply_workspace_change outside it.
                    let primary = {
                        state.rest.sessions[sess_idx]
                            .session
                            .as_ref()
                            .map(|sess| sess.workdir())
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                    };
                    super::super::spawn::apply_workspace_change(
                        state, sess_idx, primary.clone(), client, handle,
                    );
                    format!("exited to {}", primary.display())
                } else {
                    // list/create/remove output, or an error: — pass through.
                    result
                };
            state.rest.sessions[sess_idx].tool_results.push((call.id.clone(), final_result));
            state.rest.sessions[sess_idx].tool_idx += 1;
            continue;
        }
        if tool_is_risky(&call.function.name) {
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
                        if mode == AgentMode::Auto {
                            // Fall through and run it inline (no prompt).
                            state.rest.sessions[sess_idx].approval_reason = None;
                        } else {
                            state.rest.sessions[sess_idx].approval_reason =
                                Some(format!("classifier: ok — {}", verdict.reason));
                            state.rest.sessions[sess_idx].awaiting_approval = true;
                            state.rest.status =
                                format!("approve {}? [y/n]", call.function.name);
                            return;
                        }
                    } else if verdict.available {
                        // Definite block. Auto records it and continues; Normal asks.
                        if mode == AgentMode::Auto {
                            state.rest.sessions[sess_idx].tool_results.push((
                                call.id.clone(),
                                format!("blocked by harness: {}", verdict.reason),
                            ));
                            state.rest.sessions[sess_idx].tool_idx += 1;
                            continue;
                        }
                        state.rest.sessions[sess_idx].approval_reason = Some(verdict.reason);
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.status = format!("approve {}? [y/n]", call.function.name);
                        return;
                    } else {
                        // Classifier unavailable. `verdict.reason` now carries the
                        // REAL cause (e.g. "classifier error: 402 …", "classifier
                        // timeout", "unparseable verdict: …") — surface it so the
                        // user sees the actual diagnostic, not a generic string.
                        // Normal: degrade to a human y/n prompt (human decides).
                        // Auto: fail-open — user has delegated decisions; a
                        //       classifier outage must not halt or interrupt them.
                        //       Run inline and surface a toast so the degradation
                        //       is visible.
                        if mode == AgentMode::Normal {
                            state.rest.sessions[sess_idx].approval_reason =
                                Some(verdict.reason.clone());
                            state.rest.sessions[sess_idx].awaiting_approval = true;
                            state.rest.status =
                                format!("approve {}? [y/n]", call.function.name);
                            return;
                        }
                        // Auto + unavailable → run inline, no prompt.
                        state.rest.set_toast(format!(
                            "harness: {} — auto-ran {}",
                            verdict.reason, call.function.name
                        ));
                        // fall through to run_tool below
                    }
                }
                // Classifier disabled → original behaviour: Normal asks, Auto runs.
                None => {
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.status = format!("approve {}? [y/n]", call.function.name);
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
        {
            super::dispatch::dispatch_deferred(state, sess_idx, &call);
            return;
        }
        // Instant tool: name the tool for the comet phase label and run it inline.
        state.rest.status = format!("running {}", call.function.name);
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
            state.rest.status = if n == 1 {
                "delegating… (1 sub-agent)".into()
            } else {
                format!("delegating… ({n} sub-agents)")
            };
        } else {
            state.rest.status = "fetching…".into();
        }
        return;
    }
    super::dispatch::finish_tool_round(state, sess_idx, client, handle);
}

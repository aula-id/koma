//! Tool dispatch: deferred/off-thread execution, inline run, round finalization,
//! resume after delegations, and deny-all-pending for workspace rejections.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::dto::chat::{Role, ToolCall};
use crate::service::openrouter::OpenRouterClient;

/// Run a single tool call against the session workspace and return its result
/// string (an `error: …` line on failure / unknown tool). Reads the session for
/// the workspace path and clones the shared dir cache up front, then dispatches
/// to the matching [`crate::tool::Tool`].
///
/// `pub(crate)` so the approve/deny action handlers can run a single tool when
/// resuming the approval machine.
pub(crate) fn run_tool(state: &mut AppState, sess_idx: usize, call: &ToolCall) -> String {
    let ctx = super::super::spawn::build_tool_ctx(state, sess_idx);
    crate::tool::execute_tool(&ctx, call)
}

/// Dispatch a single DEFERRED (heavy/blocking) tool OFF the UI/event-loop thread
/// and register it as pending, advancing `tool_idx` past it. The caller MUST
/// `return` right after (parking the round) so the round's deferred tools run
/// SEQUENTIALLY: this one finishes, the event-loop drain folds its result into
/// `tool_results` + drops its id, and the resume gate re-enters `process_tools`
/// to handle the next call.
///
/// `pub(crate)` so the `ApproveTool` handler can defer an approved risky tool the
/// same way (rather than running it inline on the UI thread and re-freezing the
/// comet during, e.g., a large approved write).
///
/// The work runs on a PLAIN `std::thread` (NOT a tokio task): the network tools'
/// internal `reqwest::blocking` work would panic inside a tokio runtime context,
/// so the worker must have none. `ToolCtx` is Send + 'static (PathBuf / Vec / Arc
/// / Option fields, no borrows) so it moves in cleanly, and the `UnboundedSender`
/// is Send so it can fire from this off-runtime thread. The result channel is
/// created lazily once per session, then reused.
pub(crate) fn dispatch_deferred(state: &mut AppState, sess_idx: usize, call: &ToolCall) {
    // Lazily create THIS session's result channel once, then reuse it. The
    // spawned thread fires back over session `sess_idx`'s own `tool_task_tx`, so
    // the result is routed structurally to that session's drain (no id tag
    // needed) regardless of which session is foreground when it lands.
    if state.rest.sessions[sess_idx].tool_task_tx.is_none() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.rest.sessions[sess_idx].tool_task_tx = Some(tx);
        state.rest.sessions[sess_idx].tool_task_rx = Some(rx);
    }
    let ctx = super::super::spawn::build_tool_ctx(state, sess_idx);
    let call_cloned = call.clone();
    let id = call.id.clone();
    let tx = state.rest.sessions[sess_idx].tool_task_tx.as_ref().unwrap().clone();
    // Phase label for the comet: name the tool running off-thread so the
    // shimmering status surfaces what the agent is doing while it's parked.
    state.rest.status = format!("running {}", call.function.name);
    std::thread::spawn(move || {
        let result = crate::tool::execute_tool(&ctx, &call_cloned);
        let _ = tx.send((id, result));
    });
    state.rest.sessions[sess_idx].pending_tool_tasks.push(call.id.clone());
    state.rest.sessions[sess_idx].tool_idx += 1;
    // Mark the round PARKED on async tool work so the event-loop resume gate
    // (which requires this flag set AND `pending_tool_tasks` empty) fires once the
    // result lands. The caller `return`s right after this, leaving the round
    // parked; `waiting` stays true so the comet keeps shimmering.
    state.rest.sessions[sess_idx].awaiting_tool_tasks = true;
}

/// Finish a completed tool round: flush every collected result into the
/// conversation + log, clear the machine, and re-stream so the model sees the
/// tool outputs and continues the turn (`waiting` stays true throughout).
///
/// Bails cleanly if there is no session or client to continue against
/// (defensive — a turn in flight normally implies both are present).
pub(super) fn finish_tool_round(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // Push the collected tool results into the conversation + log them. Bind the
    // session runtime once so `session` (mut) + `tool_results` (read) are
    // disjoint field borrows of the same `SessionRuntime`.
    {
        let rt = &mut state.rest.sessions[sess_idx];
        if let Some(sess) = rt.session.as_mut() {
            for (id, result) in &rt.tool_results {
                let _ = crate::model::msglog::append(&sess.path, Role::Tool, result, None);
                sess.conversation.push_tool(id.clone(), result.clone());
            }
            let _ = sess.save();
        }
    }

    // Live reload: if `remember` or `forget` ran this round, re-inject the updated
    // MEMORY.md into messages[0] so the model sees the change immediately.
    // (`recall` is read-only and must NOT trigger a rebuild.)
    let memory_mutated = state.rest.sessions[sess_idx]
        .pending_tool_calls
        .iter()
        .any(|c| matches!(c.function.name.as_str(), "remember" | "forget"));
    if memory_mutated {
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            sess.rebuild_system();
        }
    }

    // Round done: clear the per-round machine before the next model call.
    state.rest.sessions[sess_idx].pending_tool_calls.clear();
    state.rest.sessions[sess_idx].tool_idx = 0;
    state.rest.sessions[sess_idx].tool_results.clear();

    // Continue the turn: hand the updated history back to the model. The
    // streaming buffer is re-armed so the next assistant text accumulates
    // cleanly. `waiting` stays true (the turn isn't finished yet). Compute the
    // history into an owned Option FIRST so no session borrow is held across the
    // per-session writes in the no-session arm.
    let history = match (state.rest.sessions[sess_idx].session.as_ref(), client.as_ref()) {
        (Some(sess), Some(_)) => Some(sess.conversation.history()),
        _ => None,
    };
    let Some(history) = history else {
        state.rest.sessions[sess_idx].waiting = false;
        state.rest.sessions[sess_idx].current_task = None;
        state.rest.sessions[sess_idx].agent_steps = 0;
        state.rest.status = "no active session".into();
        return;
    };
    // The tool round is done; this re-stream is a model wait, so label it the same
    // "thinking" phase the comet sweeps (not a tool run).
    state.rest.status = "thinking".into();
    state.rest.sessions[sess_idx].begin_stream();
    super::super::run::start_stream_task(history, state, sess_idx, client, handle);
}

/// Resume a tool round that was PARKED on deferred work — either `task`-tool
/// sub-agent delegations (`pending_subagent_calls`) or a deferred heavy tool
/// (`pending_tool_tasks`).
///
/// Called from the event-loop resume gate once BOTH deferred lists are empty
/// (every parked id has had its result folded into `tool_results`). It simply
/// RE-ENTERS [`super::approval::process_tools`] at the current `tool_idx` to CONTINUE the round:
/// - For a deferred heavy tool, exactly one call was dispatched before the park,
///   so re-entry processes the NEXT call (and may dispatch+park again). The round
///   advances one deferred tool per resume, in order.
/// - For `task`-tool delegations the round had already walked every call before
///   parking (`tool_idx == len`), so re-entry finds the loop exhausted.
///
/// In both cases, when `process_tools` reaches the end of the round with no
/// further deferred work it falls through to [`finish_tool_round`], which flushes
/// all collected `tool_results` and re-streams — the main agent now sees every
/// result and reacts. Re-entering (rather than calling `finish_tool_round`
/// directly) is what makes the deferred lane SEQUENTIAL.
pub(crate) fn resume_after_subagents(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    super::approval::process_tools(state, sess_idx, client, handle);
}

/// Halt the current turn by answering every still-pending tool call with
/// `reason` (and flushing any results already collected this round), so the
/// stored conversation keeps every `tool_call` id answered — then reset the
/// agentic-loop machine and end the turn WITHOUT re-streaming.
///
/// Shares the shape of [`super::super::actions`]'s `DenyTool` handler; used by the
/// harness workspace check (WC) to refuse a turn whose workspace isn't allowed.
/// Pending calls from `tool_idx` onward are the unanswered ones.
pub(crate) fn deny_all_pending(state: &mut AppState, sess_idx: usize, reason: &str) {
    let results = state.rest.sessions[sess_idx].tool_results.clone();
    let pending_ids: Vec<String> = state.rest.sessions[sess_idx]
        .pending_tool_calls
        .iter()
        .skip(state.rest.sessions[sess_idx].tool_idx)
        .map(|c| c.id.clone())
        .collect();
    if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
        for (id, result) in &results {
            let _ = crate::model::msglog::append(&sess.path, Role::Tool, result, None);
            sess.conversation.push_tool(id.clone(), result.clone());
        }
        for id in &pending_ids {
            let _ = crate::model::msglog::append(&sess.path, Role::Tool, reason, None);
            sess.conversation.push_tool(id.clone(), reason.to_string());
        }
        let _ = sess.save();
    }
    let rt = &mut state.rest.sessions[sess_idx];
    rt.pending_tool_calls.clear();
    rt.tool_idx = 0;
    rt.tool_results.clear();
    rt.agent_steps = 0;
    rt.awaiting_approval = false;
    rt.approval_reason = None;
    rt.waiting = false;
    rt.current_task = None;
    // Kill every running sub-agent and drop the pending queue so a killed WC
    // turn can't ghost-restart via orphaned tasks or stale awaiting flags.
    rt.abort_running_subagents();
    rt.pending_tool_tasks.clear();
    rt.awaiting_tool_tasks = false;
}

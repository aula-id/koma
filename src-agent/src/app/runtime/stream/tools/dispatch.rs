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
    state.rest.sessions[sess_idx].status = format!("running {}", call.function.name);
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
    // Drain any MEDIA_WORKDIR: workdir requests from web_download results FIRST,
    // before pushing into the conversation. Each successful download prefixes its
    // result with `MEDIA_WORKDIR:<path>\n` — collect the sentinel paths for the
    // workdir side effect, and build a cleaned copy of each result (sentinel line
    // stripped) that is what the model will see. Fix 1: model never sees the raw
    // sentinel. Fix 3: validate the extracted path before mutating workdir.
    let cleaned_results: Vec<(String, String)> = state.rest.sessions[sess_idx]
        .tool_results
        .iter()
        .map(|(id, result)| {
            if let Some(first_line) = result.lines().next() {
                if first_line.starts_with("MEDIA_WORKDIR:") {
                    // Strip the sentinel line: everything after the first '\n'.
                    let body = result
                        .find('\n')
                        .map(|pos| &result[pos + 1..])
                        .unwrap_or("")
                        .to_string();
                    return (id.clone(), body);
                }
            }
            (id.clone(), result.clone())
        })
        .collect();

    // Collect validated sentinel paths for the workdir side effect (Fix 3).
    let media_dirs: Vec<String> = state.rest.sessions[sess_idx]
        .tool_results
        .iter()
        .filter_map(|(_, result)| {
            result
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("MEDIA_WORKDIR:"))
                .and_then(|p| {
                    // Fix 3: validate the extracted path is absolute and ends with
                    // a "media" path component — matching how session_media_dir()
                    // generates it (`<pwd_bucket_dir>/media/`). Reject anything
                    // that doesn't fit this pattern.
                    let path = std::path::Path::new(p);
                    if path.is_absolute()
                        && path.components().next_back()
                            == Some(std::path::Component::Normal("media".as_ref()))
                    {
                        Some(p.to_string())
                    } else {
                        None
                    }
                })
        })
        .collect();

    // Push the CLEANED tool results (sentinel stripped) into the conversation +
    // log them. Bind the session runtime once so `session` (mut) + `tool_results`
    // (read) are disjoint field borrows of the same `SessionRuntime`.
    {
        let rt = &mut state.rest.sessions[sess_idx];
        if let Some(sess) = rt.session.as_mut() {
            for (id, result) in &cleaned_results {
                let _ = crate::model::msglog::append(&sess.path, Role::Tool, result, None);
                sess.conversation.push_tool(id.clone(), result.clone());
            }
            let _ = sess.save();
        }
    }

    // Refresh the cumulative file-change log (#24) from the per-session store: the
    // `write`/`edit`/`delete` tools recorded their ops event-driven during this
    // round, so re-read the mirror once here (cheap, once per round) — the GUI
    // Explore "File changed" panel projects `rt.file_changes`, so it now reflects
    // what this round touched. Skipped when the session has no on-disk dir.
    if let Some(dir) = state.rest.sessions[sess_idx].session.as_ref().map(|s| s.path.clone()) {
        state.rest.sessions[sess_idx].file_changes = crate::model::msglog::read_file_changes(&dir);
    }

    // Refresh the session's todo mirror (#PLAN section) from whichever backing
    // file is CURRENTLY the source of truth: `plan_todos.md` while in Plan mode
    // (already kept live by the `todowrite`/`plan_ready` interceptions in
    // `approval.rs`, so this is a cheap no-op re-read there), else the
    // per-directory `memory/TODO.md` the generic (non-intercepted) `todowrite`
    // tool writes to in every OTHER mode. Read every round (cheap, mirrors the
    // `file_changes` refresh just above) so an execution-phase `todowrite` —
    // which isn't intercepted and never touches `rt.plan_todos` at its call
    // site — is reflected the instant this round finishes, in Auto/Normal/Yolo
    // just as much as Plan.
    let in_plan = state.rest.agent_mode == crate::app::state::AgentMode::Plan;
    if let Some(todos) = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|sess| crate::app::mode::todo::load_current_todos(sess, in_plan))
    {
        state.rest.sessions[sess_idx].plan_todos = todos;
    }

    // Inject any queued mid-turn steers as ONE coalesced user message before the
    // next hop, so the model sees the tool results + the user's steer together and
    // continues with its reasoning intact. Drained here = "sent in one window".
    let steers = std::mem::take(&mut state.rest.sessions[sess_idx].pending_steer);
    if !steers.is_empty() {
        let joined = steers.join("\n\n");
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            let _ = crate::model::msglog::append(&sess.path, Role::User, &joined, None);
            sess.conversation.push_user(joined);
            let _ = sess.save();
        }
    }

    // Apply the validated workdir side effect: append each media dir so the
    // downloaded file appears in @-autocomplete.
    if !media_dirs.is_empty() {
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            let mut changed = false;
            for dir in &media_dirs {
                if !sess.settings.workdir.contains(dir) {
                    sess.settings.workdir.push(dir.clone());
                    changed = true;
                }
            }
            if changed {
                let _ = sess.save();
                // Reindex the full workdir list so @-autocomplete picks up
                // the new media directory.
                let roots: Vec<std::path::PathBuf> =
                    sess.settings.workdir.iter().map(std::path::PathBuf::from).collect();
                crate::tool::dircache::reindex(
                    roots,
                    state.rest.sessions[sess_idx].dir_cache.clone(),
                );
            }
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
        // Snapshot the new mtime so the cross-instance poll doesn't fire a
        // spurious "Memory updated by another session" toast for our own write.
        if let Some(ref sess) = state.rest.sessions[sess_idx].session {
            if let Ok(dir) = crate::model::store::memory_dir(&sess.pwd_hash) {
                state.rest.sessions[sess_idx].last_memory_mtime =
                    crate::model::memory::memory_mtime(&dir);
            }
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
        state.rest.sessions[sess_idx].status = "no active session".into();
        return;
    };
    // The tool round is done; this re-stream is a model wait, so label it the same
    // "thinking" phase the comet sweeps (not a tool run).
    state.rest.sessions[sess_idx].status = "thinking".into();
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
    // Kill every BLOCKING running sub-agent and drop the pending queue so a
    // killed WC turn can't ghost-restart via orphaned tasks or stale flags.
    // Detached background agents are preserved (include_detached = false),
    // matching the Esc/interrupt behavior — the user's background work survives
    // a workspace-check denial just as it survives an Esc.
    rt.abort_running_subagents(false);
    rt.pending_tool_tasks.clear();
    rt.awaiting_tool_tasks = false;
    // Drop any TAC-classify park too, so a WC-denied turn can't leave the round
    // parked on a verdict (channel ends stay for reuse; a late verdict is dropped
    // by the drain's park/id guard).
    rt.awaiting_classify = false;
    rt.pending_classify_verdict = None;
}

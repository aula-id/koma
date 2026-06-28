//! Action handlers for chat-turn lifecycle: Submit, Interrupt, Resend,
//! ApproveTool, DenyTool.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::model::msglog;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::stream::{abort_current, dispatch_deferred, process_tools, run_tool, start_stream_task};

/// Handle `Action::Submit`: push the user message, spawn the stream task, and
/// optionally kick off the advisory prompt-classifier on a background task.
pub(super) fn handle_submit(
    text: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if client.is_none() || state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    // Busy guard: block while a turn is streaming OR a `!` shell is draining
    // off-thread — a Submit landing during the latter would race the shell entry
    // into the same conversation tail.
    if state.rest.fg().waiting || state.rest.fg().awaiting_shell {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    // Prompt-classifier (PC): keep a copy of the user's prompt to
    // classify in the background once the turn is kicked off.
    let pc_prompt = text.clone();
    // Send-time @-scan backstop (Slice 3): before finalising the user message,
    // scan the typed text for hand-typed `@path/to/image.png` tokens that were
    // NOT converted interactively (path-paste / @-picker).  Each such token is
    // ingested and rewritten to `[Image #N]` in `text`.  This runs BEFORE
    // `take_attachments()` so the interactive attachments are still on the
    // composer; the scan's attachments are appended to them afterward.
    let (text, scan_attachments) =
        if let Some(sess) = state.rest.fg().session.as_ref() {
            let images_dir = sess.images_dir();
            let workdir = sess.workdir();
            crate::model::attachment::scan_at_image_tokens(&text, &images_dir, &workdir)
        } else {
            (text, Vec::new())
        };
    // Move the staged composer attachments (from path-paste / @-picker) AND the
    // scan-backstop attachments onto THIS user message. Ingested bytes are already
    // on disk under `<session>/images/`; the wire builder re-reads at send time.
    let mut attachments = state.rest.take_attachments();
    attachments.extend(scan_attachments);
    let had_image = !attachments.is_empty();
    let history = {
        let sess = state.rest.fg_mut().session.as_mut().unwrap();
        let _ = msglog::append(&sess.path, Role::User, &text, None);
        sess.conversation.push_user_with_attachments(text, attachments);
        if let Err(e) = sess.save() {
            state.rest.status = format!("error: {e}");
            return Ok(());
        }
        sess.conversation.history()
    };
    // Image-capability guard: if this message carries images and the resolved Main
    // model can't read them, DON'T spend an API call — post a friendly notice in the
    // chat and keep the image un-sent (the orange attachment tree still shows it).
    // Capability mirrors run.rs: cold/None catalogue => assume capable (never wrongly block).
    if had_image {
        let main = state.rest.fg().session.as_ref().and_then(|sess| {
            crate::app::resolve::resolve_role(
                &state.rest.config,
                &sess.settings,
                crate::model::app_config::ModelRole::Main,
            )
        });
        let capable = match (state.rest.models_cache.as_deref(), main.as_ref()) {
            (Some(models), Some(m)) => crate::service::openrouter::model_takes_images(models, &m.model_id),
            _ => true,
        };
        if !capable {
            crate::app::runtime::stream::push_image_unsupported_notice(&mut state.rest);
            state.rest.reset_scroll();
            state.rest.status = "ready".into();
            return Ok(());
        }
    }
    state.rest.reset_scroll();
    state.rest.fg_mut().begin_stream();
    state.rest.fg_mut().waiting = true;
    // A new user turn starts fresh: no carried-over tool-call rounds or
    // a half-finished approval machine.
    state.rest.fg_mut().agent_steps = 0;
    state.rest.fg_mut().pending_tool_calls.clear();
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().tool_idx = 0;
    state.rest.fg_mut().tool_results.clear();
    // Hard-reset the deferred-task lane so a new submit can never inherit
    // awaiting_tool_tasks=true or stale pending ids from a prior halt path.
    state.rest.fg_mut().pending_tool_tasks.clear();
    state.rest.fg_mut().awaiting_tool_tasks = false;
    // Phase label for the comet: a single word the shimmer sweeps across
    // (the elapsed counter is appended by the renderer). No trailing dots —
    // the comet supplies the motion, `· Ns` supplies the elapsed.
    state.rest.status = "thinking".into();
    // Active session this submit drives (always foreground; captured into a local
    // before the call so it isn't read while a `&mut state.rest` is live).
    let fgi = state.rest.foreground;
    start_stream_task(history, state, fgi, client, handle);

    // Prompt-classifier (PC), advisory + non-blocking: once per turn, if
    // the harness is enabled, classify the user prompt on a background
    // task. It sends one HarnessVerdict on a dedicated channel (drained
    // in run_loop) — it NEVER gates the stream that just started. Drop
    // any stale receiver from a prior turn first.
    state.rest.fg_mut().harness_rx = None;
    let pc_inputs = match (client.as_ref(), state.rest.fg().session.as_ref()) {
        (Some(c), Some(sess)) if sess.settings.classifier_enabled => Some((
            Arc::clone(c),
            state.rest.config.clone(),
            sess.settings.clone(),
        )),
        _ => None,
    };
    if let Some((c, config, settings)) = pc_inputs {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.rest.fg_mut().harness_rx = Some(rx);
        handle.spawn(async move {
            let v =
                crate::app::harness::classify_prompt(&c, &config, &settings, &pc_prompt)
                    .await;
            // A dropped receiver (turn superseded / app closing) makes
            // this a no-op — same contract as the streaming channel.
            let _ = tx.send(crate::service::StreamEvent::HarnessVerdict {
                allow: v.allow,
                reason: v.reason,
            });
        });
    }
    Ok(())
}

/// Handle `Action::Shell`: run a `!`-prefixed command in the foreground session's
/// CURRENT working directory — OFF the event-loop thread — and (once it finishes)
/// append a DISTINCT shell entry to the conversation, without sending a turn to the
/// model or starting a stream.
///
/// The command runs via the SAME primitive the `bash` tool uses
/// ([`crate::tool::shell::run_shell_capture`]): captured stdout+stderr, ANSI-
/// stripped, output-capped, and timeout-bounded (the default 120s, matching the
/// tool). It is NOT gated by the workspace allow-list (WC) — this is a user
/// affordance, so it runs wherever the session cwd currently is (the live `cd`
/// override when set, else the configured workdir).
///
/// CRITICAL — it runs OFF-THREAD. `run_shell_capture` blocks the calling thread on
/// a `recv_timeout` (up to the full 120s) waiting for the child. Calling it inline
/// here would freeze the local TUI render loop or — in the daemon — the single
/// event loop that services EVERY session, stalling all sessions for the whole
/// command duration. So we spawn the blocking work on a plain `std::thread` (the
/// same shape `dispatch_deferred` uses for the model-callable `bash` tool, which
/// is exactly why `bash` lives in `DEFERRED_TOOLS`), mark the session
/// `awaiting_shell`, and return immediately. The event-loop drain
/// (`event_loop::sessions`) folds the captured `(command, output)` into the
/// conversation when it lands.
///
/// The result is stored as a [`Role::User`] message prefixed with
/// [`crate::dto::chat::SHELL_MARK`] and shaped `"$ <cmd>\n<output>"`. The mark makes
/// the transcript render it distinctly (a `$` header over dim output, see
/// `view::chat::transcript`) and is STRIPPED by the wire builder, so the model still
/// sees the clean `$ <cmd>\n<output>` text as context on its next real turn. The
/// entry is persisted to the msglog so it survives resume.
pub(super) fn handle_shell(text: String, state: &mut AppState) -> Result<()> {
    if state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    // Waiting guard (mirrors `handle_submit` / `handle_resend`): never start a `!`
    // shell while a turn is streaming OR another `!` shell is still draining
    // off-thread. Pushing a shell entry mid-turn would interleave it between the
    // pending question and the not-yet-committed assistant reply, and the committed
    // history would read [user, shell, assistant] — sent to the model in that order
    // next turn. The controller already declines to route a `!` while busy; this is
    // the daemon-side backstop (the daemon drives the same handler over IPC).
    if state.rest.fg().waiting || state.rest.fg().awaiting_shell {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    let cmd = text.trim().to_string();
    if cmd.is_empty() {
        state.rest.status = "usage: !<command>".into();
        return Ok(());
    }
    let fgi = state.rest.foreground;
    // Lazily create THIS session's shell-result channel once, then reuse it. The
    // spawned thread fires back over session `fgi`'s own `shell_task_tx`, so the
    // result is routed structurally to that session's drain regardless of which
    // session is foreground when it lands.
    if state.rest.sessions[fgi].shell_task_tx.is_none() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.rest.sessions[fgi].shell_task_tx = Some(tx);
        state.rest.sessions[fgi].shell_task_rx = Some(rx);
    }
    // Run in the session's EFFECTIVE cwd (follows `cd`). Same default timeout as
    // the bash tool. Capture cwd + the sender before the spawn so nothing borrows
    // `state` across the thread boundary.
    let cwd = state.rest.fg().effective_cwd();
    let tx = state.rest.sessions[fgi].shell_task_tx.as_ref().unwrap().clone();
    // Plain `std::thread` (NOT a tokio task): `run_shell_capture` itself spawns a
    // helper thread + blocks on `recv_timeout`, and there's no async involved, so a
    // bare OS thread is the right home. The `UnboundedSender` is `Send`, so it can
    // fire from this off-runtime thread.
    std::thread::spawn(move || {
        let output = crate::tool::shell::run_shell_capture(&cmd, &cwd, 120_000);
        let _ = tx.send((cmd, output));
    });
    // Park the session on the shell lane: `awaiting_shell` keeps the busy indicator
    // on (it feeds `is_working`) and gates a second `!`/Submit until the drain
    // clears it. Phase label names the affordance so the shimmering status surfaces
    // what's running.
    state.rest.sessions[fgi].awaiting_shell = true;
    state.rest.status = "running shell".into();
    Ok(())
}

/// Handle `Action::Interrupt`: abort the in-flight stream, finalize the partial
/// buffer with an "[interrupted]" marker, and reset the agentic-loop state.
pub(super) fn handle_interrupt(state: &mut AppState) -> Result<()> {
    // Custom finalization (not finish_stream): the partial buffer is
    // committed with an "  [interrupted]" marker. abort_current drops
    // active_rx, so the aborted task's late events are ignored.
    if state.rest.fg().waiting {
        abort_current(&mut state.rest);
        // Halt the agentic loop: drop any stashed tool calls, reset the
        // step counter, and clear the approval machine so a halt mid-
        // approval doesn't leave the turn wedged.
        state.rest.fg_mut().pending_tool_calls.clear();
        state.rest.fg_mut().agent_steps = 0;
        state.rest.fg_mut().awaiting_approval = false;
        state.rest.fg_mut().approval_reason = None;
        state.rest.fg_mut().tool_idx = 0;
        state.rest.fg_mut().tool_results.clear();
        // Kill every running sub-agent spawned by this turn and drop the
        // pending queue.  abort_running_subagents also clears
        // pending_subagent_calls and awaiting_subagents, so the halt path is
        // complete — no orphaned background task can deliver a late result.
        state.rest.fg_mut().abort_running_subagents();
        // Abandon any round parked on a deferred tool task (the heavy/blocking
        // tools — read/write/edit/delete/bash/grep/glob/remember/web_fetch/
        // web_search) the same way. The off-thread worker keeps running but its
        // result lands with no matching pending id, so the next-turn machine reset
        // discards it; it can't resume a turn the user killed. The channel itself
        // is left intact for reuse by later deferred tools. We deliberately do NOT
        // join the worker here — joining could block the UI thread for the full
        // duration of the tool (e.g. a long bash or HTTP timeout), the exact freeze
        // this fix removes.
        state.rest.fg_mut().pending_tool_tasks.clear();
        state.rest.fg_mut().awaiting_tool_tasks = false;
        // Take any captured usage unconditionally so a partial turn's
        // usage can't leak into the next response.
        let usage = state.rest.fg_mut().pending_usage.take();
        // Likewise drain the reasoning buffer unconditionally so a
        // half-streamed thinking block can't bleed into the next turn;
        // it's folded onto the interrupted message (display-only).
        let reasoning = state.rest.fg_mut().take_reasoning();
        let buf = state.rest.fg_mut().take_stream();
        if let Some(b) = buf {
            if !b.is_empty() {
                let mut committed = false;
                if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    let content = format!("{b}  [interrupted]");
                    let _ = msglog::append(&sess.path, Role::Assistant, &content, usage);
                    sess.conversation.push_assistant(content, reasoning);
                    let _ = sess.save();
                    committed = true;
                }
                // Update the FOREGROUND session's own counters once the `sess`
                // borrow above has ended (this is the active tab being interrupted).
                if committed {
                    if let Some((pt, ct, cost)) = usage {
                        let rt = state.rest.fg_mut();
                        rt.tokens_in = pt;        // current context size, not a sum
                        rt.tokens_out += ct;
                        rt.cost += cost;
                    }
                }
            }
        }
    }
    state.rest.status = "interrupted".into();
    Ok(())
}

/// Handle `Action::Resend`: pop trailing assistant messages and re-stream the
/// last user turn.
pub(super) fn handle_resend(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if state.rest.fg().waiting || state.rest.fg().awaiting_shell {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    if client.is_none() || state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    let history = {
        let sess = state.rest.fg_mut().session.as_mut().unwrap();
        if sess.conversation.last_user_content().is_none() {
            state.rest.status = "nothing to resend".into();
            return Ok(());
        }
        sess.conversation.pop_trailing_assistants();
        let _ = sess.save();
        sess.conversation.history()
    };
    state.rest.reset_scroll();
    state.rest.fg_mut().begin_stream();
    state.rest.fg_mut().waiting = true;
    state.rest.status = "thinking".into();
    let fgi = state.rest.foreground;
    start_stream_task(history, state, fgi, client, handle);
    Ok(())
}

/// Handle `Action::ApproveTool`: run the paused risky call, record its result,
/// advance past it, then resume the tool-processing machine.
pub(super) fn handle_approve_tool(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Run the paused risky call, record its result, advance past it, then
    // resume the machine (which may pause again on the next risky call or
    // finish the round). Clone the call out first so `run_tool`'s mutable
    // borrow of `state` doesn't overlap the `pending_tool_calls` read.
    // The approval machine drives the foreground session this stage; captured
    // into a local so the turn-machinery calls below can pass it.
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    if let Some(call) = state.rest.fg().pending_tool_calls.get(state.rest.fg().tool_idx).cloned() {
        // The approved call is a risky tool (write/edit/delete/bash) — all of which
        // are heavy/blocking and live in `DEFERRED_TOOLS`. Run it OFF the UI thread
        // and PARK rather than running it inline here: an approved large write would
        // otherwise re-freeze the comet for the whole write. `dispatch_deferred`
        // advances `tool_idx` and registers the pending id; we return without
        // re-entering `process_tools` (the resume gate does that once the result
        // lands). The defensive `else` keeps the old inline path for the unexpected
        // case of a non-deferred approved tool, so no call is ever left dangling.
        if crate::tool::DEFERRED_TOOLS.contains(&call.function.name.as_str()) {
            dispatch_deferred(state, fgi, &call);
            return Ok(());
        }
        let result = run_tool(state, fgi, &call);
        state.rest.fg_mut().tool_results.push((call.id.clone(), result));
        state.rest.fg_mut().tool_idx += 1;
    }
    process_tools(state, fgi, client, handle);
    Ok(())
}

/// Handle `Action::DenyTool`: answer every pending call with "denied by user",
/// commit, and stop — do not re-stream.
pub(super) fn handle_deny_tool(state: &mut AppState) -> Result<()> {
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    // Denial halts the turn. Answer the denied call AND every remaining
    // pending call with "denied by user" (so the conversation stays
    // API-valid: every tool_call gets a result), commit any results
    // already collected this round, then STOP — do not re-stream.
    let results = state.rest.fg().tool_results.clone();
    let denied_ids: Vec<String> = state
        .rest
        .fg()
        .pending_tool_calls
        .iter()
        .skip(state.rest.fg().tool_idx)
        .map(|c| c.id.clone())
        .collect();
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        for (id, result) in &results {
            let _ = msglog::append(
                &sess.path,
                Role::Tool,
                result,
                None,
            );
            sess.conversation.push_tool(id.clone(), result.clone());
        }
        for id in &denied_ids {
            let _ = msglog::append(
                &sess.path,
                Role::Tool,
                "denied by user",
                None,
            );
            sess.conversation.push_tool(id.clone(), "denied by user".to_string());
        }
        let _ = sess.save();
    }
    // Reset the agentic-loop state and end the turn.
    state.rest.fg_mut().pending_tool_calls.clear();
    state.rest.fg_mut().tool_idx = 0;
    state.rest.fg_mut().tool_results.clear();
    state.rest.fg_mut().agent_steps = 0;
    state.rest.fg_mut().waiting = false;
    state.rest.fg_mut().current_task = None;
    // Kill every running sub-agent and drop the pending queue so the resume
    // gate can't ghost-restart a killed turn via stale flags or orphaned tasks.
    state.rest.fg_mut().abort_running_subagents();
    state.rest.fg_mut().pending_tool_tasks.clear();
    state.rest.fg_mut().awaiting_tool_tasks = false;
    state.rest.status = "denied — stopped".into();
    Ok(())
}

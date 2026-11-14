//! Action handlers for chat-turn lifecycle: Submit, Interrupt, Resend,
//! ApproveTool, DenyTool.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::model::msglog;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::stream::{dispatch_deferred, process_tools, run_tool, start_stream_task};

/// Extract a non-empty `human_gate` from a parked `mission_verify` call.
fn human_gate_from_mission_verify_call(call: &crate::dto::chat::ToolCall) -> Result<String, ()> {
    if call.function.name != "mission_verify" {
        return Err(());
    }
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    args.get("human_gate")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or(())
}

/// Persist human-gate approval only after explicit user ApproveTool.
fn approve_mission_human_gate(state: &mut AppState, sess_idx: usize, gate: &str) -> String {
    let Some(sess_path) = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.path.clone())
    else {
        return "error: no active session".into();
    };
    let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) else {
        return "error: human_gate requires an approved mission".into();
    };
    if !m.approved {
        return "error: human_gate requires an approved mission".into();
    }
    if !m.human_gates.iter().any(|g| g == gate) {
        return format!("error: unknown human_gate '{gate}'");
    }
    m.approve_human_gate(gate);
    if let Err(e) = m.save(&sess_path) {
        return format!("error: could not persist human_gate approval: {e}");
    }
    if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
        sess.rebuild_system();
        let _ = sess.save();
    }
    format!(
        "mission_verify: human_gate '{gate}' approved by user ({}/{})",
        m.human_gates_approved.len(),
        m.human_gates.len()
    )
}

/// Handle `Action::Submit`: push the user message, spawn the stream task, and
/// optionally kick off the advisory prompt-classifier on a background task.
pub(super) fn handle_submit(
    text: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if client.is_none() || state.rest.fg().session.is_none() {
        if state.rest.fg().input.is_empty() {
            state.rest.fg_mut().input = text;
            state.rest.cursor_end();
        }
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    }
    // A shell command in flight still blocks (different mode).
    if state.rest.fg().awaiting_shell {
        state.rest.fg_mut().status = "busy — wait for response".into();
        return Ok(());
    }
    // A cooking agentic turn: QUEUE this submit as a mid-turn steer instead of
    // blocking. Hard cap 5; a 6th warns via toast and is dropped.
    if state.rest.fg().waiting {
        let steer_text = text.trim().to_string();
        if steer_text.is_empty() {
            return Ok(());
        }
        if state.rest.fg().pending_steer.len() >= 5 {
            state
                .rest
                .fg_mut()
                .set_toast("5 pending allowed!".to_string());
            return Ok(());
        }
        // Real user activity (queued-steer path): reset the extension-injection
        // budget counter (cost-DoS guard, review finding) — the user typed
        // something, even though it's queued as a mid-turn steer rather than
        // starting fresh. Mirrors the reset below for the immediate-kickoff path.
        state.rest.fg_mut().ext_injected_turns = 0;
        state.rest.fg().call_track.invalidate_inspections();
        let attachments = state.rest.take_attachments(&steer_text);
        state
            .rest
            .fg_mut()
            .pending_steer
            .push(crate::app::state::FollowUp {
                text: steer_text,
                attachments,
            });
        // Keep selection on the newest item and leave focus on the composer so
        // the user can keep typing more follow-ups; they can ↑ into the list.
        let n = state.rest.fg().pending_steer.len();
        state.rest.pending_steer_sel = n.saturating_sub(1);
        return Ok(());
    }
    // Legacy submit retains its status/toast behavior; tracked callers use the
    // fallible fresh-only helper directly and receive an accurate disposition.
    let _ = kickoff_fresh_turn(text, state, client, handle);
    Ok(())
}

/// Match the send route; unknown or stale capability data remains fail-open.
pub(crate) fn model_accepts_images(state: &AppState) -> bool {
    // Resolve the SAME route the send will use so the capability guard checks the
    // model that actually receives the image.
    let main = state.rest.fg().session.as_ref().and_then(|sess| {
        crate::app::resolve::resolve_role_dispatch(
            &state.rest.config,
            &sess.settings,
            crate::model::app_config::ModelRole::Main,
        )
    });
    match (state.rest.models_cache.as_deref(), main.as_ref()) {
        // Codex Responses models are image-capable and have no static
        // catalogue to consult, so never block on a lookup that must miss.
        (_, Some(m)) if m.api_type == crate::model::app_config::ApiType::Codex => true,
        // Only trust the catalogue when it was fetched for THIS model's
        // endpoint. After a mid-session provider/model swap the cache may
        // still hold the old endpoint's models (missing the new model) —
        // treating that as "can't see images" wrongly blocks the send. If
        // the cache is cold or for a different endpoint, assume capable and
        // never wrongly block (a fresh catalogue re-fetch corrects it).
        (Some(models), Some(m))
            if state.rest.models_cache_endpoint.as_deref() == Some(m.endpoint.as_str()) =>
        {
            use crate::service::openrouter::ImageCapability;
            matches!(
                crate::service::openrouter::model_image_capability(models, &m.model_id),
                ImageCapability::Supports | ImageCapability::Unknown
            )
        }
        _ => true,
    }
}

/// Start one fresh logical user turn. Never enqueue or steer. Callers validate
/// fresh-turn preconditions (tracked callers reject empty/busy/closed). The caller selects
/// the foreground session before calling; the returned id survives tool rounds.
pub(crate) fn kickoff_fresh_turn(
    text: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<String> {
    anyhow::ensure!(client.is_some() && state.rest.fg().session.is_some(), "no active session");
    let pc_prompt = text.clone();
    // Send-time @-scan backstop (Slice 3): before finalising the user message,
    // scan the typed text for hand-typed `@path/to/image.png` tokens that were
    // NOT converted interactively (path-paste / @-picker).  Each such token is
    // ingested and rewritten to `[Image #N]` in `text`.  This runs BEFORE
    // `take_attachments()` so the interactive attachments are still on the
    // composer; the scan's attachments are appended to them afterward.
    let (text, scan_attachments) = if let Some(sess) = state.rest.fg().session.as_ref() {
        let images_dir = sess.images_dir();
        let workdir = sess.workdir();
        crate::model::attachment::scan_at_image_tokens(&text, &images_dir, &workdir)
    } else {
        (text, Vec::new())
    };
    // Move the staged composer attachments (from path-paste / @-picker / paste
    // chips) AND the scan-backstop attachments onto THIS user message. Ingested
    // bytes are already on disk under `<session>/images/` or `pastes/`; the wire
    // builder re-reads images at send time, and paste markers expand to fences
    // below so the model sees full bodies in content.
    // Pass the finalised `text` so a deleted / broken marker drops its staged
    // attachment instead of shipping a marker-less record.
    let mut attachments = state.rest.take_attachments(&text);
    attachments.extend(scan_attachments);
    execute_prepared_turn(super::turn_input::PreparedTurnInput { text, attachments }, pc_prompt, state, client, handle)
}

/// Shared execution tail: consumes only prepared, owned input.
pub(crate) fn kickoff_prepared_turn(
    input: super::turn_input::PreparedTurnInput,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<String> {
    let pc_prompt = input.text.clone();
    execute_prepared_turn(input, pc_prompt, state, client, handle)
}

fn execute_prepared_turn(
    input: super::turn_input::PreparedTurnInput,
    pc_prompt: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<String> {
    anyhow::ensure!(client.is_some() && state.rest.fg().session.is_some(), "no active session");
    let super::turn_input::PreparedTurnInput { text, attachments } = input;
    // Real user activity (immediate-kickoff path): reset the extension-injection
    // budget counter (cost-DoS guard, review finding — see
    // `SessionRuntime::ext_injected_turns`) now that we know this submit is a real,
    // non-steer user turn proceeding past the busy guards above. NEVER reset from a
    // synthetic kickoff — bash/subagent/ext-nudge auto-wakes
    // (`event_loop::sessions::deferred`) call `start_stream_task` directly and never
    // go through this handler.
    state.rest.fg_mut().ext_injected_turns = 0;
    state.rest.fg().call_track.invalidate_inspections();
    let had_image = attachments.iter().any(|a| a.is_image());
    // Expand `[Pasted Text #N]` → machine fences BEFORE msglog / push, using the
    // session dir so disk is SoT for body bytes.
    let session_dir = state.rest.fg().session.as_ref().map(|s| s.path.clone());
    let text = if let Some(dir) = session_dir.as_ref() {
        crate::model::attachment::expand_paste_markers(&text, &attachments, dir)
    } else {
        text
    };
    let Some(sess) = state.rest.fg_mut().session.as_mut() else {
        crate::model::store::append_global_error_log(
            "chat",
            "BUG: fg session missing in handle_submit",
        );
        anyhow::bail!("no active session");
    };
    let _ = msglog::append(&sess.path, Role::User, &text, None, None);
    sess.conversation
        .push_user_with_attachments(text, attachments);
    if let Err(e) = sess.save() {
        state.rest.fg_mut().status = format!("error: {e}");
        return Err(e);
    }
    let history = sess.conversation.history();
    // Image-capability guard: if this message carries images and the resolved Main
    // model can't read them, DON'T spend an API call — post a friendly notice in the
    // chat and keep the image un-sent (the orange attachment tree still shows it).
    // Uses the tri-state `model_image_capability` so an unknown/missing/stale
    // catalogue never wrongly blocks (fail-open: `Unknown` => allow).
    if had_image {
        let capable = model_accepts_images(state);
        if !capable {
            crate::app::runtime::stream::push_image_unsupported_notice(&mut state.rest);
            state.rest.reset_scroll();
            state.rest.fg_mut().status = "ready".into();
            anyhow::bail!("model does not support image input");
        }
    }
    state.rest.reset_scroll();
    state.rest.fg_mut().begin_stream();
    state.rest.fg_mut().waiting = true;
    // A new user turn starts fresh: no carried-over tool-call rounds or
    // a half-finished approval machine.
    state.rest.fg_mut().agent_steps = 0;
    state.rest.fg_mut().main_stall_nudges = 0;
    state.rest.fg_mut().pending_tool_calls.clear();
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().tool_idx = 0;
    state.rest.fg_mut().tool_results.clear();
    // Hard-reset the deferred-task lane so a new submit can never inherit
    // awaiting_tool_tasks=true or stale pending ids from a prior halt path.
    state.rest.fg_mut().pending_tool_tasks.clear();
    state.rest.fg_mut().awaiting_tool_tasks = false;
    // Same for the TAC-classify park lane: a new submit must never inherit a stale
    // awaiting_classify / pending verdict from a prior halted turn.
    state.rest.fg_mut().awaiting_classify = false;
    state.rest.fg_mut().pending_classify_verdict = None;
    // NOTE: approved_plan / approved_mission are intentionally NOT cleared here —
    // they must survive continue-nudges so the classifier stays plan/mission-aware
    // through execution. approved_plan clears on Plan re-entry; approved_mission
    // clears on deny, leave/enter SDLC, mission_clear, or integrate→done.
    // Phase label for the comet: a single word the shimmer sweeps across
    // (the elapsed counter is appended by the renderer). No trailing dots —
    // the comet supplies the motion, `· Ns` supplies the elapsed.
    state.rest.fg_mut().status = "thinking".into();
    // Active session this submit drives (always foreground; captured into a local
    // before the call so it isn't read while a `&mut state.rest` is live).
    let fgi = state.rest.foreground;
    let turn_id = state.rest.fg_mut().begin_turn();
    start_stream_task(history, state, fgi, client, handle);
    if !state.rest.fg().waiting {
        let error = state.rest.fg().toast.as_ref()
            .map(|(message, _, _)| message.clone())
            .unwrap_or_else(|| "stream kickoff failed".into());
        let rt = state.rest.fg_mut();
        rt.take_stream();
        rt.complete_turn(crate::ipc::proto::turn::TurnState::Failed, Some(error.clone()));
        anyhow::bail!(error);
    }

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
            let v = crate::app::harness::classify_prompt(&c, &config, &settings, &pc_prompt).await;
            // A dropped receiver (turn superseded / app closing) makes
            // this a no-op — same contract as the streaming channel.
            let _ = tx.send(crate::service::StreamEvent::HarnessVerdict {
                allow: v.allow,
                reason: v.reason,
            });
        });
    }
    Ok(turn_id)
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
/// same shape `dispatch_deferred` uses for heavy I/O tools), mark the session
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
        state.rest.fg_mut().status = "no active session".into();
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
        state.rest.fg_mut().status = "busy — wait for response".into();
        return Ok(());
    }
    let cmd = text.trim().to_string();
    if cmd.is_empty() {
        state.rest.fg_mut().status = "usage: !<command>".into();
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
    let Some(tx) = state.rest.sessions[fgi].shell_task_tx.as_ref().cloned() else {
        crate::model::store::append_global_error_log("chat", "BUG: shell_task_tx missing");
        return Ok(());
    };
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
    state.rest.fg_mut().status = "running shell".into();
    Ok(())
}

/// Handle `Action::Interrupt`: abort the in-flight stream, finalize the partial
/// buffer with an "[interrupted]" marker, and reset the agentic-loop state.
pub(super) fn handle_interrupt(state: &mut AppState) -> Result<()> {
    // Custom finalization (not finish_stream): the partial buffer is committed
    // with an "  [interrupted]" marker. The per-session teardown (abort task +
    // drop active_rx so late events are ignored, halt the agentic loop, kill
    // sub-agents, commit the partial buffer to the foreground session) lives in
    // `SessionRuntime::interrupt()` so the session hub's Ctrl+X can reuse it on
    // ANY session; here it runs on the foreground.
    //
    // Unconditional cut: the user has the right to interrupt at any time, busy or
    // not — like the TUI's Esc (which is pre-gated CLIENT-side on
    // `waiting || compacting` in `controller/input/chat.rs`, so this daemon-side path
    // being unconditional doesn't change TUI behavior; it matters for the GUI stop
    // button and any other caller that reaches this handler without that gate).
    // Calling `interrupt()` on an already-idle session is a harmless no-op: it aborts
    // a `None` current_task, clears already-empty queues, and `take_stream()` yields
    // `None` so nothing is committed (see `SessionRuntime::interrupt`).
    let was_busy = state.rest.fg().is_ui_busy();
    let fg = state.rest.fg_mut();
    fg.interrupt();
    // PER-SESSION compaction cleanup (C4): tear down THIS foreground session's
    // in-flight compaction animation / deferred apply so an interrupt mid-compact
    // doesn't leave the spinner stuck forever.
    fg.compact_anim_start = None;
    fg.compact_apply_at = None;
    fg.compact_pending = None;
    // Only report "interrupted" if there was actually something to cut — an idle
    // session's no-op stop gets no misleading status change.
    if was_busy {
        state.rest.fg_mut().status = "interrupted".into();
    }
    Ok(())
}

/// Handle `Action::InterruptRewind`: early-Esc auto-rewind. The model is still
/// THINKING (no content token / tool call yet), so a single Esc interrupts the
/// turn AND pulls the just-sent prompt back into the composer, physically
/// truncating the turn (no restore) so the user can edit + resend in one key.
///
/// Steps:
/// 1. Run the existing interrupt path (aborts the stream via
///    `SessionRuntime::interrupt()`; for early-thinking the content buffer is
///    empty, so nothing is committed).
/// 2. Find the index of the LAST `Role::User` message in the live conversation. If
///    there is none, the plain interrupt already happened — return.
/// 3. Cut to just before that user turn and load its text into the composer via the
///    shared rewind core (`super::rewind::rewind_to_vec_index`).
pub(super) fn handle_interrupt_rewind(state: &mut AppState) -> Result<()> {
    // 1. Plain interrupt first (idempotent + safe on an idle session).
    handle_interrupt(state)?;

    // 2. Locate the last user message; nothing to rewind to → the interrupt stands.
    let last_user_idx = state.rest.fg().session.as_ref().and_then(|s| {
        s.conversation
            .messages()
            .iter()
            .rposition(|m| m.role == Role::User)
    });
    let Some(idx) = last_user_idx else {
        return Ok(());
    };

    // 3. Truncate to before that turn + load its text into the composer.
    super::rewind::rewind_to_vec_index(state, idx)
}

/// Handle `Action::Resend`: pop trailing assistant messages and re-stream the
/// last user turn.
pub(super) fn handle_resend(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if state.rest.fg().waiting || state.rest.fg().awaiting_shell {
        state.rest.fg_mut().status = "busy — wait for response".into();
        return Ok(());
    }
    if client.is_none() || state.rest.fg().session.is_none() {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    }
    let Some(sess) = state.rest.fg_mut().session.as_mut() else {
        crate::model::store::append_global_error_log(
            "chat",
            "BUG: fg session missing in handle_resend",
        );
        return Ok(());
    };
    if sess.conversation.last_user_content().is_none() {
        state.rest.fg_mut().status = "nothing to resend".into();
        return Ok(());
    }
    sess.conversation.pop_trailing_assistants();
    let _ = sess.save();
    let history = sess.conversation.history();
    state.rest.reset_scroll();
    state.rest.fg_mut().begin_stream();
    state.rest.fg_mut().waiting = true;
    state.rest.fg_mut().status = "thinking".into();
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
    anyhow::ensure!(
        state.rest.fg().awaiting_approval,
        "no tool call awaiting approval"
    );
    // Server-side UI guarantee: generic ApproveTool must never answer a parked
    // plan_ready / mission_ready — those require PlanDecision (y/a/n). Reject
    // without clearing the park so the correct handlers remain reachable.
    if super::plan_decision::is_pending_plan_or_mission_ready(state) {
        anyhow::bail!(
            "plan_ready/mission_ready require PlanDecision (approve/compact/deny), not ApproveTool"
        );
    }
    // Approval authorizes this call, never a mode change or a read-only bypass.
    if let Some(call) = state
        .rest
        .fg()
        .pending_tool_calls
        .get(state.rest.fg().tool_idx)
    {
        let args: serde_json::Value = serde_json::from_str(
            &crate::dto::chat::sanitize_tool_arguments(&call.function.arguments),
        )
        .unwrap_or_else(|_| serde_json::json!({}));
        if state.rest.fg().agent_mode == crate::app::state::AgentMode::Plan {
            crate::tool::plan_tool_call_allowed(&call.function.name, &args)
                .map_err(anyhow::Error::msg)?;
        }
        if state.rest.fg().agent_mode == crate::app::state::AgentMode::Sdlc
            && state.rest.fg().sdlc_phase.as_deref() == Some("assess")
        {
            anyhow::ensure!(
                crate::tool::tool_allowed_in_sdlc_assess(&call.function.name),
                "SDLC assess is read-only"
            );
            if call.function.name == "git_operator" {
                let arguments = args
                    .get("args")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow::anyhow!("missing git args"))?;
                let arguments: Vec<&str> = arguments
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| anyhow::anyhow!("invalid git args"))
                    })
                    .collect::<Result<_>>()?;
                crate::tool::sdlc_assess_git_args_allowed(&arguments)
                    .map_err(anyhow::Error::msg)?;
            }
        }
    }
    // Run the paused risky call, record its result, advance past it, then
    // resume the machine (which may pause again on the next risky call or
    // finish the round). Clone the call out first so `run_tool`'s mutable
    // borrow of `state` doesn't overlap the `pending_tool_calls` read.
    // The approval machine drives the foreground session this stage; captured
    // into a local so the turn-machinery calls below can pass it.
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    if let Some(call) = state
        .rest
        .fg()
        .pending_tool_calls
        .get(state.rest.fg().tool_idx)
        .cloned()
    {
        // `git_worktree` is intercepted in `process_tools` and needs its special
        // post-processing (workdir de-registration + cwd snap-back); the generic
        // `run_tool` path below would push a raw sentinel string and skip that
        // cleanup. So for an approved git_worktree call, mark it approved and
        // re-enter the machine, which runs the interception (skipping the gate for
        // this one call).
        if call.function.name == "git_worktree" {
            state.rest.fg_mut().approved_worktree_call = Some(call.id.clone());
            process_tools(state, fgi, client, handle);
            return Ok(());
        }
        // SDLC human gate: mission_verify(human_gate=...) parks for user y/n.
        // Persist approval ONLY on explicit ApproveTool — never from the model.
        if call.function.name == "mission_verify" {
            if let Ok(gate) = human_gate_from_mission_verify_call(&call) {
                let result = approve_mission_human_gate(state, fgi, &gate);
                state
                    .rest
                    .fg_mut()
                    .tool_results
                    .push((call.id.clone(), result));
                state.rest.fg_mut().tool_idx += 1;
                process_tools(state, fgi, client, handle);
                return Ok(());
            }
        }
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
        state
            .rest
            .fg_mut()
            .tool_results
            .push((call.id.clone(), result));
        state.rest.fg_mut().tool_idx += 1;
    }
    process_tools(state, fgi, client, handle);
    Ok(())
}

/// Handle `Action::DenyTool`: answer every pending call with "denied by user",
/// commit, and stop — do not re-stream.
pub(super) fn handle_deny_tool(state: &mut AppState) -> Result<()> {
    // Mirror ApproveTool: do not let DenyTool short-circuit a parked plan/mission
    // decision (that would halt the turn instead of "chat more" via DenyPlan).
    if super::plan_decision::is_pending_plan_or_mission_ready(state) {
        anyhow::bail!(
            "plan_ready/mission_ready require PlanDecision (approve/compact/deny), not DenyTool"
        );
    }
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    // Denial halts the turn. Answer the denied call AND every remaining
    // pending call with "denied by user" (so the conversation stays
    // API-valid: every tool_call gets a result), commit any results
    // already collected this round, then STOP — do not re-stream.
    //
    // Special case: a parked mission_verify human_gate denial records an
    // explicit gate-denial result (gate stays unapproved) for the first call.
    let results = state.rest.fg().tool_results.clone();
    let denied_from = state.rest.fg().tool_idx;
    let pending: Vec<crate::dto::chat::ToolCall> = state
        .rest
        .fg()
        .pending_tool_calls
        .iter()
        .skip(denied_from)
        .cloned()
        .collect();
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        for (id, result) in &results {
            let _ = msglog::append(&sess.path, Role::Tool, result, None, None);
            sess.conversation.push_tool(id.clone(), result.clone());
        }
        for (i, call) in pending.iter().enumerate() {
            let msg = if i == 0 {
                if let Ok(gate) = human_gate_from_mission_verify_call(call) {
                    format!(
                        "mission_verify: human_gate '{gate}' DENIED by user — gate remains unapproved"
                    )
                } else {
                    "denied by user".to_string()
                }
            } else {
                "denied by user".to_string()
            };
            let _ = msglog::append(&sess.path, Role::Tool, &msg, None, None);
            sess.conversation.push_tool(call.id.clone(), msg);
        }
        let _ = sess.save();
    }
    // Reset the agentic-loop state and end the turn.
    state.rest.fg_mut().complete_turn(crate::ipc::proto::turn::TurnState::Interrupted, None);
    state.rest.fg_mut().pending_tool_calls.clear();
    state.rest.fg_mut().tool_idx = 0;
    state.rest.fg_mut().tool_results.clear();
    state.rest.fg_mut().agent_steps = 0;
    state.rest.fg_mut().waiting = false;
    state.rest.fg_mut().current_task = None;
    // Kill every BLOCKING running sub-agent and drop the pending queue so the
    // resume gate can't ghost-restart a killed turn via stale flags or orphaned
    // tasks. Detached background agents are preserved (include_detached = false),
    // matching the Esc/interrupt behavior — deny-tool is a turn-halt, not a
    // session close, so the user's background work keeps running.
    state.rest.fg_mut().abort_running_subagents(false);
    state.rest.fg_mut().pending_tool_tasks.clear();
    state.rest.fg_mut().awaiting_tool_tasks = false;
    // Drop any TAC-classify park too (deny is a turn-halt): clear the flag + any
    // staged verdict so the halted round can't later resume off a stale verdict.
    state.rest.fg_mut().awaiting_classify = false;
    state.rest.fg_mut().pending_classify_verdict = None;
    state.rest.fg_mut().status = "denied — stopped".into();
    Ok(())
}

#[cfg(test)]
#[path = "chat_approval_tests.rs"]
mod approval_tests;

#[cfg(test)]
mod followup_tests {
    use crate::app::mode::Mode;
    use crate::app::state::SessionRuntime;
    use crate::app::state::{AppState, FollowUp};
    use crate::dto::chat::{Attachment, AttachmentKind};
    fn image(n: usize) -> Attachment {
        Attachment {
            kind: AttachmentKind::Image,
            marker_n: n,
            rel_path: format!("images/{n}.png"),
            mime: "image/png".into(),
            label: None,
        }
    }
    #[test]
    fn enqueue_captures_only_accepted_media_and_full_queue_keeps_draft() {
        let mut state = AppState::new(Mode::Chat);
        state.rest.fg_mut().session = Some(crate::model::session::Session::new(
            "queue".into(),
            std::env::temp_dir().join("koma-queue-fixture"),
            "test".into(),
            Default::default(),
            crate::model::conversation::Conversation::new(""),
        ));
        state.rest.fg_mut().waiting = true;
        state.rest.fg_mut().pending_attachments = vec![image(1)];
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut client = Some(std::sync::Arc::new(
            crate::service::openrouter::OpenRouterClient::new(),
        ));
        super::handle_submit(
            "queued [Image #1]".into(),
            &mut state,
            &mut client,
            runtime.handle(),
        )
        .unwrap();
        assert_eq!(state.rest.fg().pending_steer[0].attachments, vec![image(1)]);
        assert!(state.rest.fg().pending_attachments.is_empty());
        for _ in 0..4 {
            state
                .rest
                .fg_mut()
                .pending_steer
                .push("text".to_string().into());
        }
        state.rest.fg_mut().input = "sixth [Image #2]".into();
        state.rest.fg_mut().pending_attachments = vec![image(2)];
        state.rest.cursor_end();
        let action = crate::controller::input::handle_chat(
            &mut state.rest,
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Enter,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert!(matches!(action, crate::controller::input::Action::None));
        assert_eq!(state.rest.fg().input, "sixth [Image #2]");
        assert_eq!(state.rest.fg().pending_attachments, vec![image(2)]);
    }
    #[test]
    fn followups_keep_media_through_projection_shadow_edit_and_clear() {
        let mut state = AppState::new(Mode::Chat);
        state.rest.fg_mut().pending_steer.push(FollowUp {
            text: "queued [Image #1]".into(),
            attachments: vec![image(1)],
        });
        state.rest.fg_mut().pending_attachments.push(image(2));
        let snap = crate::ipc::snapshot::build_snapshot(&state);
        let shadow = crate::app::runtime::client_shadow::shadow_session_runtime(&snap.sessions[0]);
        assert_eq!(shadow.pending_steer, state.rest.fg().pending_steer);
        let mut changed = snap.clone();
        changed.sessions[0].pending_steer_attachments[0][0].label = Some("changed".into());
        assert!(crate::ipc::snapshot::diff(&snap, &changed, None, None).needs_full);
        let wire = serde_json::to_value(&snap).unwrap();
        assert_eq!(wire["sessions"][0]["pending_steer"][0], "queued [Image #1]");
        assert_eq!(
            wire["sessions"][0]["pending_steer_attachments"][0][0]["marker_n"],
            1
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut client = None;
        super::super::apply_action(
            crate::controller::input::Action::EditSteer(0),
            &mut state,
            &mut client,
            rt.handle(),
        )
        .unwrap();
        assert_eq!(state.rest.fg().input, "queued [Image #1]");
        assert_eq!(
            state.rest.fg().pending_attachments,
            vec![image(2), image(1)]
        );
        state
            .rest
            .fg_mut()
            .pending_steer
            .push("another".to_string().into());
        super::super::apply_action(
            crate::controller::input::Action::CancelSteers,
            &mut state,
            &mut client,
            rt.handle(),
        )
        .unwrap();
        assert_eq!(state.rest.fg().pending_attachments.len(), 2);
        let mut legacy = wire;
        legacy["sessions"][0]
            .as_object_mut()
            .unwrap()
            .remove("pending_steer_attachments");
        let restored: crate::ipc::proto::StateSnapshot = serde_json::from_value(legacy).unwrap();
        assert!(restored.sessions[0].pending_steer_attachments.is_empty());
    }
    #[test]
    fn followup_delivery_expands_pastes_and_persists_media() {
        let dir = std::env::temp_dir().join(format!("koma-followup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("pastes")).unwrap();
        std::fs::write(dir.join("pastes/1.txt"), "Complete pasted body 🐈").unwrap();
        let mut rt = SessionRuntime::new();
        rt.session = Some(crate::model::session::Session::new(
            "followup".into(),
            dir.clone(),
            "test".into(),
            Default::default(),
            crate::model::conversation::Conversation::new(""),
        ));
        let paste = Attachment {
            kind: AttachmentKind::PastedText,
            marker_n: 1,
            rel_path: "pastes/1.txt".into(),
            mime: "text/plain".into(),
            label: None,
        };
        rt.pending_steer.push(FollowUp {
            text: "first [Image #1]".into(),
            attachments: vec![image(1)],
        });
        rt.pending_steer.push(FollowUp {
            text: "second [Pasted Text #1]".into(),
            attachments: vec![paste],
        });
        rt.deliver_followups();
        assert!(rt.pending_steer.is_empty());
        let sess = rt.session.as_ref().unwrap();
        let message = sess.conversation.messages().last().unwrap();
        assert!(message.content.contains("Complete pasted body 🐈"));
        assert_eq!(message.attachments.len(), 2);
        assert_eq!(
            crate::model::session::Session::load(&dir)
                .unwrap()
                .conversation
                .messages()
                .last()
                .unwrap()
                .attachments,
            message.attachments
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

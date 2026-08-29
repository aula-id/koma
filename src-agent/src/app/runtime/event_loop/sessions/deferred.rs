use std::sync::Arc;

use crate::app::state::{AppState, EXT_TURN_BUDGET};
use crate::service::openrouter::OpenRouterClient;

use super::super::super::stream::resume_after_subagents;

/// Drain the deferred tool-task lane (`tool_task_rx`) and the user-shell lane
/// (`shell_task_rx`), then fire the resume gate when both deferred lanes are
/// empty. Returns true if anything changed.
pub(super) fn drain_deferred_and_resume(
    state: &mut AppState,
    idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    let mut dirty = false;

    // --- drain deferred tool-task results (heavy/blocking tools) ---
    // Deferred tools (read/write/edit/delete/grep/glob/remember/
    // web_fetch/web_search; bash is a BashJob lane, not this channel) run on a plain
    // std::thread (spawned in `dispatch_deferred`) and send their `(call_id, result)`
    // back over `tool_task_rx`. Fold each into the PARKED round's `tool_results` and
    // drop its id from `pending_tool_tasks`, exactly mirroring the sub-agent deferral
    // — so the resume gate below sees the settled set. Done within this same
    // block (before the gate) so both lanes' results are in place when emptiness
    // is tested. A round runs its deferred tools ONE AT A TIME, so at most one
    // id settles here per resume. (FG bash also parks on `pending_tool_tasks` but
    // delivers via the bash_done branch below, not this channel.)
    {
        // Drain into a local vec FIRST inside a narrow scope so the `rx` borrow
        // of this session's runtime is released before we touch
        // `pending_tool_tasks` / `tool_results` on the same runtime below.
        let mut received: Vec<(String, String)> = Vec::new();
        if let Some(rx) = state.rest.sessions[idx].tool_task_rx.as_mut() {
            while let Ok(pair) = rx.try_recv() {
                received.push(pair);
            }
        }
        // Fold only results whose id is still in pending_tool_tasks; anything
        // else is a stale delivery from a killed/interrupted turn and must be
        // discarded rather than corrupting the next turn.
        for (id, result) in received {
            if let Some(pos) = state.rest.sessions[idx]
                .pending_tool_tasks
                .iter()
                .position(|c| c == &id)
            {
                state.rest.sessions[idx].pending_tool_tasks.remove(pos);
                state.rest.sessions[idx].tool_results.push((id, result));
                dirty = true;
            }
            // else: stale delivery — drop silently
        }
    }

    // --- drain `!` user-shell results (off-thread, independent lane) ---
    // A `!`-shortcut command runs the blocking `run_shell_capture` on a plain
    // std::thread (spawned in `actions::chat::handle_shell`) and sends its
    // `(command, captured_output)` back over `shell_task_rx`. Folding it here —
    // not inline in the handler — is what keeps the event loop (and so every
    // session) responsive for the whole command duration. Build the distinct
    // SHELL_MARK entry (a `$ <cmd>` block over dim output that the wire builder
    // strips to clean `$ <cmd>\n<output>` context for the model), append it to
    // the conversation + msglog, and clear the park. Status/scroll updates are
    // FOREGROUND-ONLY so a background session's shell finishing can't yank the
    // viewed transcript. Only fold while `awaiting_shell` is set; a delivery
    // after a close/clear is stale and dropped.
    {
        // Drain into a local FIRST inside a narrow scope so the `rx` borrow of
        // this session's runtime is released before the session/conversation
        // writes below. At most one `!` runs per session at a time (the busy
        // guard), so this is normally zero or one pair.
        let mut shell_results: Vec<(String, String)> = Vec::new();
        if state.rest.sessions[idx].awaiting_shell {
            if let Some(rx) = state.rest.sessions[idx].shell_task_rx.as_mut() {
                while let Ok(pair) = rx.try_recv() {
                    shell_results.push(pair);
                }
            }
        }
        for (cmd, output) in shell_results {
            // Invisible SHELL_MARK so the transcript renders this as a `$ <cmd>`
            // block (not a `★` user turn); the visible `$ <cmd>\n<output>` body is
            // what the model reads (the mark is stripped on the wire).
            let content = format!("{}$ {cmd}\n{output}", crate::dto::chat::SHELL_MARK);
            if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
                let _ = crate::model::msglog::append(
                    &sess.path,
                    crate::dto::chat::Role::User,
                    &content,
                    None,
                    None,
                );
                sess.conversation.push_user(content);
                let _ = sess.save();
            }
            // Park ends: a fresh `!`/Submit is allowed again.
            state.rest.sessions[idx].awaiting_shell = false;
            // Snap THIS session's OWN view to the newest line (C2): scroll is per-session,
            // so resetting `sessions[idx]`'s scroll when its own shell output lands is
            // correct regardless of which client is the acting cursor — the client viewing
            // `idx` sees the snap-to-bottom, an unrelated session's view is untouched.
            state.rest.reset_scroll_at(idx);
            // Status is per-session now (C6): write it on `sessions[idx]` itself. The
            // projection reads `fg().status` after the per-client foreground swap, so this
            // surfaces ONLY in the client(s) viewing `idx` — a background session's
            // `!`-shell completion can no longer yank an unrelated window's status line.
            let exit_line = output.lines().last().unwrap_or("done");
            state.rest.sessions[idx].status = format!("$ {cmd} — {exit_line}");
            dirty = true;
        }
    }

    // --- drain bash COMPLETION signals ---
    // Every model `bash` (FG or BG) runs as a BashJob. When the worker exits it
    // fires the job id over `bash_done_tx`. Branch on remaining `tool_call_id`:
    //   Some → still-blocking FG: deliver as tool_result (not nudge), clear park
    //   None → true BG or already promoted: toast + pending_bash_nudges
    // Race with Ctrl+B: both paths `take()` tool_call_id — only one wins.
    {
        // Drain the finished ids into a local FIRST so the `rx` borrow of this
        // session's runtime is released before we look the jobs back up below.
        let mut finished: Vec<usize> = Vec::new();
        if let Some(rx) = state.rest.sessions[idx].bash_done_rx.as_mut() {
            while let Ok(id) = rx.try_recv() {
                finished.push(id);
            }
        }
        let had_finished = !finished.is_empty();
        for id in finished {
            // Find job index; skip unknown ids (cleared session).
            let Some(job_idx) = state.rest.sessions[idx]
                .bash_jobs
                .iter()
                .position(|j| j.id == id)
            else {
                continue;
            };

            // take() the call id — races promote; only one path delivers the result.
            let call_id = state.rest.sessions[idx].bash_jobs[job_idx]
                .tool_call_id
                .take();
            let suppress = state.rest.sessions[idx].bash_jobs[job_idx].suppress_completion_nudge;

            if let Some(call_id) = call_id {
                // Still blocking FG — deliver as tool result, not nudge.
                let saving = state.rest.sessions[idx]
                    .session
                    .as_ref()
                    .map(|s| s.settings.bash_saving)
                    .unwrap_or(true);
                let log_dir = state.rest.sessions[idx]
                    .session
                    .as_ref()
                    .map(|s| s.path.join("opt"));
                let result = state.rest.sessions[idx].bash_jobs[job_idx]
                    .format_tool_result(saving, log_dir.as_deref());

                // Only fold if still pending (Esc may have cleared the park set).
                if let Some(pos) = state.rest.sessions[idx]
                    .pending_tool_tasks
                    .iter()
                    .position(|c| c == &call_id)
                {
                    state.rest.sessions[idx].pending_tool_tasks.remove(pos);
                    state.rest.sessions[idx]
                        .tool_results
                        .push((call_id, result));
                    dirty = true;
                }
                // else: turn was interrupted — drop the result silently
            } else if !suppress {
                // True BG or already promoted — existing nudge path.
                let label = match state.rest.sessions[idx].bash_jobs[job_idx].snapshot_status() {
                    crate::app::bgbash::BashJobStatus::Running => "running".to_string(),
                    crate::app::bgbash::BashJobStatus::Done(code) => format!("exit {code}"),
                    crate::app::bgbash::BashJobStatus::Killed => "killed".to_string(),
                    crate::app::bgbash::BashJobStatus::Error(msg) => format!("error: {msg}"),
                };
                // The bg-bash completion toast is about THIS session's job, and the drain
                // runs unbracketed (fg() is stale scratch here), so raise it on
                // `sessions[idx]` itself (C6) — it surfaces only in the client(s) viewing idx.
                state.rest.sessions[idx].set_toast_info(format!("bash-{id} finished: {label}"));
                state.rest.sessions[idx]
                    .pending_bash_nudges
                    .push((id, label));
                dirty = true;
            }
            // else: suppress_completion_nudge (Esc killed still-blocking FG) — no nudge
        }
        // A job just reached a terminal state — re-persist the set so the restored
        // record reflects the final status, not the stale "running" (#25).
        if had_finished {
            crate::app::runtime::bg_persist::persist_bash_jobs(&state.rest.sessions[idx]);
        }
    }

    // --- resume a round parked on deferred work (BOTH lanes) ---
    // Unpark only when EVERY deferred id — sub-agent delegations AND deferred
    // tool tasks — has filled its result (above). The resume
    // (`resume_after_subagents`) RE-ENTERS `process_tools` at the advanced
    // `tool_idx` to CONTINUE the round: a deferred heavy tool dispatched the NEXT
    // call (and may park again), making the lane SEQUENTIAL; once the round has
    // no further deferred work it falls through to `finish_tool_round`, which
    // flushes ALL collected `tool_results` and re-streams so the MAIN AGENT
    // reacts. Clearing both awaiting flags drops the parked status; `waiting`
    // stays true through the re-stream. Gating on both lists means a mixed round
    // waits for the last pending id of either kind before resuming — no dangling
    // tool_call ids.
    if (state.rest.sessions[idx].awaiting_subagents || state.rest.sessions[idx].awaiting_tool_tasks)
        && state.rest.sessions[idx].pending_subagent_calls.is_empty()
        && state.rest.sessions[idx].pending_tool_tasks.is_empty()
    {
        state.rest.sessions[idx].awaiting_subagents = false;
        state.rest.sessions[idx].awaiting_tool_tasks = false;
        resume_after_subagents(state, idx, client, handle);
        dirty = true;
    }

    // --- drain TAC-classify verdicts + resume the parked round ---
    // A risky tool call parked on the classifier (see `approval::process_tools`)
    // spawned an off-thread classify task that sends its `(call_id, verdict)` back
    // over `classify_rx`. Stage the verdict for the PARKED call (its id must match
    // the call at `tool_idx`) and clear the park, then RE-ENTER `process_tools`,
    // which consumes the staged verdict via the SAME three-way branch the old
    // inline `block_on` drove — only now the 1-12s classify never froze the event
    // loop. A verdict for any other id (a stale delivery from an interrupted /
    // superseded turn), or one arriving when nothing is parked, is dropped. This is
    // its OWN resume gate, separate from the deferred-work gate above: a classify
    // park and a tool-task/sub-agent park never coexist (the classifier gate runs
    // BEFORE dispatch/delegation), so the two gates can't double-resume a round.
    {
        // Narrow scope for the `rx` borrow (mirrors the tool-task drain above) so it
        // is released before we touch `pending_classify_verdict` / re-enter
        // `process_tools` on the same runtime.
        let mut received: Vec<(String, crate::app::harness::Verdict)> = Vec::new();
        if let Some(rx) = state.rest.sessions[idx].classify_rx.as_mut() {
            while let Ok(pair) = rx.try_recv() {
                received.push(pair);
            }
        }
        let mut resume_classify = false;
        for (call_id, verdict) in received {
            // Match only a verdict for the call this round is PARKED on. Once one
            // matches, `awaiting_classify` flips false, so a second delivery in the
            // same drain can never double-stage.
            let matches_parked = state.rest.sessions[idx].awaiting_classify
                && state.rest.sessions[idx]
                    .pending_tool_calls
                    .get(state.rest.sessions[idx].tool_idx)
                    .map(|c| c.id == call_id)
                    .unwrap_or(false);
            if matches_parked {
                state.rest.sessions[idx].pending_classify_verdict = Some((call_id, verdict));
                state.rest.sessions[idx].awaiting_classify = false;
                resume_classify = true;
                dirty = true;
            }
            // else: stale/mismatched, or nothing parked — drop silently
        }
        if resume_classify {
            resume_after_subagents(state, idx, client, handle);
            dirty = true;
        }
    }

    // --- bg-bash completion NUDGE: inject + auto-wake when idle ---
    // A finished bg-bash job is buffered in `pending_bash_nudges` (above). The
    // moment this session is idle (no turn in flight, nothing parked, no running
    // sub-agents) we drain the whole buffer into ONE synthetic user turn so the
    // model REACTS to the completion(s). While busy we leave the buffer untouched
    // and re-check on a later tick — so we never inject mid-turn (which would
    // corrupt tool_call/tool_result ordering). Auto-wake mirrors `handle_submit`:
    // begin_stream + waiting + the per-turn resets, then `start_stream_task`.
    if !state.rest.sessions[idx].pending_bash_nudges.is_empty()
        && !state.rest.sessions[idx].is_working()
        && client.is_some()
        && state.rest.sessions[idx].session.is_some()
    {
        let nudges = std::mem::take(&mut state.rest.sessions[idx].pending_bash_nudges);
        // Line 1 = terse per-job summary shown in the transcript (a dim green-✓
        // line). Lines 2+ = model-only context, hidden from the transcript and
        // stripped of the mark on the wire. The leading BASH_NUDGE_MARK is what
        // makes the transcript render this compactly instead of as a `★` turn.
        let summary = nudges
            .iter()
            .map(|(id, label)| format!("[bash-{id}] {label}"))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        let body = format!(
            "{}{summary}\nbackground bash job(s) finished \u{2014} read full output with bash_output if needed; react only if action is required, otherwise acknowledge briefly.",
            crate::dto::chat::BASH_NUDGE_MARK,
        );

        // Append as a USER turn (so the model treats it as input to respond to),
        // persist to msglog + messages.json, then capture history for the wire.
        let sess = match state.rest.sessions[idx].session.as_mut() {
            Some(s) => s,
            None => return dirty,
        };
        let _ = crate::model::msglog::append(
            &sess.path,
            crate::dto::chat::Role::User,
            &body,
            None,
            None,
        );
        sess.conversation.push_user(body);
        let _ = sess.save();
        let history = sess.conversation.history();

        // Per-turn reset + start stream, mirroring handle_submit's kickoff. The
        // session is idle here, so these are clean-state resets (defensive).
        {
            let rt = &mut state.rest.sessions[idx];
            rt.begin_stream();
            rt.waiting = true;
            rt.agent_steps = 0;
            rt.pending_tool_calls.clear();
            rt.awaiting_approval = false;
            rt.tool_idx = 0;
            rt.tool_results.clear();
            rt.pending_tool_tasks.clear();
            rt.awaiting_tool_tasks = false;
            rt.awaiting_classify = false;
            rt.pending_classify_verdict = None;
        }
        // Snap THIS session's OWN view to the newest line as its auto-wake stream starts
        // (C2): scroll is per-session, so this only affects `sessions[idx]` — a client
        // viewing `idx` sees the snap-to-bottom, an unrelated session's view is untouched.
        state.rest.reset_scroll_at(idx);
        // Status is per-session now (C6): write "thinking" on `sessions[idx]` itself. The
        // projection sources `fg().status` per client, so a background auto-wake only
        // shows in the client(s) viewing idx — never overwriting another window's status.
        state.rest.sessions[idx].status = "thinking".into();
        super::super::super::stream::start_stream_task(history, state, idx, client, handle);
        dirty = true;
    }

    // --- detached sub-agent completion NUDGE: inject + auto-wake when idle ---
    // A DETACHED (`task` `run_in_background`) sub-agent that reached a terminal
    // state is buffered in `pending_subagent_nudges` (filled once per agent by
    // `drain_subagents`). Exactly like the bg-bash nudge above, the moment this
    // session is idle (no turn in flight, nothing parked, no running sub-agents)
    // we drain the whole buffer into ONE synthetic user turn so the model REACTS
    // to the completion(s). While busy we leave the buffer untouched and re-check
    // on a later tick — so we never inject mid-turn (which would corrupt
    // tool_call/tool_result ordering). `is_working()` returns true while any
    // sub-agent is Running, so a still-running detached agent can never trip this.
    // Auto-wake mirrors `handle_submit`: begin_stream + waiting + the per-turn
    // resets, then `start_stream_task`.
    // Ordering-dependent invariant: the bg-bash nudge block above sets waiting=true
    // when it fires, and is_working() subsumes waiting — so if a bash nudge already
    // kicked off a stream this tick, the gate below is false and this block defers to
    // the next tick. The two background-completion nudges therefore never double-launch
    // a stream in the same tick. Do NOT reorder these blocks without preserving that.
    if !state.rest.sessions[idx].pending_subagent_nudges.is_empty()
        && !state.rest.sessions[idx].is_working()
        && client.is_some()
        && state.rest.sessions[idx].session.is_some()
    {
        let nudges = std::mem::take(&mut state.rest.sessions[idx].pending_subagent_nudges);
        // The leading BASH_NUDGE_MARK keeps the transcript renderer compact
        // (dim green-✓ line) while making the full reports available to the model.
        // Each finished agent's complete report is injected verbatim — no polling
        // needed; the model receives the result directly, exactly as a blocking
        // sub-agent delivers its tool result.
        let mut body = String::from(crate::dto::chat::BASH_NUDGE_MARK);
        for (id, agent, report) in &nudges {
            body.push_str(&format!(
                "background sub-agent #{id} ({agent}) finished — its full report:\n\n{report}\n\n"
            ));
        }
        body.push_str("React only if action is required; otherwise acknowledge briefly.");

        // Append as a USER turn (so the model treats it as input to respond to),
        // persist to msglog + messages.json, then capture history for the wire.
        let sess = match state.rest.sessions[idx].session.as_mut() {
            Some(s) => s,
            None => return dirty,
        };
        let _ = crate::model::msglog::append(
            &sess.path,
            crate::dto::chat::Role::User,
            &body,
            None,
            None,
        );
        sess.conversation.push_user(body);
        let _ = sess.save();
        let history = sess.conversation.history();

        // Per-turn reset + start stream, mirroring handle_submit's kickoff. The
        // session is idle here, so these are clean-state resets (defensive).
        {
            let rt = &mut state.rest.sessions[idx];
            rt.begin_stream();
            rt.waiting = true;
            rt.agent_steps = 0;
            rt.pending_tool_calls.clear();
            rt.awaiting_approval = false;
            rt.tool_idx = 0;
            rt.tool_results.clear();
            rt.pending_tool_tasks.clear();
            rt.awaiting_tool_tasks = false;
            rt.awaiting_classify = false;
            rt.pending_classify_verdict = None;
        }
        state.rest.reset_scroll_at(idx);
        state.rest.sessions[idx].status = "thinking".into();
        super::super::super::stream::start_stream_task(history, state, idx, client, handle);
        dirty = true;
    }

    // --- SDLC LLM-keeper result poll: drain async classify result + stage inject ---
    // Non-blocking poll of the oneshot receiver. Result lands in
    // `pending_sdlc_keeper_llm` for the inject block below to drain when idle.
    // Epoch-guarded: results spawned under a prior epoch (exit / phase / hash
    // change) are dropped so they cannot inject a non-SDLC or wrong-phase turn.
    if state.rest.sessions[idx].sdlc_keeper_llm_rx.is_some() {
        let mut finished: Option<(u64, Option<String>)> = None;
        let mut closed = false;
        if let Some(rx) = state.rest.sessions[idx].sdlc_keeper_llm_rx.as_mut() {
            match rx.try_recv() {
                Ok(opt) => finished = Some(opt),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => closed = true,
            }
        }
        if finished.is_some() || closed {
            state.rest.sessions[idx].sdlc_keeper_llm_rx = None;
            state.rest.sessions[idx].sdlc_keeper_llm_inflight = false;
            if let Some((epoch, inject)) = finished {
                let current = state.rest.sessions[idx].sdlc_keeper_epoch;
                let still_sdlc =
                    state.rest.sessions[idx].agent_mode == crate::app::state::AgentMode::Sdlc;
                let phase_ok = matches!(
                    state.rest.sessions[idx].sdlc_phase.as_deref(),
                    Some("execute") | Some("integrate") | Some("prepare")
                );
                if epoch == current && still_sdlc && phase_ok {
                    if let Some(inject) = inject {
                        state.rest.sessions[idx].pending_sdlc_keeper_llm = Some(inject);
                    }
                }
                // else: stale — drop on the floor
            }
        }
    }
    // Drain pending LLM keeper inject when idle — same inject path as deterministic.
    // Re-check mode/phase so a staged inject cannot fire after SDLC exit.
    if !state.rest.sessions[idx].is_working()
        && client.is_some()
        && state.rest.sessions[idx].session.is_some()
        && state.rest.sessions[idx].agent_mode == crate::app::state::AgentMode::Sdlc
        && matches!(
            state.rest.sessions[idx].sdlc_phase.as_deref(),
            Some("execute") | Some("integrate") | Some("prepare")
        )
    {
        if let Some(inject) = state.rest.sessions[idx].pending_sdlc_keeper_llm.take() {
            let body = format!("{}{inject}", crate::dto::chat::BASH_NUDGE_MARK);
            if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
                let _ = crate::model::msglog::append(
                    &sess.path,
                    crate::dto::chat::Role::User,
                    &body,
                    None,
                    None,
                );
                sess.conversation.push_user(body);
                let _ = sess.save();
                sess.rebuild_system();
                let _ = sess.save();
                let history = sess.conversation.history();
                {
                    let rt = &mut state.rest.sessions[idx];
                    rt.begin_stream();
                    rt.waiting = true;
                    rt.agent_steps = 0;
                    rt.pending_tool_calls.clear();
                    rt.awaiting_approval = false;
                    rt.tool_idx = 0;
                    rt.tool_results.clear();
                    rt.pending_tool_tasks.clear();
                    rt.awaiting_tool_tasks = false;
                    rt.awaiting_classify = false;
                    rt.pending_classify_verdict = None;
                }
                state.rest.reset_scroll_at(idx);
                state.rest.sessions[idx].status = "thinking".into();
                super::super::super::stream::start_stream_task(history, state, idx, client, handle);
                dirty = true;
            }
        }
    }

    // --- SDLC keeper: false-done reopen + integrate nudge on idle ---
    // Armed after mission approve and each finished tool round while in Sdlc.
    // Runs only when idle so we never inject mid-tool-round. Dedupe lives inside
    // keeper::evaluate (mission_meta hash). If bash/subagent already woke this
    // tick, is_working() is true and we defer — same single-wake invariant.
    //
    // When deterministic has nothing to say, optionally spawn ONE Safeguard
    // oneshot (classifier_enabled only) for stalled/dishonest progress. Spawn is
    // gated on the same `sdlc_keeper_due` edge so we never thrash every idle tick.
    if state.rest.sessions[idx].agent_mode == crate::app::state::AgentMode::Sdlc
        && state.rest.sessions[idx].sdlc_keeper_due
        && !state.rest.sessions[idx].is_working()
        && client.is_some()
        && state.rest.sessions[idx].session.is_some()
    {
        state.rest.sessions[idx].sdlc_keeper_due = false;
        let sess_path = state.rest.sessions[idx]
            .session
            .as_ref()
            .map(|s| s.path.clone());
        if let Some(path) = sess_path {
            let report = crate::model::sdlc::keeper::evaluate(&path);
            // Handle typed reassessment action: fail-closed disk mutation + runtime assess.
            // The action is produced when the keeper detects an invalid contract hash,
            // missing graph hash, or lost mission binding during an active phase.
            if let Some(crate::model::sdlc::keeper::KeeperAction::RequireReassessment {
                ref reason,
            }) = report.action
            {
                let reassess_note = format!("keeper reassessment: {reason}");
                let mut disk_ok = false;
                if let Some(mut m) = crate::model::sdlc::Mission::load(&path) {
                    if m.approved && matches!(m.phase.as_str(), "execute" | "integrate" | "prepare")
                    {
                        m.approved = false;
                        m.needs_reapproval = true;
                        m.amendment_note = Some(reassess_note);
                        if state
                            .rest
                            .apply_sdlc_phase_with_mission(idx, &mut m, "assess")
                            .is_ok()
                        {
                            disk_ok = true;
                        }
                    }
                }
                // Make runtime assess even if disk persistence fails.
                if !disk_ok {
                    state.rest.force_sdlc_assess_safe(idx);
                }
            }
            if !report.reopened.is_empty() {
                state.rest.sessions[idx].set_toast_info(format!(
                    "SDLC keeper reopened {} false-done task(s)",
                    report.reopened.len()
                ));
                dirty = true;
            }
            if let Some(inject) = report.inject {
                let body = format!("{}{inject}", crate::dto::chat::BASH_NUDGE_MARK,);
                let sess = match state.rest.sessions[idx].session.as_mut() {
                    Some(s) => s,
                    None => return dirty,
                };
                let _ = crate::model::msglog::append(
                    &sess.path,
                    crate::dto::chat::Role::User,
                    &body,
                    None,
                    None,
                );
                sess.conversation.push_user(body);
                let _ = sess.save();
                // Rebuild so OPEN/SEALED after reopen is visible next turn.
                sess.rebuild_system();
                let _ = sess.save();
                let history = sess.conversation.history();
                {
                    let rt = &mut state.rest.sessions[idx];
                    rt.begin_stream();
                    rt.waiting = true;
                    rt.agent_steps = 0;
                    rt.pending_tool_calls.clear();
                    rt.awaiting_approval = false;
                    rt.tool_idx = 0;
                    rt.tool_results.clear();
                    rt.pending_tool_tasks.clear();
                    rt.awaiting_tool_tasks = false;
                    rt.awaiting_classify = false;
                    rt.pending_classify_verdict = None;
                }
                state.rest.reset_scroll_at(idx);
                state.rest.sessions[idx].status = "thinking".into();
                super::super::super::stream::start_stream_task(history, state, idx, client, handle);
                dirty = true;
            } else if !state.rest.sessions[idx].sdlc_keeper_llm_inflight
                && state.rest.sessions[idx].pending_sdlc_keeper_llm.is_none()
                && state.rest.sessions[idx]
                    .session
                    .as_ref()
                    .map(|s| s.settings.classifier_enabled)
                    .unwrap_or(false)
                && matches!(
                    state.rest.sessions[idx].sdlc_phase.as_deref(),
                    Some("execute") | Some("integrate") | Some("prepare")
                )
            {
                // Deterministic keeper quiet — optional second-model review.
                let client = match client.clone() {
                    Some(c) => c,
                    None => return dirty,
                };
                let config = state.rest.config.clone();
                let settings = match state.rest.sessions[idx]
                    .session
                    .as_ref()
                    .map(|s| s.settings.clone())
                {
                    Some(s) => s,
                    None => return dirty,
                };
                let (tx, rx) = tokio::sync::oneshot::channel();
                let epoch_at_spawn = state.rest.sessions[idx].sdlc_keeper_epoch;
                state.rest.sessions[idx].sdlc_keeper_llm_rx = Some(rx);
                state.rest.sessions[idx].sdlc_keeper_llm_inflight = true;
                handle.spawn(async move {
                    let inject = async {
                        let mission = match crate::model::sdlc::Mission::load(&path) {
                            Some(m) if m.approved => m,
                            _ => return None,
                        };
                        let conn = crate::model::msglog::open(&path).ok()?;
                        let _ = crate::model::sdlc::graph::ensure_tables(&conn);
                        let open = crate::model::sdlc::graph::list_open(&conn).unwrap_or_default();
                        let sealed =
                            crate::model::sdlc::graph::list_sealed(&conn).unwrap_or_default();
                        let messages =
                            crate::model::sdlc::keeper::llm_keeper_prompt(&mission, &open, &sealed);
                        let verdict = crate::app::harness::classify(
                            &client, &config, &settings, messages, true,
                        )
                        .await;
                        if !verdict.available {
                            return None;
                        }
                        let reply = serde_json::json!({
                            "allow": verdict.allow,
                            "reason": verdict.reason,
                        })
                        .to_string();
                        crate::model::sdlc::keeper::llm_verdict_to_inject(&reply)
                    }
                    .await;
                    let _ = tx.send((epoch_at_spawn, inject));
                });
            }
        }
    }

    // --- SDLC historian: poll async summary result + spawn when idle ---
    // Non-blocking poll of the oneshot receiver. Result persists as an edit_summary
    // event so the capsule can project it. Epoch-guarded like the keeper LLM.
    if state.rest.sessions[idx].sdlc_historian_rx.is_some() {
        let mut finished: Option<(u64, Option<String>)> = None;
        let mut closed = false;
        if let Some(rx) = state.rest.sessions[idx].sdlc_historian_rx.as_mut() {
            match rx.try_recv() {
                Ok(opt) => finished = Some(opt),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => closed = true,
            }
        }
        if finished.is_some() || closed {
            state.rest.sessions[idx].sdlc_historian_rx = None;
            state.rest.sessions[idx].sdlc_historian_inflight = false;
            if let Some((epoch, summary_json)) = finished {
                let current = state.rest.sessions[idx].sdlc_historian_epoch;
                let still_sdlc =
                    state.rest.sessions[idx].agent_mode == crate::app::state::AgentMode::Sdlc;
                if epoch == current && still_sdlc {
                    // Consume the pending batch (it was moved into the task).
                    if let Some(batch) =
                        state.rest.sessions[idx].pending_sdlc_historian_batch.take()
                    {
                        if let Some(ref inject) = summary_json {
                            if let Some(rec) = crate::model::sdlc::history::parse_historian_reply(
                                inject,
                                &batch.batch_id,
                                batch.node_id.as_deref(),
                                batch.paths.clone(),
                            ) {
                                // Best-effort persist the summary event.
                                if let Some(path) = state.rest.sessions[idx]
                                    .session
                                    .as_ref()
                                    .map(|s| s.path.clone())
                                {
                                    if let Ok(conn) = crate::model::msglog::open(&path) {
                                        let _ = crate::model::sdlc::graph::ensure_tables(&conn);
                                        crate::model::sdlc::graph::append_edit_summary(
                                            &conn,
                                            rec.node_id.as_deref(),
                                            &rec,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                // else: stale — drop on the floor
            }
        }
    }
    // Spawn historian when idle, batch pending, and no inflight historian.
    // Uses the Awareness role — cheap secondary free-form completion.
    if state.rest.sessions[idx].agent_mode == crate::app::state::AgentMode::Sdlc
        && state.rest.sessions[idx]
            .pending_sdlc_historian_batch
            .is_some()
        && !state.rest.sessions[idx].sdlc_historian_inflight
        && !state.rest.sessions[idx].is_working()
        && client.is_some()
        && state.rest.sessions[idx].session.is_some()
    {
        let config = state.rest.config.clone();
        let settings = match state.rest.sessions[idx]
            .session
            .as_ref()
            .map(|s| s.settings.clone())
        {
            Some(s) => s,
            None => return dirty,
        };
        let Some(route) = crate::app::resolve::resolve_role_dispatch(
            &config,
            &settings,
            crate::model::app_config::ModelRole::Awareness,
        )
        .filter(|r| r.is_routable()) else {
            // No Awareness route — drop the pending batch silently (best-effort).
            state.rest.sessions[idx].pending_sdlc_historian_batch = None;
            return dirty;
        };
        let client = match client.clone() {
            Some(c) => c,
            None => return dirty,
        };
        let Some(batch) = state.rest.sessions[idx]
            .pending_sdlc_historian_batch
            .clone()
        else {
            return dirty;
        };
        let epoch_at_spawn = state.rest.sessions[idx].sdlc_historian_epoch;
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.rest.sessions[idx].sdlc_historian_rx = Some(rx);
        state.rest.sessions[idx].sdlc_historian_inflight = true;
        handle.spawn(async move {
            let summary = async {
                let audits: Vec<crate::model::sdlc::graph::EditAuditRecord> = batch
                    .paths
                    .iter()
                    .map(|p| crate::model::sdlc::graph::EditAuditRecord {
                        tool: "edit".into(),
                        path: p.clone(),
                        node_id: batch.node_id.clone(),
                        batch_id: batch.batch_id.clone(),
                    })
                    .collect();
                let (system, user) = crate::model::sdlc::history::build_historian_prompt(
                    &batch.mission_goal,
                    &batch.mission_phase,
                    batch.node_title.as_deref(),
                    &audits,
                );
                let messages = vec![
                    crate::dto::chat::ChatMessage::new(crate::dto::chat::Role::System, &system),
                    crate::dto::chat::ChatMessage::new(crate::dto::chat::Role::User, &user),
                ];
                client
                    .complete_with(
                        route.conn(),
                        &route.model_id,
                        route.provider(),
                        messages,
                        true,
                    )
                    .await
                    .ok()
            }
            .await;
            let _ = tx.send((epoch_at_spawn, summary));
        });
    }

    // --- extension-prompt injection: inject + auto-wake when idle ---
    // Extensions BUFFER `chat.prompt` texts into `pending_ext_prompts` via the grant
    // broker (buffer-only — the broker NEVER injects). Exactly like the two
    // background-completion nudges above, the moment this session is idle we drain
    // the WHOLE buffer into ONE synthetic user turn so the model acts on the
    // extensions' prompts as user requests. While busy we leave the buffer untouched
    // and re-check next tick — never injecting mid-turn (which would corrupt
    // tool_call/tool_result ordering).
    //
    // LOOP-GUARD semantics:
    // - IDLE-ONLY: `is_working()` subsumes `waiting`, so a turn already in flight —
    //   INCLUDING one the bash/subagent nudge blocks above just kicked off THIS tick
    //   (they set `waiting = true`) — makes this gate false and we defer to a later
    //   tick. Same never-double-launch-a-stream invariant the subagent block above
    //   documents: at most ONE auto-wake stream starts per tick, and this block, being
    //   LAST, defers to either background-completion nudge that already fired.
    // - CAP-5 LOAD-BEARING: the broker caps the buffer at 5. An extension subscribed
    //   to `agent.turn_end` that re-prompts on every turn-end converges to at most ONE
    //   buffered prompt per turn (its prompt drains here; it re-buffers one on the
    //   resulting turn_end; that drains next idle) — a steady state of ≤1 turn per
    //   prompt. It CANNOT amplify into a runaway loop; the cap is the hard ceiling if
    //   several extensions prompt at once.
    // - CONSECUTIVE-DUP DEDUPE: the broker refuses a prompt identical to the buffer's
    //   last entry, so an extension resending the same text can't fill the buffer.
    // - TURN BUDGET (cost-DoS guard, review finding): additionally gated on
    //   `ext_injected_turns < EXT_TURN_BUDGET`. This is the belt-and-braces half of
    //   the pair with `broker_chat_prompt`'s own budget check — a prompt buffered
    //   just BEFORE the budget tripped would otherwise still get injected here even
    //   though the broker would now refuse a fresh one. Once the budget is exhausted
    //   the buffer stays parked (not dropped) until a real user turn resets the
    //   counter (see `SessionRuntime::ext_injected_turns` / `actions::chat::handle_submit`).
    //   The toast block just below fires once when that park begins.
    if !state.rest.sessions[idx].pending_ext_prompts.is_empty()
        && !state.rest.sessions[idx].is_working()
        && state.rest.sessions[idx].ext_injected_turns == EXT_TURN_BUDGET
    {
        // One-shot: the FIRST idle tick the budget is found EXACTLY exhausted (not
        // `>=`) with prompts still buffered pops the toast, then nudges the counter
        // past budget so this can't re-fire every tick while parked. A real user
        // turn resets the counter to 0 (`handle_submit`), re-arming this for the
        // next park. Mirrors the existing `set_toast_info` pattern used elsewhere in
        // this file (the bg-bash / `!`-shell completion toasts above).
        state.rest.sessions[idx].set_toast_info(
            "extensions paused: turn budget reached — type anything to resume".to_string(),
        );
        state.rest.sessions[idx].ext_injected_turns = EXT_TURN_BUDGET + 1;
        dirty = true;
    }
    if ext_prompts_ready(
        !state.rest.sessions[idx].pending_ext_prompts.is_empty(),
        state.rest.sessions[idx].is_working(),
        client.is_some(),
        state.rest.sessions[idx].session.is_some(),
        state.rest.sessions[idx].ext_injected_turns,
    ) {
        let prompts = std::mem::take(&mut state.rest.sessions[idx].pending_ext_prompts);
        // Leading EXT_PROMPT_MARK → compact transcript render + wire strip. One
        // `[ext:<id>] <text>` line per buffered prompt, then a trailing instruction.
        let body = ext_prompt_body(&prompts);

        // Append as a USER turn (model input), persist to msglog + messages.json, then
        // capture history for the wire — mirrors the bash-nudge block above EXACTLY.
        let sess = match state.rest.sessions[idx].session.as_mut() {
            Some(s) => s,
            None => return dirty,
        };
        let _ = crate::model::msglog::append(
            &sess.path,
            crate::dto::chat::Role::User,
            &body,
            None,
            None,
        );
        sess.conversation.push_user(body);
        let _ = sess.save();
        let history = sess.conversation.history();

        // Per-turn reset + start stream, mirroring handle_submit's kickoff. The session
        // is idle here, so these are clean-state resets (defensive).
        {
            let rt = &mut state.rest.sessions[idx];
            rt.begin_stream();
            rt.waiting = true;
            rt.agent_steps = 0;
            rt.pending_tool_calls.clear();
            rt.awaiting_approval = false;
            rt.tool_idx = 0;
            rt.tool_results.clear();
            rt.pending_tool_tasks.clear();
            rt.awaiting_tool_tasks = false;
            rt.awaiting_classify = false;
            rt.pending_classify_verdict = None;
        }
        state.rest.reset_scroll_at(idx);
        state.rest.sessions[idx].status = "thinking".into();
        super::super::super::stream::start_stream_task(history, state, idx, client, handle);
        // Cost-DoS guard (review finding): count THIS injected turn AFTER the
        // successful kickoff, so the budget check above (and `broker_chat_prompt`'s
        // own check) sees an accurate consecutive-injection count on the next call.
        state.rest.sessions[idx].ext_injected_turns += 1;
        dirty = true;
    }

    dirty
}

/// Pure gate for the extension-prompt injection lane (mirrors the bash/subagent
/// nudge gates): drain-and-inject ONLY when the session is IDLE, has BOTH a client
/// and a live session to run the resulting turn, AND the consecutive-injection
/// budget (`injected_turns < EXT_TURN_BUDGET`, the cost-DoS guard — see
/// [`EXT_TURN_BUDGET`](crate::app::state::EXT_TURN_BUDGET)) is not yet exhausted.
/// `is_working` subsumes `waiting`, so a stream already kicked off this tick (by the
/// bash/subagent nudge blocks) keeps this false — the never-double-launch invariant.
/// Factored out so the loop-guard can be unit-tested without a live session/client
/// fixture.
fn ext_prompts_ready(
    has_prompts: bool,
    is_working: bool,
    has_client: bool,
    has_session: bool,
    injected_turns: u32,
) -> bool {
    has_prompts && !is_working && has_client && has_session && injected_turns < EXT_TURN_BUDGET
}

/// Build the injected user-turn body for buffered extension prompts: a leading
/// [`EXT_PROMPT_MARK`](crate::dto::chat::EXT_PROMPT_MARK) (compact transcript render
/// + wire strip), one `[ext:<id>] <text>` line per buffered prompt, then a trailing
///   instruction line telling the model these are extension-injected user requests.
///   Pure so the shape is unit-testable without the event loop.
fn ext_prompt_body(prompts: &[(String, String)]) -> String {
    let mut body = String::from(crate::dto::chat::EXT_PROMPT_MARK);
    let lines = prompts
        .iter()
        .map(|(ext_id, text)| format!("[ext:{ext_id}] {text}"))
        .collect::<Vec<_>>()
        .join("\n");
    body.push_str(&lines);
    body.push_str("\nThese prompts were injected by extensions; act on them as user requests.");
    body
}

/// Detect the working→ready edge for `idx` and emit a background-finish toast.
/// Also clears the sticky `finished_unseen` marker when the session is VIEWED BY SOME
/// client (C2: `state.rest.viewed_sessions`), not merely the transient foreground.
/// Updates `was_working` for the next tick.
/// Returns true if any state changed (toast or marker).
pub(super) fn nudge_background_finish(state: &mut AppState, idx: usize) -> bool {
    let mut dirty = false;

    // Is this session VIEWED BY SOME client this tick (C2)? Computed once up front so the
    // gates below test "viewed by ANY client" instead of the transient `foreground` cursor
    // (which is stale scratch in this per-tick service). A session viewed by NOBODY behaves
    // exactly like the old "not foreground" background session: it earns a finish toast +
    // sticky-unseen marker, and never has that marker cleared until some client views it.
    // The immutable borrow of `sessions[idx].id` + `viewed_sessions` is released here.
    let viewed = state
        .rest
        .sessions
        .get(idx)
        .map(|s| state.rest.viewed_sessions.contains(&s.id))
        .unwrap_or(false);

    // --- background-finish nudge ---
    // Detect this session's working→ready edge for THIS tick. When a session that
    // was working last tick is now idle AND is VIEWED BY NOBODY (so no client can
    // already see it finish), pop an info toast naming it. Borrows are ordered: read
    // the edge inputs + name into locals FIRST (immutable borrow of the session), then
    // set the toast on `rest`, then write `was_working` — so no borrow of `sessions[idx]`
    // overlaps the `rest`-level toast mutation.
    let now_working = state.rest.sessions[idx].is_working();
    // W5: the RAW working->ready edge (NO `!viewed` qualifier, unlike `edge_finished`
    // below). An extension wants every turn boundary — whether or not a client is
    // watching — so `agent.turn_end` fires on this raw edge. Computed separately here
    // so the existing toast / finished_unseen / was_working bookkeeping stays
    // byte-identical; used only by the fan-out at the end of this function.
    let raw_turn_end = state.rest.sessions[idx].was_working && !now_working;
    let edge_finished = state.rest.sessions[idx].was_working && !now_working && !viewed;
    if edge_finished {
        let name = state.rest.sessions[idx]
            .session
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("session {idx}"));
        // Per-session toast (C6): raise it on `sessions[idx]` itself. The edge fired for
        // a session VIEWED BY NOBODY, so a client foregrounding it later will project
        // ITS toast (`fg().toast`) and see the "ready" notice — instead of the toast
        // landing on whatever the stale `foreground` cursor happened to point at.
        state.rest.sessions[idx].set_toast_info(format!("session {name} ready"));
        // STICKY counterpart of the TTL toast (daemon critique #3): latch the
        // unseen marker so a DETACHED client still learns this background session
        // finished once it reattaches, long after the toast would have expired.
        state.rest.sessions[idx].finished_unseen = true;
        dirty = true;
    }
    // Clear the sticky marker the moment this session is VIEWED BY SOME client (C2).
    // Covers the local TUI (the single foreground is the only viewed session) and any
    // daemon client that foregrounds this session: a session appearing in some client's
    // view counts as "seen". Keeps the marker honest with no extra plumbing.
    if viewed && state.rest.sessions[idx].finished_unseen {
        state.rest.sessions[idx].finished_unseen = false;
        dirty = true;
    }
    state.rest.sessions[idx].was_working = now_working;

    // W5: fan out `agent.turn_end` on the RAW working->ready edge. Placed after the
    // toast / finished_unseen / was_working bookkeeping above (all left byte-identical)
    // so the immutable `&AppState` the emit needs is a clean reborrow. Purely additive:
    // it never sets `dirty` and, with no subscribed extensions, is a structural no-op.
    if raw_turn_end {
        let session_uuid = state.rest.sessions[idx].id.clone();
        let params = serde_json::json!({ "session": session_uuid });
        crate::app::ext::events::emit(state, "agent.turn_end", &params);
    }

    dirty
}

#[cfg(test)]
#[path = "deferred_ext_prompt_tests.rs"]
mod ext_prompt_tests;

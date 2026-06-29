use std::sync::Arc;

use crate::app::state::AppState;
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
    // Deferred tools (read/write/edit/delete/bash/grep/glob/remember/
    // web_fetch/web_search) run on a plain std::thread (spawned in
    // `dispatch_deferred`) and send their `(call_id, result)` back over
    // `tool_task_rx`. Fold each into the PARKED round's `tool_results` and drop
    // its id from `pending_tool_tasks`, exactly mirroring the sub-agent deferral
    // — so the resume gate below sees the settled set. Done within this same
    // block (before the gate) so both lanes' results are in place when emptiness
    // is tested. A round runs its deferred tools ONE AT A TIME, so at most one
    // id settles here per resume.
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
            if let Some(pos) = state.rest.sessions[idx].pending_tool_tasks.iter().position(|c| c == &id) {
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
                let _ = crate::model::msglog::append(&sess.path, crate::dto::chat::Role::User, &content, None);
                sess.conversation.push_user(content);
                let _ = sess.save();
            }
            // Park ends: a fresh `!`/Submit is allowed again.
            state.rest.sessions[idx].awaiting_shell = false;
            // Foreground-only UI: surface the new entry + the command's exit
            // status (the captured output's last line is `exit code: N`).
            if idx == state.rest.foreground {
                state.rest.reset_scroll();
                let exit_line = output.lines().last().unwrap_or("done");
                state.rest.status = format!("$ {cmd} — {exit_line}");
            }
            dirty = true;
        }
    }

    // --- drain background-bash COMPLETION signals (toast only) ---
    // A `bash` call with `run_in_background: true` runs DETACHED on its own worker
    // thread (spawned in `process_tools` via `bgbash::spawn_bash_job`); when the
    // child exits the worker fires the finished job id over `bash_done_tx`. This is
    // a fire-and-forget completion: the job is NEVER parked on (the tool already
    // answered with its id immediately), so finishing only pops an info toast so
    // the user sees it landed. The job STAYS in `bash_jobs` so a later
    // `bash_output` can still read its final status + output. Non-blocking
    // try_recv loop. (Chat-line rendering of the completion is a later stage.)
    {
        // Drain the finished ids into a local FIRST so the `rx` borrow of this
        // session's runtime is released before we look the jobs back up below.
        let mut finished: Vec<usize> = Vec::new();
        if let Some(rx) = state.rest.sessions[idx].bash_done_rx.as_mut() {
            while let Ok(id) = rx.try_recv() {
                finished.push(id);
            }
        }
        for id in finished {
            // Snapshot the final status into a short label for the toast. An id
            // with no matching job (cleared session) just falls through silently.
            let label = state.rest.sessions[idx]
                .bash_jobs
                .iter()
                .find(|j| j.id == id)
                .map(|j| match j.snapshot_status() {
                    crate::app::bgbash::BashJobStatus::Running => "running".to_string(),
                    crate::app::bgbash::BashJobStatus::Done(code) => format!("exit {code}"),
                    crate::app::bgbash::BashJobStatus::Killed => "killed".to_string(),
                    crate::app::bgbash::BashJobStatus::Error(msg) => format!("error: {msg}"),
                });
            if let Some(label) = label {
                state.rest.set_toast_info(format!("bash-{id} finished: {label}"));
                state.rest.sessions[idx].pending_bash_nudges.push((id, label));
                dirty = true;
            }
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
        let history = {
            let sess = state.rest.sessions[idx].session.as_mut().unwrap();
            let _ = crate::model::msglog::append(&sess.path, crate::dto::chat::Role::User, &body, None);
            sess.conversation.push_user(body);
            let _ = sess.save();
            sess.conversation.history()
        };

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
        }
        // Foreground-only UI cues (don't yank a background session's transcript).
        if idx == state.rest.foreground {
            state.rest.reset_scroll();
            state.rest.status = "thinking".into();
        }
        super::super::super::stream::start_stream_task(history, state, idx, client, handle);
        dirty = true;
    }

    dirty
}

/// Detect the working→ready edge for `idx` and emit a background-finish toast.
/// Also clears the sticky `finished_unseen` marker when the session comes into
/// the foreground. Updates `was_working` for the next tick.
/// Returns true if any state changed (toast or marker).
pub(super) fn nudge_background_finish(state: &mut AppState, idx: usize) -> bool {
    let mut dirty = false;

    // --- background-finish nudge ---
    // Detect this session's working→ready edge for THIS tick. When a session that
    // was working last tick is now idle AND it is NOT the foreground (so the user
    // can't already see it finish), pop an info toast naming it. Borrows are
    // ordered: read the edge inputs + name into locals FIRST (immutable borrow of
    // the session), then set the toast on `rest`, then write `was_working` — so no
    // borrow of `sessions[idx]` overlaps the `rest`-level toast mutation.
    let now_working = state.rest.sessions[idx].is_working();
    let edge_finished = state.rest.sessions[idx].was_working
        && !now_working
        && idx != state.rest.foreground;
    if edge_finished {
        let name = state.rest.sessions[idx]
            .session
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("session {idx}"));
        state.rest.set_toast_info(format!("session {name} ready"));
        // STICKY counterpart of the TTL toast (daemon critique #3): latch the
        // unseen marker so a DETACHED client still learns this background session
        // finished once it reattaches, long after the toast would have expired.
        state.rest.sessions[idx].finished_unseen = true;
        dirty = true;
    }
    // Clear the sticky marker the moment this session is the one being looked at
    // (it is the foreground). Covers the local TUI (always has a foreground) and a
    // client that foregrounds this session: switching INTO it counts as "seen". A
    // later switch-handler stage may also clear on an explicit view; this keeps the
    // marker honest for the common foreground==idx case with no extra plumbing.
    if idx == state.rest.foreground && state.rest.sessions[idx].finished_unseen {
        state.rest.sessions[idx].finished_unseen = false;
        dirty = true;
    }
    state.rest.sessions[idx].was_working = now_working;

    dirty
}

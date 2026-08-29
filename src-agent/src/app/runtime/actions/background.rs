//! Action handlers that flip running blocking sub-agents / foreground bash jobs
//! to detached (backgrounded) mode: BackgroundSubagent, BackgroundAllSubagents.
//! Split out of [`super::chat`] for file size.

use anyhow::Result;

use crate::app::state::AppState;

/// Handle `Action::BackgroundSubagent`: flip a running blocking sub-agent to
/// detached mode without killing it.
///
/// - Sets `sa.detached = true` and clears `sa.tool_call_id`.
/// - Removes the call id from `pending_subagent_calls`.
/// - Pushes an immediate tool result for that call id so the parked main turn
///   can resume (the park gate in `deferred.rs` fires next tick on an empty
///   `pending_subagent_calls`).
/// - Does NOT manually resume the turn — the existing gate in
///   `event_loop/sessions/deferred.rs` (`pending_subagent_calls.is_empty()`)
///   detects the cleared list next tick and calls `resume_after_subagents`.
/// - The agent keeps running; on completion `drain_subagents` fires the standard
///   detached completion nudge (keyed off `sa.detached`).
pub(super) fn handle_background_subagent(id: usize, state: &mut AppState) -> Result<()> {
    let fg = state.rest.foreground;

    // Locate the sub-agent by stable session id.
    let sa_idx = state.rest.sessions[fg]
        .subagents
        .iter()
        .position(|sa| sa.id == id);
    let Some(sa_idx) = sa_idx else {
        // Stale id (agent was pruned / id wrapped) — no-op.
        return Ok(());
    };

    // Guard: must be Running and not already detached, and must have a tool_call_id.
    {
        let sa = &state.rest.sessions[fg].subagents[sa_idx];
        if !matches!(sa.status, crate::app::subagent::SubAgentStatus::Running)
            || sa.detached
            || sa.tool_call_id.is_none()
        {
            return Ok(());
        }
    }

    // Take the call id before we mutate the sub-agent.
    let Some(call_id) = state.rest.sessions[fg].subagents[sa_idx]
        .tool_call_id
        .take()
    else {
        crate::model::store::append_global_error_log(
            "background",
            "BUG: tool_call_id was None after is_none() check",
        );
        return Ok(());
    };

    // Flip to detached so the completion path fires a nudge instead of a tool result.
    state.rest.sessions[fg].subagents[sa_idx].detached = true;

    // Remove the call id from the park set and push an immediate tool result so the
    // parked round's tool_call has a matching result and the round can continue.
    state.rest.sessions[fg]
        .pending_subagent_calls
        .retain(|c| c != &call_id);

    let agent_name = state.rest.sessions[fg].subagents[sa_idx].agent_name.clone();
    let result_text = format!(
        "backgrounded sub-agent #{id} ({agent_name}) — now running in the background. Its full \
         report is delivered to you automatically when it finishes — no need to poll. Don't \
         re-announce it to the user; just continue the conversation naturally, and you'll be \
         woken with the result when it lands."
    );
    state.rest.sessions[fg]
        .tool_results
        .push((call_id, result_text));

    // Status toast for user feedback.
    state.rest.sessions[fg].status = format!("↳ backgrounded sub-agent #{id}");

    Ok(())
}

/// Promote one still-blocking foreground bash job: take `tool_call_id`, clear
/// deadline, remove from `pending_tool_tasks`, push synthetic tool result.
/// Returns true if a job was promoted.
fn promote_bash_job_at(state: &mut AppState, fg: usize, job_idx: usize) -> bool {
    let job = &mut state.rest.sessions[fg].bash_jobs[job_idx];
    if !matches!(
        job.snapshot_status(),
        crate::app::bgbash::BashJobStatus::Running
    ) {
        return false;
    }
    let Some(call_id) = job.tool_call_id.take() else {
        return false;
    };
    let id = job.id;
    job.clear_deadline();

    state.rest.sessions[fg]
        .pending_tool_tasks
        .retain(|c| c != &call_id);

    let result_text = format!(
        "backgrounded bash-{id} — now running in the background. Poll with \
         bash_output{{\"job_id\":\"bash-{id}\"}}, stop with \
         bash_kill{{\"job_id\":\"bash-{id}\"}}. Its completion is delivered to you \
         automatically when it finishes — no need to re-announce it; just continue."
    );
    state.rest.sessions[fg]
        .tool_results
        .push((call_id, result_text));
    true
}

/// Promote every still-blocking FG bash job in the foreground session.
/// Returns how many jobs were promoted.
fn promote_all_blocking_bash(state: &mut AppState) -> usize {
    let fg = state.rest.foreground;
    let eligible: Vec<usize> = state.rest.sessions[fg]
        .bash_jobs
        .iter()
        .enumerate()
        .filter(|(_, j)| j.is_blocking())
        .map(|(i, _)| i)
        .collect();
    let mut n = 0;
    for job_idx in eligible {
        if promote_bash_job_at(state, fg, job_idx) {
            n += 1;
        }
    }
    n
}

/// Handle `Action::BackgroundAllSubagents`: detach EVERY running blocking
/// sub-agent AND promote every still-blocking foreground bash job in the
/// foreground session (composer Ctrl+B).
///
/// Emptying `pending_subagent_calls` / `pending_tool_tasks` lets the park gate
/// in `event_loop/sessions/deferred.rs` fire `resume_after_subagents`
/// automatically on the next tick — no manual resume needed here.
pub(super) fn handle_background_all_subagents(state: &mut AppState) -> Result<()> {
    let fg = state.rest.foreground;

    // Collect eligible sub-agent indices first (immutable borrow).
    let eligible_indices: Vec<usize> = state.rest.sessions[fg]
        .subagents
        .iter()
        .enumerate()
        .filter(|(_, sa)| {
            matches!(sa.status, crate::app::subagent::SubAgentStatus::Running)
                && !sa.detached
                && sa.tool_call_id.is_some()
        })
        .map(|(i, _)| i)
        .collect();

    let mut n_sa = 0usize;
    for sa_idx in eligible_indices {
        let Some(call_id) = state.rest.sessions[fg].subagents[sa_idx]
            .tool_call_id
            .take()
        else {
            crate::model::store::append_global_error_log(
                "background",
                "BUG: tool_call_id was None after filter Some",
            );
            continue;
        };

        let id = state.rest.sessions[fg].subagents[sa_idx].id;
        let agent_name = state.rest.sessions[fg].subagents[sa_idx].agent_name.clone();

        state.rest.sessions[fg].subagents[sa_idx].detached = true;

        state.rest.sessions[fg]
            .pending_subagent_calls
            .retain(|c| c != &call_id);

        let result_text = format!(
            "backgrounded sub-agent #{id} ({agent_name}) — now running in the background. Its full \
             report is delivered to you automatically when it finishes — no need to poll. Don't \
             re-announce it to the user; just continue the conversation naturally, and you'll be \
             woken with the result when it lands."
        );
        state.rest.sessions[fg]
            .tool_results
            .push((call_id, result_text));
        n_sa += 1;
    }

    let n_bash = promote_all_blocking_bash(state);
    let n = n_sa + n_bash;
    if n == 0 {
        // Nothing to background — no-op; don't touch status.
        return Ok(());
    }

    // Single summary status after processing all.
    state.rest.sessions[fg].status = match (n_sa, n_bash) {
        (s, 0) => format!("↳ backgrounded {s} sub-agent(s)"),
        (0, b) => format!("↳ backgrounded {b} bash job(s)"),
        (s, b) => format!("↳ backgrounded {s} sub-agent(s), {b} bash job(s)"),
    };

    Ok(())
}

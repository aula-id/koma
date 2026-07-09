//! Action handlers that flip running blocking sub-agents to detached
//! (backgrounded) mode: BackgroundSubagent, BackgroundAllSubagents. Split out
//! of [`super::chat`] for file size.

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
    let call_id = state.rest.sessions[fg].subagents[sa_idx]
        .tool_call_id
        .take()
        .unwrap(); // Safe: checked Some above.

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

/// Handle `Action::BackgroundAllSubagents`: flip EVERY running blocking sub-agent
/// in the foreground session to detached mode at once.
///
/// Mirrors `handle_background_subagent` for each eligible agent, then sets a
/// single summary status toast.  Eligibility: `Running` status, not already
/// detached, has a `tool_call_id` (i.e. is blocking a main-turn park gate).
///
/// Emptying all entries from `pending_subagent_calls` lets the existing park gate
/// in `event_loop/sessions/deferred.rs` (`pending_subagent_calls.is_empty()`)
/// fire `resume_after_subagents` automatically on the next tick — no manual
/// resume needed here.
pub(super) fn handle_background_all_subagents(state: &mut AppState) -> Result<()> {
    let fg = state.rest.foreground;

    // Collect eligible indices first (immutable borrow) to avoid borrow conflicts
    // when we later mutate per-agent fields and push tool results.
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

    let n = eligible_indices.len();
    if n == 0 {
        // Nothing to background — no-op; don't touch status.
        return Ok(());
    }

    for sa_idx in eligible_indices {
        // Take the call id before flipping detached.
        let call_id = state.rest.sessions[fg].subagents[sa_idx]
            .tool_call_id
            .take()
            .unwrap(); // Safe: filtered Some above.

        let id = state.rest.sessions[fg].subagents[sa_idx].id;
        let agent_name = state.rest.sessions[fg].subagents[sa_idx].agent_name.clone();

        // Flip to detached so completion fires a nudge instead of a tool result.
        state.rest.sessions[fg].subagents[sa_idx].detached = true;

        // Remove from the park set + push an immediate tool result.
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
    }

    // Single summary status after processing all agents.
    state.rest.sessions[fg].status = format!("↳ backgrounded {n} sub-agent(s)");

    Ok(())
}

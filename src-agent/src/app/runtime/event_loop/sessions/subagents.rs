use std::sync::Arc;

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

use super::super::super::stream::try_start_pending;

/// Drain each sub-agent's event channel for session `idx`.
/// Collects events into local vecs (collect-then-apply pattern to avoid borrow
/// conflicts), delivers terminal results, folds usage, starts queued delegations.
/// Returns true if anything changed.
pub(super) fn drain_subagents(
    state: &mut AppState,
    idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    use crate::app::subagent::{AgentEvent, SubAgentStatus};

    let mut dirty = false;

    // Char-safe truncation helper (avoids panicking on multibyte boundaries).
    fn trunc(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            s.to_string()
        } else {
            let cut: String = s.chars().take(max).collect();
            format!("{cut}…")
        }
    }

    // Deferred `task`-tool results to deliver into the PARKED tool round
    // (call_id, result_text), accumulated across every sub-agent this tick and
    // applied after the loop. A sub-agent that reaches a terminal state and
    // still has its call id in `pending_subagent_calls` fills its result here
    // (the FULL report on Done, an error/killed note otherwise) so the parked
    // round can resume with no dangling tool_call ids.
    let mut deferred_results: Vec<(String, String)> = Vec::new();

    for i in 0..state.rest.sessions[idx].subagents.len() {
        // --- collect phase: drain rx into a local vec ---
        let mut disconnected = false;
        let events: Vec<AgentEvent> = {
            let sa = &mut state.rest.sessions[idx].subagents[i];
            let mut evs = Vec::new();
            loop {
                match sa.rx.try_recv() {
                    Ok(ev) => evs.push(ev),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            evs
        };

        // Channel closed (task ended): mark Killed if still Running.
        if disconnected {
            let sa = &mut state.rest.sessions[idx].subagents[i];
            if matches!(sa.status, SubAgentStatus::Running) {
                sa.status = SubAgentStatus::Killed;
                dirty = true;
            }
        }

        // --- apply phase: fold events onto the sub-agent ---
        // The task-tool path delivers its result via `deferred_results`
        // (computed from the settled status below), not here.
        if !events.is_empty() {
            dirty = true;
            let sa = &mut state.rest.sessions[idx].subagents[i];
            for ev in events {
                match ev {
                    AgentEvent::Token(t) => {
                        // Merge consecutive token chunks into the last transcript
                        // line when it is still a "token" line (not a marker line)
                        // and short. Push a new line otherwise.
                        let is_marker = sa.transcript.last().is_some_and(|l| {
                            l.starts_with("— ")
                                || l.starts_with("→ ")
                                || l.starts_with("✓ ")
                                || l.starts_with("done:")
                                || l.starts_with("error:")
                        });
                        if !is_marker
                            && sa.transcript.last().is_some_and(|l| l.len() < 200)
                        {
                            if let Some(last) = sa.transcript.last_mut() {
                                last.push_str(&t);
                            }
                        } else {
                            sa.transcript.push(t);
                        }
                        // Cap growth at ~200 lines.
                        if sa.transcript.len() > 200 {
                            let drop = sa.transcript.len() - 200;
                            sa.transcript.drain(..drop);
                        }
                    }
                    AgentEvent::Step(n) => {
                        sa.transcript.push(format!("— step {n} —"));
                    }
                    AgentEvent::Snapshot(m) => {
                        // Replace the structured history wholesale; drives the
                        // full-screen sub-agent viewer.
                        sa.messages = m;
                    }
                    AgentEvent::ToolStarted { name, args } => {
                        sa.transcript.push(format!("→ {name} {}", trunc(&args, 120)));
                    }
                    AgentEvent::ToolDone { name, result } => {
                        let first = result.lines().next().unwrap_or("").trim();
                        sa.transcript.push(format!("✓ {name}: {}", trunc(first, 120)));
                    }
                    AgentEvent::Done(s) => {
                        sa.transcript.push(format!("done: {}", trunc(&s, 200)));
                        sa.status = SubAgentStatus::Done(s);
                    }
                    AgentEvent::Error(e) => {
                        sa.transcript.push(format!("error: {e}"));
                        sa.status = SubAgentStatus::Error(e);
                    }
                    AgentEvent::UsageReport { model_id, tokens_in, tokens_out, cost } => {
                        // Overwrite with the final report's values; the loop
                        // emits exactly one UsageReport (just before Done).
                        sa.model_id = model_id;
                        sa.usage_tokens_in = tokens_in;
                        sa.usage_tokens_out = tokens_out;
                        sa.usage_cost = cost;
                    }
                }
            }
        }

        // --- terminal delivery / fold ---
        // Inspect the SETTLED status + origin once events are folded, capturing
        // owned values up front so the immutable borrow of `subagents[i]` is
        // released before any `sessions[idx].session` mutation below. Runs every
        // tick (even when no events arrived, so a disconnect-only Killed is still
        // delivered). The "still in pending_subagent_calls" guard makes a
        // task-tool delivery happen EXACTLY ONCE (the id is removed after the
        // loop, so later ticks skip it).
        //
        // `chat_fold` carries the /task chat-fold note; `defer` carries the
        // (call_id, result) for the task-tool deferred delivery. `sub_usage`
        // carries (model_id, tokens_in, tokens_out, cost) to merge+record when
        // the sub-agent reaches any terminal state. At most one of chat_fold /
        // defer is Some (the two origins are mutually exclusive on tool_call_id).
        // sub_usage is Some whenever the status is terminal and usage > 0.
        let (chat_fold, defer, sub_usage) = {
            let sa = &state.rest.sessions[idx].subagents[i];
            // Capture usage once; only carry it if there is something to record.
            let usage_tuple = if sa.usage_tokens_out > 0 || sa.usage_cost > 0.0 {
                Some((
                    sa.model_id.clone(),
                    sa.usage_tokens_in,
                    sa.usage_tokens_out,
                    sa.usage_cost,
                ))
            } else {
                None
            };
            match (&sa.tool_call_id, &sa.status) {
                // task-tool path: deliver the deferred result back to the model.
                (Some(call_id), status)
                    if state.rest.sessions[idx].pending_subagent_calls.contains(call_id) =>
                {
                    let result = match status {
                        // Deliver the FULL, untruncated report.
                        SubAgentStatus::Done(s) => Some(s.clone()),
                        SubAgentStatus::Error(e) => Some(format!("sub-agent error: {e}")),
                        // Killed (user Ctrl+X / task died) — fill so the round
                        // can't hang waiting on a result that will never come.
                        SubAgentStatus::Killed => Some("[sub-agent killed]".to_string()),
                        // Still running: nothing to deliver this tick.
                        SubAgentStatus::Running => None,
                    };
                    // Only carry usage on a terminal transition (result is Some).
                    let carry_usage = if result.is_some() { usage_tuple } else { None };
                    (None, result.map(|r| (call_id.clone(), r)), carry_usage)
                }
                // /task command path (tool_call_id == None): on Done, build the
                // FULL, untruncated report note (injected as an assistant turn
                // below). Done is terminal and the agent is pruned this tick, so
                // it fires once.
                (None, SubAgentStatus::Done(result)) => (
                    Some(format!(
                        "[sub-agent #{} {}] finished: {result}",
                        sa.id, sa.agent_name
                    )),
                    None,
                    usage_tuple,
                ),
                // /task command path: Killed or Error — no chat-fold note (the
                // turn is dead), but still carry accumulated usage so cost is
                // not silently lost.
                (None, SubAgentStatus::Killed | SubAgentStatus::Error(_)) => {
                    (None, None, usage_tuple)
                }
                _ => (None, None, None),
            }
        };
        if let Some(note) = chat_fold {
            // /task command path: append the full report as a display-only
            // assistant turn so the session retains a complete record.
            if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
                // Log to sqlite (no usage/cost for a sub-agent fold).
                let _ = crate::model::msglog::append(
                    &sess.path,
                    crate::dto::chat::Role::Assistant,
                    &note,
                    None,
                );
                sess.conversation.push_assistant(note, None);
                let _ = sess.save();
            }
        }
        // Merge sub-agent spend into the OWNING session's totals + record a
        // ledger row. Done for BOTH paths (chat_fold = /task, defer = task-tool)
        // at the single point where a terminal status is first observed.
        // Non-fatal: skipped when no usage was ever reported (provider omits it).
        // The spend credits THIS session (`sessions[idx]`), never a global, so
        // each tab's counters reflect only its own (and its sub-agents') usage.
        if let Some((sub_model_id, sub_ti, sub_to, sub_cost)) = sub_usage {
            // Merge into THIS session's counters: cost and tokens_out are
            // cumulative (summed); tokens_in is the main-context gauge and must
            // NOT be touched (adding sub-agent prompt size would corrupt the
            // context-window display).
            state.rest.sessions[idx].cost += sub_cost;
            state.rest.sessions[idx].tokens_out += sub_to;
            // Record one ledger row per sub-agent completion (best-effort).
            let (sess_uuid, pwd_hash) = state
                .rest
                .sessions[idx]
                .session
                .as_ref()
                .map(|s| (s.id.clone(), s.pwd_hash.clone()))
                .unwrap_or_default();
            let sa_name = state.rest.sessions[idx].subagents[i].agent_name.clone();
            crate::model::usage::record_usage(
                &sub_model_id,
                &format!("sub:{sa_name}"),
                &sess_uuid,
                &pwd_hash,
                sub_ti,
                0, // sub-agents never receive cached-tokens data
                sub_to,
                sub_cost,
            );
        }
        if let Some(pair) = defer {
            deferred_results.push(pair);
        }
    }

    // Deliver every terminal task-tool result into the parked round's
    // `tool_results` and drop its id from `pending_subagent_calls`. Done AFTER
    // the loop so the per-agent borrow above stays immutable.
    for (call_id, result) in deferred_results {
        state.rest.sessions[idx].pending_subagent_calls.retain(|c| c != &call_id);
        state.rest.sessions[idx].tool_results.push((call_id, result));
        dirty = true;
    }

    // --- keep terminated sub-agents as session history ---
    // Terminated agents (Done, Error, Killed) are NOT pruned: the $ panel is a
    // session history, so every sub-agent that ran stays in the list with its
    // final status + structured `messages` for later viewing.
    // `running_subagents()` still counts only `Running`, so the cap is
    // unaffected. The list only ever grows here, so `subagent_sel` (always <
    // len once set) can never fall out of range — no clamp needed.

    // --- start queued delegations into any freed slots ---
    // A terminal handle above may have freed a slot. Start as many pending
    // sub-agents (FRONT-first) as now fit. Done BEFORE the resume gate below: a
    // queued task-tool delegation keeps its call id in `pending_subagent_calls`
    // across the queued→running transition, and an unstartable entry delivers
    // its error result + drops its id HERE — so the
    // `pending_subagent_calls.is_empty()` test sees the settled set and can't
    // resume a round that still has a queued delegation outstanding.
    if !state.rest.sessions[idx].pending_subagents.is_empty() {
        try_start_pending(state, idx, client, handle);
        dirty = true;
    }

    dirty
}

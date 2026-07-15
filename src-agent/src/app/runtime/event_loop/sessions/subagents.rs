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
    // Set whenever a sub-agent's lifecycle STATUS transitions to terminal this tick
    // (disconnect→Killed / Done / Error), so the persisted records (#25) are
    // re-written exactly once — reflecting the final status, not a stale "running".
    // Not tied to `dirty` (which also flips on pure token/transcript growth that
    // would trigger a needless DB write every streaming tick).
    let mut status_changed = false;

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

    // W5: sub-agent terminal transitions observed THIS tick, to fan out to
    // subscribed extensions AFTER the loop (collect-then-apply, same borrow
    // discipline as `deferred_results` — the emit takes `&AppState`). Each tuple is
    // (session_uuid, local_subagent_id, agent_name, status, error). `error` is
    // `Some(text)` only for a `SubAgentStatus::Error` settlement — the text behind
    // the sub-agent's `agents.done`/`subagent.done` `"error"` field. Only a genuine
    // Running->terminal edge is pushed; see the `was_running` gate below.
    let mut ext_terminal_emissions: Vec<(String, usize, String, &'static str, Option<String>)> =
        Vec::new();

    for i in 0..state.rest.sessions[idx].subagents.len() {
        // W5: snapshot whether this sub-agent was Running at the START of this tick.
        // A Running->terminal transition observed by the end of the iteration is
        // fanned out exactly once; a restored/already-terminal record was NOT Running
        // here, so it never emits (and a kill applied via the broker already emitted).
        let was_running = matches!(
            state.rest.sessions[idx].subagents[i].status,
            SubAgentStatus::Running
        );

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
                status_changed = true;
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
                        // Accumulate the raw streamed report text for the CURRENT
                        // turn so the full-screen viewer can render it live (the
                        // transcript below is a lossy, capped display log; this is
                        // the verbatim in-progress report). Cleared on the next
                        // Snapshot, which commits this turn into `messages`.
                        sa.live_text.push_str(&t);
                        // Merge consecutive token chunks into the last transcript
                        // line when it is still a "token" line (not a marker line)
                        // and short. Push a new line otherwise.
                        let is_marker = sa.transcript.last().is_some_and(|l| {
                            l.starts_with("— ")
                                || l.starts_with("→ ")
                                || l.starts_with("✓ ")
                                || l.starts_with("⋯ ")
                                || l.starts_with("» ")
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
                        // The stall/continue path re-emits the SAME step number after a
                        // reasoning-only round; don't stack a duplicate divider. Skip if
                        // the most recent step divider in the tail is already this one.
                        let divider = format!("— step {n} —");
                        let dup = sa
                            .transcript
                            .iter()
                            .rev()
                            .find(|l| l.starts_with("— step "))
                            .is_some_and(|l| l == &divider);
                        if !dup {
                            sa.transcript.push(divider);
                        }
                    }
                    AgentEvent::Injected(msg) => {
                        // A follow-up user message was injected into this running
                        // sub-agent (broker `agents.send` / `task_send`) and folded
                        // into its history at this turn boundary. Log a distinct
                        // transcript line so a human watching the $ panel sees the
                        // steer; the Snapshot emitted right after carries the same
                        // message into the structured viewer history.
                        sa.transcript.push(format!("» steer: {}", trunc(&msg, 200)));
                        if sa.transcript.len() > 200 {
                            let drop = sa.transcript.len() - 200;
                            sa.transcript.drain(..drop);
                        }
                    }
                    AgentEvent::Snapshot(m) => {
                        // Surface reasoning for an otherwise-SILENT step: if the step
                        // that just committed produced no Token/tool lines (its divider
                        // is still the last transcript line), the model spent the step
                        // thinking. Reasoning is never streamed as Token, so this is the
                        // only place it can reach the flat transcript — without it a
                        // thinking-only step is a blank `— step N —`. Pull the newest
                        // assistant message's reasoning and log a one-line summary.
                        let step_silent = sa
                            .transcript
                            .last()
                            .is_some_and(|l| l.starts_with("— step "));
                        if step_silent {
                            if let Some(first) = m
                                .iter()
                                .rev()
                                .find(|msg| {
                                    matches!(msg.role, crate::dto::chat::Role::Assistant)
                                        && msg
                                            .reasoning
                                            .as_deref()
                                            .is_some_and(|r| !r.trim().is_empty())
                                })
                                .and_then(|msg| msg.reasoning.as_deref())
                                .and_then(|r| {
                                    r.trim().lines().map(str::trim).find(|l| !l.is_empty())
                                })
                            {
                                sa.transcript.push(format!("⋯ {}", trunc(first, 140)));
                            }
                        }
                        // Replace the structured history wholesale; drives the
                        // full-screen sub-agent viewer.
                        sa.messages = m;
                        // The turn just committed into `messages`, so the live
                        // in-progress buffer is now duplicated there — clear it so
                        // the viewer doesn't render the report twice.
                        sa.live_text = String::new();
                    }
                    AgentEvent::ToolStarted { name, args } => {
                        // Readable quote-less signature (`read(koma.ts)`,
                        // `grep(enum HostCtl)`) instead of raw JSON args, reusing the
                        // same formatter the main chat transcript uses.
                        let sig = crate::view::chat::transcript::format_tool_signature(&name, &args);
                        sa.transcript.push(format!("→ {sig}"));
                    }
                    AgentEvent::ToolDone { name, result } => {
                        let first = result.lines().next().unwrap_or("").trim();
                        sa.transcript.push(format!("✓ {name}: {}", trunc(first, 120)));
                    }
                    AgentEvent::Done(s) => {
                        sa.transcript.push(format!("done: {}", trunc(&s, 200)));
                        sa.status = SubAgentStatus::Done(s);
                        status_changed = true;
                    }
                    AgentEvent::Error(e) => {
                        sa.transcript.push(format!("error: {e}"));
                        sa.status = SubAgentStatus::Error(e);
                        status_changed = true;
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
        // `chat_fold` carries the /task COMPACT completion note (a
        // BASH_NUDGE_MARK-prefixed User body; `None` for an ext-owned agent, which
        // stays silent — Tier B); `defer` carries the (call_id, result) for the
        // task-tool deferred delivery; `nudge` carries the (id, agent, status_label)
        // for a DETACHED sub-agent's one-shot completion nudge (buffered, injected
        // when idle — mirrors bg-bash).
        // `sub_usage` carries (model_id, tokens_in, tokens_out, cost) to merge+record
        // when the sub-agent reaches any terminal state. At most one of chat_fold /
        // defer / nudge is Some (blocking-task-tool, /task, and detached are mutually
        // exclusive). sub_usage is Some whenever the status is terminal and usage > 0.
        // `mark_nudged` is true whenever this tick consumed a one-shot arm gated on
        // `!sa.nudged` (the detached nudge arm, or either /task-path terminal arm
        // below) — applied to `sa.nudged` right after the match closes, so that same
        // arm's guard blocks it on every later tick (terminated records are kept as
        // history, never pruned, so without this they would re-fire forever).
        let (chat_fold, defer, nudge, sub_usage, mark_nudged) = {
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
            // DETACHED (task run_in_background) path FIRST: it carries
            // tool_call_id == None (so it would otherwise fall into the /task
            // arms below), but it must NOT chat-fold — instead it fires a ONE-shot
            // completion nudge. The `!sa.nudged` latch makes it fire exactly once
            // even though this block runs every tick until the agent is done.
            if sa.detached {
                let outcome = match &sa.status {
                    SubAgentStatus::Done(s) => Some(s.clone()),
                    SubAgentStatus::Error(e) => Some(format!("error: {e}")),
                    SubAgentStatus::Killed => Some("[killed]".to_string()),
                    SubAgentStatus::Running => None,
                };
                match outcome {
                    // Terminal + not yet nudged: carry the nudge + usage.
                    // The 3rd element of the tuple is the FULL outcome/report text,
                    // not a short status label — it is injected verbatim into the
                    // wake-nudge user turn so the model receives the complete result
                    // without needing to poll task_output.
                    Some(outcome) if !sa.nudged => {
                        (None, None, Some((sa.id, sa.agent_name.clone(), outcome)), usage_tuple, true)
                    }
                    // Terminal but already nudged: nothing to do (usage already
                    // recorded on the first terminal tick).
                    Some(_) => (None, None, None, None, false),
                    // Still running: nothing this tick.
                    None => (None, None, None, None, false),
                }
            } else {
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
                        // Not gated on `sa.nudged` — this path's one-shot delivery is
                        // already latched by removing `call_id` from
                        // `pending_subagent_calls` after the loop, so it never fires
                        // twice regardless of `nudged`.
                        (None, result.map(|r| (call_id.clone(), r)), None, carry_usage, false)
                    }
                    // /task command path (tool_call_id == None), HUMAN-spawned (not
                    // ext-owned): on Done, fold the report into a COMPACT completion
                    // note — a BASH_NUDGE_MARK-prefixed body whose line 1 is the terse
                    // `[sub-agent #N name] finished` summary the transcript renders (a
                    // dim green ✓), while the FULL report on the following lines is what
                    // the model reads (the mark is stripped on the wire, so no text is
                    // lost). Mirrors the detached-completion nudge (deferred.rs) and the
                    // bg-bash nudge: same mark, same compact render, full text preserved
                    // for the model. Restored/live records are NOT pruned once terminal
                    // (the list only ever grows, see below), so this arm would otherwise
                    // re-fire on every tick forever; gated on `!sa.nudged` and latched
                    // via `mark_nudged` (applied to `sa.nudged` right after the match) so
                    // it fires exactly once, mirroring the detached arm above. An
                    // EXT-OWNED agent (Tier B) SKIPS this arm via `!sa.ext_owned` and
                    // falls through to the silent usage-only arm below — its spawner
                    // already gets the result via `agents.done`, so it must never
                    // interrupt the human's chat.
                    (None, SubAgentStatus::Done(result)) if !sa.nudged && !sa.ext_owned => (
                        Some(format!(
                            "{}[sub-agent #{} {}] finished\n{result}",
                            crate::dto::chat::BASH_NUDGE_MARK,
                            sa.id,
                            sa.agent_name
                        )),
                        None,
                        None,
                        usage_tuple,
                        true,
                    ),
                    // Terminal with NO chat-fold note, but still carry accumulated
                    // usage so cost is never silently lost. Covers:
                    // - an EXT-OWNED Done (Tier B — completely silent in the human chat;
                    //   the spawner gets its `agents.done` callback instead), and
                    // - a /task Killed or Error (the turn is dead — nothing to fold).
                    // Latched the same as the Done arm above: without `!sa.nudged`, a
                    // terminated-but-kept record would re-add its usage every tick.
                    (
                        None,
                        SubAgentStatus::Done(_)
                        | SubAgentStatus::Killed
                        | SubAgentStatus::Error(_),
                    ) if !sa.nudged => (None, None, None, usage_tuple, true),
                    _ => (None, None, None, None, false),
                }
            }
        };
        // Buffer a detached agent's completion nudge and latch `nudged` so it
        // fires exactly once. Drained into ONE synthetic user turn when the
        // session next goes idle (see `deferred.rs`), mirroring bg-bash.
        if let Some(entry) = nudge {
            state.rest.sessions[idx].pending_subagent_nudges.push(entry);
            state.rest.sessions[idx].subagents[i].nudged = true;
            dirty = true;
        }
        // Latch the /task-path terminal arms (Done chat-fold, Killed/Error
        // usage-only) the same way the detached arm just latched above: once
        // consumed, `nudged` flips to true so their `!sa.nudged` guard skips
        // them on every later tick.
        if mark_nudged {
            state.rest.sessions[idx].subagents[i].nudged = true;
            dirty = true;
        }
        if let Some(note) = chat_fold {
            // /task command path: append the COMPACT completion note (a
            // BASH_NUDGE_MARK-prefixed USER turn) so the transcript shows a dim green
            // ✓ line while the model still reads the FULL report on its next turn (the
            // mark is stripped on the wire). A USER turn — mirroring the detached /
            // bg-bash nudges — so the transcript's mark render (User-gated,
            // `render_bash_nudge_block`) and every wire builder's mark strip apply.
            // Pushed as display-only history with NO stream kickoff, preserving the old
            // fold's "sits in history, the model sees it on its next turn" behavior. The
            // marked body is logged VERBATIM to sqlite (as deferred.rs does) so a reload
            // renders the same compact line.
            if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
                let _ = crate::model::msglog::append(
                    &sess.path,
                    crate::dto::chat::Role::User,
                    &note,
                    None,
                );
                sess.conversation.push_user(note);
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

        // W5: a Running->terminal transition THIS tick → queue an extension fan-out
        // for after the loop. The `was_running` gate makes this an EDGE (restored /
        // already-terminal records were not Running at tick start, so they never
        // fire); the settled status decides the label. Disconnect->Killed then a
        // late Done in the same tick settles to Done (Done wins), and that is what
        // is reported here.
        if was_running {
            let sa = &state.rest.sessions[idx].subagents[i];
            let terminal = match &sa.status {
                SubAgentStatus::Done(_) => Some(("done", None)),
                SubAgentStatus::Error(e) => Some(("error", Some(e.clone()))),
                SubAgentStatus::Killed => Some(("killed", None)),
                SubAgentStatus::Running => None,
            };
            if let Some((status, error)) = terminal {
                ext_terminal_emissions.push((
                    state.rest.sessions[idx].id.clone(),
                    sa.id,
                    sa.agent_name.clone(),
                    status,
                    error,
                ));
            }
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

    // Re-persist the sub-agent records once if any reached a terminal state this
    // tick, so a restored session shows the final status not a stale "running" (#25).
    if status_changed {
        crate::app::runtime::bg_persist::persist_subagents(&state.rest.sessions[idx]);
    }

    // W5: fan out every sub-agent terminal transition collected this tick. AFTER the
    // per-agent loop + persist so the immutable `&AppState` the emit needs is a clean
    // reborrow. Each fires the owned `agents.done` (notify:true spawner only) plus the
    // subscribed `subagent.done` broadcast; with no subscribed extensions it is a
    // structural no-op that never touches `dirty` or any session state.
    for (session_uuid, local_id, agent, status, error) in ext_terminal_emissions {
        crate::app::ext::events::emit_subagent_terminal(
            state,
            &session_uuid,
            local_id,
            &agent,
            status,
            error.as_deref(),
        );
    }

    dirty
}

//! The autonomous sub-agent loop.
//!
//! [`run_agent_loop`] is a NON-INTERACTIVE condensation of the interactive
//! engine in `app::runtime::stream` (`advance_turn` + `process_tools` +
//! `finish_tool_round`): it streams a model reply, runs the requested tools, and
//! feeds the results back — looping until the model produces a final answer or
//! the step budget is exhausted. Unlike the interactive engine it owns no
//! `AppState`, never prompts a human, and reports progress purely as
//! [`AgentEvent`]s.
//!
//! ## Differences from the interactive loop (deliberate)
//!
//! - **Allow-list enforcement.** `stream_complete` advertises ONLY this agent's
//!   `tools` allow-list to the model, so the model sees just the tools it is
//!   permitted to call. The loop ALSO rejects any call whose name is not in that
//!   allow-list with an `error: …` tool result — a backstop that keeps the
//!   conversation API-valid even if a model fabricates a name.
//! - **Fail CLOSED on classifier outage.** The interactive loop fails OPEN in
//!   Auto mode (an unavailable classifier auto-runs a risky call). A sub-agent
//!   has no human to fall back to, so an unavailable classifier BLOCKS the risky
//!   call instead — the safe default for an unattended actor.
//! - **No human approval.** There is no `y/n`: a risky call is gated solely by
//!   the tool-call classifier (TAC). When the harness is disabled (no Safeguard
//!   route), TAC is "unavailable" and the fail-closed rule blocks the call.

// Inert in Stage 1: the loop is fully implemented but not yet driven by the chat
// loop / `task` tool, so its items are unreferenced from the binary until a later
// stage wires it in.
#![allow(dead_code)]

use std::sync::Arc;

use tokio::sync::mpsc::{self, UnboundedSender};

use crate::app::resolve::Resolved;
use crate::dto::chat::ToolCall;
use crate::model::app_config::AppConfig;
use crate::model::conversation::Conversation;
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;
use crate::service::StreamEvent;
use crate::tool::ToolCtx;

use super::event::AgentEvent;

/// Send one event on the sub-agent channel, ignoring a closed receiver (the
/// orchestrator dropped it — e.g. the sub-agent was killed — so the event is
/// simply discarded, exactly like the interactive client's `emit`).
fn emit(tx: &UnboundedSender<AgentEvent>, event: AgentEvent) {
    let _ = tx.send(event);
}

/// True for tools that mutate the workspace (or run arbitrary shell commands).
/// Delegates to the single canonical definition in [`crate::tool::tool_is_risky`]
/// so the builtin-risky check is never duplicated. A risky call must clear the
/// tool-call classifier before it runs.
fn tool_is_risky(name: &str) -> bool {
    crate::tool::tool_is_risky(name)
}

/// Returns `true` when `text` looks like interstitial narration rather than a
/// finished report — e.g. "Let me read a few more files:" — so the engine can
/// nudge the model to keep going instead of accepting the half-thought as done.
///
/// Criteria (any one is enough):
/// - trimmed text is empty
/// - trimmed text ends with `:`  (classic "Let me read…:" cliffhanger)
/// - trimmed text is shorter than 40 chars  (too short to be a real report)
/// - trimmed text starts with a known procrastination phrase (case-insensitive)
fn is_stall(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.ends_with(':') || t.len() < 40 {
        return true;
    }
    let lower = t.to_lowercase();
    let stall_prefixes = [
        "let me", "i'll", "i will", "let's", "now i", "next,", "next i",
        "first,",
    ];
    stall_prefixes.iter().any(|p| lower.starts_with(p))
}

/// One drained stream result: the assistant text, any requested tool calls,
/// a fatal error if the stream failed, and the optional usage tuple from the
/// final `StreamEvent::Usage` chunk (prompt_tokens, completion_tokens, cost).
#[derive(Default)]
struct StreamOutcome {
    text: String,
    tool_calls: Vec<ToolCall>,
    error: Option<String>,
    /// Last-seen usage chunk: (prompt_tokens, completion_tokens, cost).
    /// `None` when the provider emitted no Usage event for this step.
    usage: Option<(u64, u64, f64)>,
}

/// Clean a sub-agent's raw final text into a deliverable report, mirroring the
/// interactive engine's `final_answer` (commit 3e2401c) for the autonomous loop.
///
/// Weak models often wrap their answer in XML-ish markup the native tool-call
/// path never stripped: a `<content>…</content>` wrapper, or inline
/// `<tool_call>…</tool_call>` / orphan tags. Delivered verbatim that markup
/// either leaks (`</content>`) or, once stripped, collapses to nothing — which is
/// how the report arrived EMPTY. So:
///   1. unwrap a single `<content>…</content>` wrapper to its inner text (if the
///      whole message is such a wrapper), then
///   2. run `strip_tool_call_tags` to drop residual tool-call markup, then
///   3. EMPTY-FALLBACK: if cleaning emptied the text, fall back to the RAW text
///      (better a tag-bearing report than an empty one); if the raw was itself
///      blank, deliver a clear `(no report)` placeholder rather than nothing.
///
/// Returns the cleaned report ready for `cap_report`.
fn finalize_report(raw: &str) -> String {
    let unwrapped = unwrap_content_tag(raw);
    let cleaned = crate::dto::chat::strip_tool_call_tags(unwrapped);
    if !cleaned.trim().is_empty() {
        return cleaned;
    }
    // Cleaning emptied it — prefer the raw text so a wrapped-but-real report is
    // still delivered; only when the raw is ALSO blank do we emit the placeholder.
    if !raw.trim().is_empty() {
        raw.to_string()
    } else {
        "(no report)".to_string()
    }
}

/// If `text` (trimmed) is wrapped ENTIRELY in a single `<content>…</content>`
/// block, return the inner slice; otherwise return `text` unchanged. Only the
/// outer wrapper is unwrapped (the inner text is then tag-stripped by the
/// caller). Matching is case-insensitive on the tag name and tolerates a closing
/// tag with trailing whitespace, but not extra prose outside the wrapper (so a
/// genuine report that merely mentions `<content>` is left intact).
fn unwrap_content_tag(text: &str) -> &str {
    const OPEN: &str = "<content>";
    const CLOSE: &str = "</content>";
    let trimmed = text.trim();
    // Case-insensitive prefix/suffix check without allocating for the body.
    let lower = trimmed.to_lowercase();
    if lower.starts_with(OPEN) && lower.ends_with(CLOSE) && trimmed.len() >= OPEN.len() + CLOSE.len()
    {
        let inner = &trimmed[OPEN.len()..trimmed.len() - CLOSE.len()];
        inner.trim()
    } else {
        trimmed
    }
}

/// Cap a sub-agent's final report so it can't overflow the main agent's context
/// window when delivered as a tool result. Truncates by CHARACTERS (not bytes,
/// so it never splits a UTF-8 boundary) and appends a marker.
fn cap_report(text: String) -> String {
    let max = crate::config::MAX_SUBAGENT_REPORT_CHARS;
    if text.chars().count() > max {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}\n\n[report truncated at {max} chars for delivery to the main agent — be more concise next time]")
    } else {
        text
    }
}

/// Run the autonomous sub-agent loop to completion.
///
/// Loops until the model produces a final answer (no tool calls) or, when
/// `max_steps` is `Some(n)`, until the step cap is reached. Each step:
/// 1. emits [`AgentEvent::Step`], then streams one reply via
///    [`OpenRouterClient::stream_complete`] on the resolved route, draining the
///    per-step channel (accumulating assistant text as [`AgentEvent::Token`]s,
///    collecting any tool calls, capturing a fatal error);
/// 2. pushes the assistant message into the isolated `convo`;
/// 3. if the model requested NO tools, emits [`AgentEvent::Done`] with the
///    answer and returns;
/// 4. otherwise runs each requested call — rejecting not-permitted names,
///    classifier-gating risky ones (fail CLOSED), running the rest via
///    [`crate::tool::execute_tool`] — pushing every result back into `convo` so
///    the next step sees them.
///
/// `max_steps = None` means unbounded (the natural termination above is the only
/// exit). `max_steps = Some(n)` adds an explicit cap; exhausting it emits
/// [`AgentEvent::Done`] with the last assistant text (or a "(stopped: step
/// budget reached)" note). A fatal stream error emits [`AgentEvent::Error`] and
/// returns. Never panics; a dropped receiver makes every emit a no-op.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop(
    client: Arc<OpenRouterClient>,
    resolved: Resolved,
    config: AppConfig,
    settings: Settings,
    tools: Vec<String>,
    ctx: ToolCtx,
    mut convo: Conversation,
    task_intent: String,
    max_steps: Option<usize>,
    tx: UnboundedSender<AgentEvent>,
) {
    // The most-recent assistant text, surfaced as the final answer if the loop
    // runs out of steps before the model gives a no-tool reply.
    let mut last_text = String::new();
    // Count how many consecutive stall nudges have been issued so far.
    let mut nudges: usize = 0;
    // Accumulated token/cost spend across all steps. tokens_in is reported as
    // the last-seen prompt size (not summed — it is a context-window gauge,
    // matching the main-model convention). tokens_out and cost are summed
    // across steps so the total reflects actual spend.
    let mut acc_tokens_out: u64 = 0;
    let mut acc_cost: f64 = 0.0;

    let mut step: usize = 0;
    loop {
        emit(&tx, AgentEvent::Step(step));

        // 1. Stream one model reply on a fresh per-step channel, then drain it.
        //    Advertise ONLY this agent's allow-list to the model (the execution
        //    gate below stays as a backstop).
        let outcome = stream_step(&client, &resolved, convo.history(), &tools, &tx).await;

        // Fold this step's usage into the running totals (best-effort: a step
        // with no Usage chunk simply contributes nothing). tokens_in is
        // reported as-is (current context size), tokens_out and cost are summed.
        // Emit a UsageReport after EVERY step so the SubAgent struct always
        // holds the latest accumulated spend — on kill/abort the drain can
        // still record what was captured so far (loses at most one step).
        if let Some((pt, ct, c)) = outcome.usage {
            acc_tokens_out += ct;
            acc_cost += c;
            emit(&tx, AgentEvent::UsageReport {
                model_id: resolved.model_id.clone(),
                tokens_in: pt,
                tokens_out: acc_tokens_out,
                cost: acc_cost,
            });
        }

        // A fatal stream error ends the run immediately.
        if let Some(err) = outcome.error {
            emit(&tx, AgentEvent::Error(err));
            return;
        }

        let assistant_text = outcome.text;
        let tool_calls = outcome.tool_calls;
        if !assistant_text.trim().is_empty() {
            last_text = assistant_text.clone();
        }

        // 2. Commit the assistant turn into the isolated history (with tool calls
        //    when present so the tool results can answer them). Reasoning is
        //    display-only and not tracked by the sub-agent, so `None`.
        if tool_calls.is_empty() {
            // Clean the raw text the SAME way the report will be delivered (unwrap
            // a <content>…</content> wrapper + strip tool-call markup) BEFORE the
            // stall gate, so the gate judges the deliverable content — a valid
            // report wrapped in tags isn't wrongly nudged, and a pure-markup
            // message (empty once stripped) is correctly treated as a stall.
            let report = finalize_report(&assistant_text);
            // 3. No tools → check whether this looks like an interstitial stall
            //    ("Let me read a few more files:" with no actual tool call) rather
            //    than a genuine final answer.  If so, nudge the model to continue
            //    instead of accepting the half-thought as a report. The gate runs
            //    on the cleaned report; commit the RAW text into history so the
            //    transcript still shows what the model literally said.
            if nudges < 2 && is_stall(&report) {
                convo.push_assistant(assistant_text, None);
                convo.push_user(
                    "Continue now: call the tools you need to finish the task, \
                     then write your COMPLETE final report. \
                     Do not stop with a 'let me...' line."
                        .to_string(),
                );
                nudges += 1;
                // Turn committed (assistant + nudge): snapshot the history.
                emit(&tx, AgentEvent::Snapshot(convo.messages().to_vec()));
                // Do not emit Done; loop for another step.
                continue;
            }
            // Genuine final answer (or nudge budget exhausted).
            convo.push_assistant(assistant_text, None);
            // Final turn committed: snapshot the full history before finishing.
            emit(&tx, AgentEvent::Snapshot(convo.messages().to_vec()));
            // Deliver the CLEANED report (tags stripped, with empty-fallback) so a
            // weak model's wrapped output never reaches the orchestrator as empty
            // or with a leaked </content>.
            emit(&tx, AgentEvent::Done(cap_report(report)));
            return;
        }
        convo.push_assistant_with_tools(assistant_text, tool_calls.clone(), None);

        // 4. Run each requested call, appending a result for EVERY call id so the
        //    conversation stays API-valid (no dangling tool_call ids).
        for call in &tool_calls {
            let name = call.function.name.clone();
            let args_json = call.function.arguments.clone();

            // 4a. Allow-list gate: a call the agent isn't permitted to make is
            //     refused with an error result (the model sees it and adapts).
            if !tools.iter().any(|t| t == &name) {
                let result = format!("error: tool {name} not permitted for this agent");
                convo.push_tool(call.id.clone(), result);
                continue;
            }

            // 4b. Risky calls (write/delete/edit/bash, and sec tools with risk=true)
            //     must clear the tool-call classifier first. FAIL CLOSED: an
            //     unavailable classifier blocks the call (a sub-agent has no human to
            //     defer to). The sec_manager check mirrors the main-agent gate in
            //     approval.rs so a sub-agent cannot bypass risk=true sec tools.
            if tool_is_risky(&name)
                || ctx.sec_manager.as_ref().is_some_and(|m| m.tool_risk(&name))
            {
                let verdict = crate::app::harness::classify_toolcall(
                    &client,
                    &config,
                    &settings,
                    &task_intent,
                    &name,
                    &args_json,
                )
                .await;
                if !verdict.available {
                    let result = format!("blocked: classifier unavailable ({})", verdict.reason);
                    convo.push_tool(call.id.clone(), result);
                    continue;
                }
                if !verdict.allow {
                    let result = format!("blocked by harness: {}", verdict.reason);
                    convo.push_tool(call.id.clone(), result);
                    continue;
                }
                // available && allow → fall through and run it.
            }

            // 4c. Permitted (and, if risky, classifier-approved) → run it.
            emit(
                &tx,
                AgentEvent::ToolStarted {
                    name: name.clone(),
                    args: args_json,
                },
            );
            let mut result = crate::tool::execute_tool(&ctx, call);
            // The `cd` tool returns a `CWD_CHANGE_PREFIX`-tagged target that only the
            // main runtime's interception knows how to apply (it repoints the LIVE
            // session). A sub-agent runs to completion in a fixed workspace and has no
            // persistent cwd to move, so translate the sentinel into a plain note here
            // rather than leak the internal marker into the sub-agent's transcript.
            if let Some(target) = result.strip_prefix(crate::tool::cd::CWD_CHANGE_PREFIX) {
                result = format!(
                    "note: changing the working directory is not supported inside a sub-agent (target was {target}); continue using paths under your workspace"
                );
            }
            emit(
                &tx,
                AgentEvent::ToolDone {
                    name,
                    result: result.clone(),
                },
            );
            convo.push_tool(call.id.clone(), result);
        }
        // Turn committed (assistant + every tool result): snapshot the history
        // so the UI sees this step's tool round.
        emit(&tx, AgentEvent::Snapshot(convo.messages().to_vec()));

        // Advance counter; check explicit cap (None = unbounded).
        step += 1;
        if let Some(cap) = max_steps {
            if step >= cap {
                // Explicit cap exhausted without a no-tool finish. Clean the last
                // assistant text the same way the natural-finish path does (unwrap
                // <content>, strip tool-call markup, empty-fallback) so a budget-
                // exhausted report is never leaked-tags or empty either.
                let final_text = if last_text.trim().is_empty() {
                    "(stopped: step budget reached)".to_string()
                } else {
                    finalize_report(&last_text)
                };
                emit(&tx, AgentEvent::Done(cap_report(final_text)));
                return;
            }
        }
        // Loop: the next step re-streams with the tool results in `convo`.
    }
}

/// Stream a single model reply and drain its events into a [`StreamOutcome`].
///
/// Opens a fresh inner [`StreamEvent`] channel, dispatches
/// [`OpenRouterClient::stream_complete`] on the resolved route, and folds the
/// drained events: `Token` deltas append to the text (and are re-emitted as
/// [`AgentEvent::Token`]), `ToolCalls` are collected, `Error` is captured.
/// `Reasoning` / `Usage` are display-only accounting the sub-agent ignores.
async fn stream_step(
    client: &Arc<OpenRouterClient>,
    resolved: &Resolved,
    history: Vec<crate::dto::chat::ChatMessage>,
    tools: &[String],
    tx: &UnboundedSender<AgentEvent>,
) -> StreamOutcome {
    let (inner_tx, mut inner_rx) = mpsc::unbounded_channel();
    // Dispatch the stream as a task so we can drain its events concurrently. The
    // task owns its sender; the channel closes when it finishes, ending the drain.
    let c = Arc::clone(client);
    let model_id = resolved.model_id.clone();
    let provider = resolved.provider().to_string();
    let effort = resolved.effort.clone();
    let endpoint = resolved.endpoint.clone();
    let api_key = resolved.api_key.clone();
    // Advertise only this agent's allow-list (owned clone moved into the task).
    let advertise = tools.to_vec();
    let send = tokio::spawn(async move {
        let conn = crate::service::openrouter::Conn {
            endpoint: &endpoint,
            api_key: &api_key,
        };
        // Sub-agents advertise only their own allow-list and receive NO MCP tools
        // (kept simple — MCP is a main-agent capability for now), so pass an empty
        // `mcp_tools` slice.
        let _ = c
            .stream_complete(conn, &model_id, &provider, &effort, history, &advertise, &[], None, inner_tx)
            .await;
    });

    let mut outcome = StreamOutcome::default();
    while let Some(event) = inner_rx.recv().await {
        match event {
            StreamEvent::Token(t) => {
                if !t.is_empty() {
                    outcome.text.push_str(&t);
                    emit(tx, AgentEvent::Token(t));
                }
            }
            StreamEvent::ToolCalls(calls) => {
                outcome.tool_calls = calls;
            }
            StreamEvent::Error(e) => {
                outcome.error = Some(e);
            }
            // Capture the usage chunk so the caller can accumulate spend.
            StreamEvent::Usage { prompt_tokens, completion_tokens, cost, .. } => {
                outcome.usage = Some((prompt_tokens, completion_tokens, cost));
            }
            // Display-only / accounting events the sub-agent doesn't track.
            StreamEvent::Reasoning(_)
            | StreamEvent::Done
            | StreamEvent::Compacted { .. }
            | StreamEvent::HarnessVerdict { .. }
            | StreamEvent::EndpointsLoaded { .. }
            | StreamEvent::EndpointsError { .. } => {}
        }
    }
    // The sender task has nothing left to emit; await it so it's fully joined
    // (it only ever returns `()` and never panics — every failure is an `Error`
    // event already folded above).
    let _ = send.await;
    outcome
}

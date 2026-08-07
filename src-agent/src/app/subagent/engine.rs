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

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

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
/// Altitude-aware: a substantial or structured response (long, multi-line, or
/// containing markdown headings/tables/lists) is NEVER a stall. Only short,
/// bodyless lead-ins or dangling colons qualify.
///
/// Criteria for NOT a stall (any one is enough to return false):
/// - trimmed length >= 300 chars
/// - contains a newline (multi-line = has a body)
/// - contains "##" (markdown heading)
/// - contains "| " (table row)
/// - contains "- " (list item)
///
/// A stall requires ALL of the following (after ruling out the above):
/// - trimmed text is empty, OR
/// - trimmed text ends with `:` (classic "Let me read…:" cliffhanger), OR
/// - trimmed text starts with a known procrastination phrase (case-insensitive)
fn is_stall(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // Substantial -> long, multi-line, or structured (headings/tables/lists). Never a stall.
    let substantial = t.len() >= 300
        || t.contains('\n')
        || t.contains("##")
        || t.contains("| ")
        || t.contains("- ");
    if substantial {
        return false;
    }
    // Short + bodyless: a "let me..."/"next I..." lead-in or a dangling colon.
    let lower = t.to_lowercase();
    let lead_in = [
        "let me", "i'll", "i will", "let's", "now i", "next,", "next i", "first,",
    ]
    .iter()
    .any(|p| lower.starts_with(p));
    t.ends_with(':') || lead_in
}

/// One drained stream result: the assistant text, any requested tool calls,
/// a fatal error if the stream failed, and the optional usage tuple from the
/// final `StreamEvent::Usage` chunk (prompt_tokens, completion_tokens,
/// cached_tokens, cost).
#[derive(Default)]
struct StreamOutcome {
    text: String,
    /// Display-only reasoning/thinking accumulated from the `delta.reasoning`
    /// channel; committed onto the assistant message so the viewer renders it.
    reasoning: String,
    /// OpenRouter `reasoning_details` merged (by index) across streaming chunks.
    /// Carried onto a tool-call assistant message so the sub-agent replays its
    /// chain-of-thought (incl. signatures) on the next continuation request.
    reasoning_details: Vec<crate::dto::chat::ReasoningDetail>,
    tool_calls: Vec<ToolCall>,
    error: Option<String>,
    /// Last-seen usage chunk: (prompt_tokens, completion_tokens, cached_tokens, cost).
    /// `None` when the provider emitted no Usage event for this step.
    /// `cost` is overlay-corrected when the provider reports 0.0 (see [`stream_step`]).
    usage: Option<(u64, u64, u64, f64)>,
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
    if lower.starts_with(OPEN)
        && lower.ends_with(CLOSE)
        && trimmed.len() >= OPEN.len() + CLOSE.len()
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

/// A report that carries no real deliverable — empty, the `(no report)`
/// placeholder, or content that is ONLY an inline think/thinking/thought block
/// (the agent's answer went to the reasoning channel). Used to decide whether to
/// fall back to the sub-agent's reasoning when building the delivered report.
fn report_is_blank(report: &str) -> bool {
    let t = report.trim();
    if t.is_empty() || t == "(no report)" {
        return true;
    }
    strip_think_blocks(t).trim().is_empty()
}

/// Remove `<think>…</think>`, `<thinking>…</thinking>`, and `<thought>…</thought>`
/// blocks so a message that is only inline thinking registers as blank. Matches
/// the common lowercase tag forms; dangling/unmatched tags are left as-is.
fn strip_think_blocks(s: &str) -> String {
    let mut out = s.to_string();
    for (open, close) in [
        ("<think>", "</think>"),
        ("<thinking>", "</thinking>"),
        ("<thought>", "</thought>"),
    ] {
        while let Some(o) = out.find(open) {
            let Some(rel) = out[o..].find(close) else {
                break;
            };
            let end = o + rel + close.len();
            out.replace_range(o..end, "");
        }
    }
    out
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
    mcp_tools: Vec<crate::dto::openrouter::ToolDef>,
    ctx: ToolCtx,
    mut convo: Conversation,
    task_intent: String,
    max_steps: Option<usize>,
    tx: UnboundedSender<AgentEvent>,
    mut inject_rx: UnboundedReceiver<String>,
    // Display identity for this run, used ONLY for the `error.log` line below (a
    // dead sub-agent otherwise leaves no trace outside its own — often unread —
    // transcript). `agent_name` is the agent-def name (e.g. "general"); `agent_id`
    // is the per-session sub-agent id assigned by the orchestrator at spawn.
    agent_name: String,
    agent_id: usize,
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
        // Injection drain (turn-boundary steering): fold any messages pushed onto
        // this sub-agent's injection channel since the last turn into the isolated
        // history as fresh `user` turns BEFORE this step streams, so the model sees
        // them on its NEXT model call (never mid-stream). Non-blocking: `try_recv`
        // until empty/closed. Each is mirrored to the orchestrator as an `Injected`
        // event (flat transcript) and, once any landed, a single `Snapshot` (the
        // structured viewer history), so a human watching the `$` panel sees the
        // steer. A closed channel (the sender dropped) simply drains nothing.
        let mut injected_any = false;
        while let Ok(msg) = inject_rx.try_recv() {
            convo.push_user(msg.clone());
            emit(&tx, AgentEvent::Injected(msg));
            injected_any = true;
        }
        if injected_any {
            emit(&tx, AgentEvent::Snapshot(convo.messages().to_vec()));
        }

        emit(&tx, AgentEvent::Step(step));

        // 1. Stream one model reply on a fresh per-step channel, then drain it.
        //    Advertise ONLY this agent's allow-list to the model (the execution
        //    gate below stays as a backstop).
        let outcome =
            stream_step(&client, &resolved, convo.history(), &tools, &mcp_tools, &tx).await;

        // Fold this step's usage into the running totals (best-effort: a step
        // with no Usage chunk simply contributes nothing). tokens_in is
        // reported as-is (current context size), tokens_out and cost are summed.
        // Emit a UsageReport after EVERY step so the SubAgent struct always
        // holds the latest accumulated spend — on kill/abort the orchestrator
        // has already rolled each prior step into the parent counters + ledger
        // (loses at most the in-flight step whose Usage chunk never arrived).
        if let Some((pt, ct, cached, c)) = outcome.usage {
            let (next_out, next_cost) =
                super::usage_math::accumulate_step(acc_tokens_out, acc_cost, ct, c);
            acc_tokens_out = next_out;
            acc_cost = next_cost;
            emit(
                &tx,
                AgentEvent::UsageReport {
                    model_id: resolved.model_id.clone(),
                    tokens_in: pt,
                    tokens_out: acc_tokens_out,
                    // Per-step completion tokens + cost (not cumulative) so the
                    // orchestrator can ledger each step independently and survive
                    // a mid-run kill without losing earlier steps.
                    step_tokens_out: ct,
                    step_tokens_cached: cached,
                    step_cost: c,
                    cost: acc_cost,
                },
            );
        }

        // A fatal stream error ends the run immediately. Beyond the in-memory
        // `AgentEvent::Error` (folded into `sa.status`/transcript by the orchestrator),
        // also record it to the global error.log — otherwise a sub-agent that dies
        // unattended (no human watching the `$` panel) leaves no trace anywhere a
        // human is likely to look.
        if let Some(err) = outcome.error {
            crate::model::store::append_global_error_log(
                "subagent",
                &format!("agent '{agent_name}' #{agent_id} died: {err}"),
            );
            emit(&tx, AgentEvent::Error(err));
            return;
        }

        // Decode any echoed-back escaped reasoning tag BEFORE this text goes
        // anywhere: it is the single upstream source for every downstream use
        // this step — the isolated conversation commit (both the no-tool and
        // tool-call branches below), the stall-nudge commit, and the delivered
        // report (via `finalize_report` / `last_text`). Mirrors the interactive
        // engine's `final_answer` + `turn.rs` tool-call-turn decode; only decode,
        // strip nothing else.
        let assistant_text = crate::dto::chat::unescape_reasoning_tags(&outcome.text).into_owned();
        let tool_calls = outcome.tool_calls;
        // Attach this step's captured thinking to the committed assistant message
        // so the full-screen viewer can render it. `None` when the model emitted
        // no reasoning this step.
        let reasoning = {
            let r = outcome.reasoning;
            (!r.trim().is_empty()).then_some(r)
        };
        // Structured reasoning_details for THIS step: `None` when the model emitted
        // none. Only attached to a tool-call assistant message (a tool round-trip
        // follows, so replaying the signed chain-of-thought preserves continuity).
        let reasoning_details = {
            let d = outcome.reasoning_details;
            (!d.is_empty()).then_some(d)
        };
        if !assistant_text.trim().is_empty() {
            last_text = assistant_text.clone();
        }

        // 2. Commit the assistant turn into the isolated history (with tool calls
        //    when present so the tool results can answer them), carrying the
        //    step's captured reasoning so the viewer renders the thinking block.
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
                convo.push_assistant(assistant_text, reasoning.clone(), false);
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
            convo.push_assistant(assistant_text, None, false);
            // Final turn committed: snapshot the full history before finishing.
            emit(&tx, AgentEvent::Snapshot(convo.messages().to_vec()));
            // Deliver the CLEANED report (tags stripped, with empty-fallback) so a
            // weak model's wrapped output never reaches the orchestrator as empty
            // or with a leaked </content>. If the model produced NO usable content
            // (empty, "(no report)", or only an inline think tag) — i.e. it spent
            // its final turn in the reasoning channel — fall back to that reasoning
            // so the orchestrator gets real text instead of a blank report.
            let delivered = if report_is_blank(&report) {
                match &reasoning {
                    Some(r) if !r.trim().is_empty() => format!(
                        "(the sub-agent finished without a written report; its reasoning follows)\n\n{}",
                        r.trim()
                    ),
                    _ => report,
                }
            } else {
                report
            };
            emit(&tx, AgentEvent::Done(cap_report(delivered)));
            return;
        }
        convo.push_assistant_with_tools(
            assistant_text,
            tool_calls.clone(),
            reasoning.clone(),
            reasoning_details,
        );

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

            // 4b. Risky calls (write/delete/edit/bash) must clear the tool-call
            //     classifier first. FAIL CLOSED: an unavailable classifier blocks
            //     the call (a sub-agent has no human to defer to).
            // sec_* tools are harness-exempt (see approval.rs) — only builtin risky tools gate.
            if tool_is_risky(&name) {
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

            // 4b2. SDLC path ownership gate: reject write/edit/delete to paths
            //      owned by a DIFFERENT active graph node (glob matching).
            if matches!(name.as_str(), "write" | "edit" | "delete") {
                let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
                let args: serde_json::Value =
                    serde_json::from_str(&sanitized).unwrap_or(serde_json::json!({}));
                if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                    if !path.trim().is_empty() {
                        if let Some(session_dir) = &ctx.session_dir {
                            if let Ok(conn) = crate::model::msglog::open(session_dir) {
                                let _ = crate::model::sdlc::graph::ensure_tables(&conn);
                                if let Err(e) = crate::model::sdlc::graph::check_path_ownership(
                                    &conn,
                                    ctx.sdlc_active_node_id.as_deref(),
                                    path.trim(),
                                ) {
                                    emit(
                                        &tx,
                                        AgentEvent::ToolDone {
                                            name: name.clone(),
                                            result: e,
                                        },
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                }
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
/// [`AgentEvent::Token`]), `ToolCalls` are collected, `Error` is captured, and
/// `Reasoning` deltas accumulate into a parallel buffer committed onto the
/// assistant message (so the viewer renders the thinking). `Usage` is accounting.
async fn stream_step(
    client: &Arc<OpenRouterClient>,
    resolved: &Resolved,
    history: Vec<crate::dto::chat::ChatMessage>,
    tools: &[String],
    mcp_tools: &[crate::dto::openrouter::ToolDef],
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
    // Overlay lookup copies — kept on this side of the spawn so Usage handling
    // can price a 0.0 provider cost without racing the moved task locals.
    let overlay_model_id = model_id.clone();
    let overlay_endpoint = endpoint.clone();
    let api_key = resolved.api_key.clone();
    // OAuth identity + wire type, threaded so a sub-agent resolved onto a Codex /
    // Kilo OAuth route dispatches through the right transport with a refreshable
    // token (all "" / OpenAiCompatible for a static-key route).
    let api_type = resolved.api_type;
    let account_id = resolved.account_id.clone();
    let oauth_uuid = resolved.oauth_uuid.clone();
    // koma-free install id (X-Koma), threaded like the OAuth identity above so a
    // sub-agent that inherits a koma-free route keeps a valid rate-limit bucket;
    // "" for every other route, so non-koma-free sends are unchanged.
    let install_id = resolved.install_id.clone();
    // Advertise only this agent's allow-list (owned clone moved into the task).
    let advertise = tools.to_vec();
    // Owned clone of the inherited MCP tool defs, moved into the task alongside
    // `advertise` (same pattern — see doc comment above `stream_step`).
    let mcp_tools = mcp_tools.to_vec();
    let send = tokio::spawn(async move {
        let conn = crate::service::openrouter::Conn {
            endpoint: &endpoint,
            api_key: &api_key,
            api_type,
            account_id: &account_id,
            oauth_uuid: &oauth_uuid,
            install_id: &install_id,
        };
        // Sub-agents advertise their own allow-list PLUS any connected MCP tools,
        // inherited automatically from the shared manager (see `spawn_subagent`) —
        // exactly like the main agent's advertise fold (run.rs:447-456).
        let _ = c
            .stream_complete(
                conn, &model_id, &provider, &effort, history, &advertise, &mcp_tools, None,
                inner_tx,
            )
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
            // When the provider hardcodes / omits cost (Codex, Claude, many
            // direct APIs → 0.0), fall back to the curated catalogue overlay —
            // same rule the main interactive loop applies in `turn.rs`. Without
            // this, multi-step sub-agents always report $0 and the parent
            // footer / ledger never see the spend.
            StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                cost,
            } => {
                let eff_cost = super::usage_math::effective_step_cost(
                    &overlay_endpoint,
                    &overlay_model_id,
                    prompt_tokens,
                    cached_tokens,
                    completion_tokens,
                    cost,
                );
                outcome.usage = Some((prompt_tokens, completion_tokens, cached_tokens, eff_cost));
            }
            // Accumulate the model's thinking into a parallel buffer so the
            // committed assistant message carries it (the viewer renders it as a
            // dim/italic block). Display-only: never re-emitted as content.
            StreamEvent::Reasoning(t) => {
                outcome.reasoning.push_str(&t);
            }
            // Merge structured reasoning_details (by index) so the tool-call
            // assistant message can replay the model's signed chain-of-thought.
            StreamEvent::ReasoningDetails(d) => {
                crate::dto::chat::merge_reasoning_details(&mut outcome.reasoning_details, d);
            }
            // Lifecycle / accounting events the sub-agent doesn't track here.
            StreamEvent::Done
            | StreamEvent::Compacted { .. }
            | StreamEvent::HarnessVerdict { .. }
            | StreamEvent::EndpointsLoaded { .. }
            | StreamEvent::EndpointsError { .. }
            | StreamEvent::Retrying { .. } => {}
        }
    }
    // The sender task has nothing left to emit; await it so it's fully joined
    // (it only ever returns `()` and never panics — every failure is an `Error`
    // event already folded above).
    let _ = send.await;
    outcome
}

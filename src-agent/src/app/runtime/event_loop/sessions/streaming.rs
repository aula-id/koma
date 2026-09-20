use std::sync::Arc;

use crate::app::state::AppState;
use crate::service::{openrouter::OpenRouterClient, StreamEvent};

use super::super::super::stream::{advance_turn, finish_stream};
use super::super::drains::apply_compaction_result;
use super::super::MIN_COMPACT_ANIM;

/// Drain this session's `active_rx` stream (Token/Reasoning/Usage/ToolCalls/Done/Error/Retrying/Compacted).
/// Returns true if any event was processed.
pub(super) fn drain_stream(
    state: &mut AppState,
    idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    let mut dirty = false;

    if let Some(mut rx) = state.rest.sessions[idx].active_rx.take() {
        let mut still_streaming = true;
        while let Ok(event) = rx.try_recv() {
            dirty = true;
            match event {
                StreamEvent::Token(t) => {
                    state.rest.sessions[idx].append_token(&t);
                    // Status is per-session (C6): set it on THIS session's slot. The
                    // projection reads `fg().status` per client, so a background session's
                    // stream only flashes "streaming" in the client(s) viewing it.
                    state.rest.sessions[idx].status = "streaming".into();
                }
                StreamEvent::Reasoning(t) => {
                    // Accumulate the model's thinking into the parallel buffer;
                    // `dirty` is already set so it animates in like content.
                    state.rest.sessions[idx].append_reasoning(&t);
                    state.rest.sessions[idx].status = "thinking".into();
                }
                StreamEvent::ReasoningDetails(d) => {
                    // Merge structured reasoning_details (by index) into this
                    // session's replay buffer; drained on the tool-call commit and
                    // echoed back on tool-continuation requests (OpenRouter only).
                    crate::dto::chat::merge_reasoning_details(
                        &mut state.rest.sessions[idx].stream_reasoning_details,
                        d,
                    );
                }
                StreamEvent::ContextPrepared {
                    prompt_tokens,
                    effective_window,
                } => {
                    let rt = &mut state.rest.sessions[idx];
                    rt.context_usage = Some(crate::service::context_limits::ContextUsage {
                        prompt_tokens,
                        effective_window,
                        estimated: true,
                    });
                    // The previous request's cache hit does not describe this prompt.
                    rt.tokens_cached = 0;
                }
                StreamEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                    cached_tokens,
                    cost,
                } => {
                    // Stash for the assistant-commit step; do NOT break — usage
                    // arrives just before Done.
                    state.rest.sessions[idx].pending_usage =
                        Some((prompt_tokens, completion_tokens, cost));
                    // Cached-prompt-token count for THIS prompt (current context,
                    // like tokens_in — not cumulative). Set straight away on THIS
                    // session so its readout can show the cache hit even on a tool
                    // round-trip that commits no assistant text.
                    state.rest.sessions[idx].tokens_cached = cached_tokens;
                    if prompt_tokens > 0 {
                        if let Some(usage) = state.rest.sessions[idx].context_usage.as_mut() {
                            usage.prompt_tokens = prompt_tokens;
                            usage.estimated = false;
                        }
                    }
                    // Latch: once any response reports cache hits we know this
                    // provider supports prompt caching. Never reset.
                    if cached_tokens > 0 {
                        state.rest.sessions[idx].provider_caches = true;
                    }
                }
                StreamEvent::ToolCalls(calls) => {
                    // Stash the requested tool calls; do NOT break — Done follows
                    // and `advance_turn` consumes them there.
                    state.rest.sessions[idx].pending_tool_calls = calls;
                }
                StreamEvent::Done => {
                    // Drive the turn: commit the assistant message and either end
                    // the turn or run tools + continue (which spawns the next task
                    // into a fresh active_rx).
                    advance_turn(state, idx, client, handle);
                    still_streaming = false;
                    break;
                }
                StreamEvent::Error(e) => {
                    // Surface the error and halt the whole turn (drop any
                    // half-stashed tool calls / step count / approval machine).
                    finish_stream(&mut state.rest, idx, Some(e));
                    state.rest.sessions[idx].agent_steps = 0;
                    state.rest.sessions[idx].main_stall_nudges = 0;
                    state.rest.sessions[idx].pending_tool_calls.clear();
                    state.rest.sessions[idx].awaiting_approval = false;
                    state.rest.sessions[idx].approval_reason = None;
                    state.rest.sessions[idx].tool_idx = 0;
                    state.rest.sessions[idx].tool_results.clear();
                    // Drop any TAC-classify park so a stream error can't leave the
                    // round parked on a verdict that will never resume.
                    state.rest.sessions[idx].awaiting_classify = false;
                    state.rest.sessions[idx].pending_classify_verdict = None;
                    // Clear THIS session's in-flight compaction animation so a failed
                    // compaction (e.g. null content decode error) doesn't leave the
                    // spinner stuck driving per-tick redraws indefinitely. Per-session (C4).
                    state.rest.sessions[idx].compact_anim_start = None;
                    state.rest.sessions[idx].compact_apply_at = None;
                    state.rest.sessions[idx].compact_pending = None;
                    // A stream error during a plan/mission-approval compaction flow (the
                    // post-approval turn OR the compactor call) aborts it: drop the
                    // one-shot seeds so they can't fire on a later `/compact`.
                    // No-op when no plan/mission flow is armed.
                    state.rest.sessions[idx].pending_plan_seed = false;
                    state.rest.sessions[idx].pending_plan_seed_body = None;
                    state.rest.sessions[idx].pending_mission_seed = None;
                    still_streaming = false;
                    break;
                }
                StreamEvent::Retrying {
                    attempt,
                    max,
                    delay_ms: _,
                } => {
                    // Transient upstream failure; the stream task is sleeping
                    // and will re-POST.  Update the status line but do NOT
                    // finish the stream or clear any tool/approval state.
                    state.rest.sessions[idx].status = format!("retrying {attempt}/{max}\u{2026}");
                    // still_streaming remains true — keep draining.
                }
                StreamEvent::Compacted { summary, kept_tail } => {
                    // The model is done; the task is finished either way.
                    state.rest.sessions[idx].current_task = None;
                    // Enforce a short cosmetic minimum so a fast compaction doesn't
                    // flash the animation. If we haven't shown the animation long
                    // enough yet, stash the result and defer the apply to a later
                    // tick (NON-blocking — never sleep).
                    // Per-session (C4): read/write THIS session's own animation clock,
                    // never the transient foreground — so a fast compaction on a
                    // background session defers + applies to ITS OWN slot.
                    let elapsed = state.rest.sessions[idx]
                        .compact_anim_start
                        .map(|t| t.elapsed())
                        .unwrap_or(MIN_COMPACT_ANIM);
                    if elapsed < MIN_COMPACT_ANIM {
                        if let Some(start) = state.rest.sessions[idx].compact_anim_start {
                            state.rest.sessions[idx].compact_apply_at =
                                Some(start + MIN_COMPACT_ANIM);
                            state.rest.sessions[idx].compact_pending = Some((summary, kept_tail));
                        } else {
                            apply_compaction_result(state, idx, client, handle, summary, kept_tail);
                        }
                    } else {
                        apply_compaction_result(state, idx, client, handle, summary, kept_tail);
                    }
                    still_streaming = false;
                    break;
                }
                // The advisory PC verdict is delivered on the dedicated
                // `harness_rx` channel (drained per-session below), and the
                // per-model provider endpoints on `endpoints_rx` (drained in
                // run_loop) — never on a streaming request's channel. So these arms
                // are unreachable here; ignore them to keep the match exhaustive
                // without affecting the stream.
                StreamEvent::HarnessVerdict { .. }
                | StreamEvent::EndpointsLoaded { .. }
                | StreamEvent::EndpointsError { .. } => {}
            }
        }
        if still_streaming {
            state.rest.sessions[idx].active_rx = Some(rx);
        }
    }

    // 1.5. Drain THIS session's advisory prompt-classifier (PC) verdict channel.
    //      Fully independent of streaming: a BLOCK verdict never cancels the turn
    //      (it already proceeded) — it only raises an advisory toast. This is
    //      PER-SESSION so a background session's verdict is drained promptly
    //      instead of being stuck until the user swaps to it. The advisory toast is
    //      now per-session too (C6): raise it on `sessions[idx]` itself so the
    //      projection (`fg().toast`) surfaces it ONLY in the client(s) viewing this
    //      session — a verdict for a session no one is looking at lands on that
    //      session's slot and is shown when (if) a client foregrounds it, never
    //      hijacking another window. take() the receiver so the arm can mutate
    //      `state.rest`; put it back unless the PC task finished / delivered.
    if let Some(mut hrx) = state.rest.sessions[idx].harness_rx.take() {
        let mut keep = true;
        while let Ok(event) = hrx.try_recv() {
            if let StreamEvent::HarnessVerdict { allow, reason } = event {
                if !allow {
                    let reason = if reason.is_empty() {
                        "flagged".into()
                    } else {
                        reason
                    };
                    state.rest.sessions[idx].set_toast(format!("harness flagged: {reason}"));
                    dirty = true;
                }
                // One verdict per turn; stop listening on this channel.
                keep = false;
                break;
            }
        }
        if keep {
            state.rest.sessions[idx].harness_rx = Some(hrx);
        }
    }

    dirty
}

#[cfg(test)]
mod context_usage_tests {
    use super::*;
    use crate::app::{mode::Mode, state::SessionRuntime};

    #[test]
    fn request_context_is_session_local_and_provider_usage_replaces_estimate() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut state = AppState::new(Mode::Chat);
        state.rest.sessions.push(SessionRuntime::new());
        state.rest.sessions[1].tokens_in = 240_000;
        state.rest.sessions[1].tokens_cached = 200_000;
        state.rest.sessions[1].tokens_out = 1_300_000;
        state.rest.sessions[1].cost = 232.9321;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.rest.sessions[1].active_rx = Some(rx);
        tx.send(StreamEvent::ContextPrepared {
            prompt_tokens: 54_000,
            effective_window: 300_000,
        })
        .unwrap();
        assert!(drain_stream(&mut state, 1, &None, runtime.handle()));
        let estimate = state.rest.sessions[1].context_usage.unwrap();
        assert_eq!(estimate.prompt_tokens, 54_000);
        assert_eq!(estimate.effective_window, 300_000);
        assert!(estimate.estimated);
        assert_eq!(state.rest.sessions[1].tokens_cached, 0);
        assert!(state.rest.sessions[0].context_usage.is_none());

        // A provider omitting prompt usage must not turn the estimate into 0%.
        tx.send(StreamEvent::Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            cost: 0.0,
        })
        .unwrap();
        drain_stream(&mut state, 1, &None, runtime.handle());
        assert_eq!(state.rest.sessions[1].context_usage, Some(estimate));

        // Even an empty/tool-only response can confirm the latest prompt.
        tx.send(StreamEvent::Usage {
            prompt_tokens: 51_500,
            completion_tokens: 25,
            cached_tokens: 49_400,
            cost: 0.01,
        })
        .unwrap();
        drain_stream(&mut state, 1, &None, runtime.handle());
        let reported = state.rest.sessions[1].context_usage.unwrap();
        assert_eq!(reported.prompt_tokens, 51_500);
        assert_eq!(reported.effective_window, 300_000);
        assert!(!reported.estimated);
        assert_eq!(state.rest.sessions[1].tokens_cached, 49_400);
        let snapshot = crate::ipc::snapshot::build_snapshot(&state);
        let shadow =
            crate::app::runtime::client_shadow::shadow_session_runtime(&snapshot.sessions[1]);
        assert_eq!(shadow.context_usage, Some(reported));
        assert_eq!(shadow.tokens_cached, 49_400);
        // Display telemetry never changes the billed/cumulative ledger counters.
        assert_eq!(state.rest.sessions[1].tokens_in, 240_000);
        assert_eq!(state.rest.sessions[1].tokens_out, 1_300_000);
        assert_eq!(state.rest.sessions[1].cost, 232.9321);

        // A new request snapshots its own smaller/fallback window and cache state.
        tx.send(StreamEvent::ContextPrepared {
            prompt_tokens: 32_000,
            effective_window: 128_000,
        })
        .unwrap();
        drain_stream(&mut state, 1, &None, runtime.handle());
        let next = state.rest.sessions[1].context_usage.unwrap();
        assert_eq!(next.effective_window, 128_000);
        assert_eq!(next.prompt_tokens, 32_000);
        assert!(next.estimated);
        assert_eq!(state.rest.sessions[1].tokens_cached, 0);
    }
}

use std::sync::Arc;

use crate::app::state::AppState;
use crate::service::{openrouter::OpenRouterClient, StreamEvent};

use super::super::drains::apply_compaction_result;
use super::super::MIN_COMPACT_ANIM;
use super::super::super::stream::{advance_turn, finish_stream};

/// Drain this session's `active_rx` stream (Token/Reasoning/Usage/ToolCalls/Done/Error/Compacted).
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
                    state.rest.status = "streaming".into();
                }
                StreamEvent::Reasoning(t) => {
                    // Accumulate the model's thinking into the parallel buffer;
                    // `dirty` is already set so it animates in like content.
                    state.rest.sessions[idx].append_reasoning(&t);
                    state.rest.status = "thinking".into();
                }
                StreamEvent::Usage { prompt_tokens, completion_tokens, cached_tokens, cost } => {
                    // Stash for the assistant-commit step; do NOT break — usage
                    // arrives just before Done.
                    state.rest.sessions[idx].pending_usage = Some((prompt_tokens, completion_tokens, cost));
                    // Cached-prompt-token count for THIS prompt (current context,
                    // like tokens_in — not cumulative). Set straight away on THIS
                    // session so its readout can show the cache hit even on a tool
                    // round-trip that commits no assistant text.
                    state.rest.sessions[idx].tokens_cached = cached_tokens;
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
                    state.rest.sessions[idx].pending_tool_calls.clear();
                    state.rest.sessions[idx].awaiting_approval = false;
                    state.rest.sessions[idx].approval_reason = None;
                    state.rest.sessions[idx].tool_idx = 0;
                    state.rest.sessions[idx].tool_results.clear();
                    // Clear THIS session's in-flight compaction animation so a failed
                    // compaction (e.g. null content decode error) doesn't leave the
                    // spinner stuck driving per-tick redraws indefinitely. Per-session (C4).
                    state.rest.sessions[idx].compact_anim_start = None;
                    state.rest.sessions[idx].compact_apply_at = None;
                    state.rest.sessions[idx].compact_pending = None;
                    still_streaming = false;
                    break;
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
                        let start = state.rest.sessions[idx].compact_anim_start.unwrap();
                        state.rest.sessions[idx].compact_apply_at = Some(start + MIN_COMPACT_ANIM);
                        state.rest.sessions[idx].compact_pending = Some((summary, kept_tail));
                        // Keep `waiting` true so the 8ms poll + per-tick redraw keep
                        // the animation running until the gate opens.
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
    //      (it already proceeded) — it only raises an advisory toast. This is now
    //      PER-SESSION so a background session's verdict is drained promptly
    //      instead of being stuck until the user swaps to it. The toast is a
    //      GLOBAL/foreground UI surface, so it is shown ONLY for the foreground
    //      session; a non-foreground session's verdict is drained + parked
    //      silently (no toast, no mode change) so it can't hijack the screen the
    //      user is looking at. take() the receiver so the arm can mutate
    //      `state.rest`; put it back unless the PC task finished / delivered.
    if let Some(mut hrx) = state.rest.sessions[idx].harness_rx.take() {
        // VIEWED BY SOME client this tick (C2)? The advisory toast is a single GLOBAL
        // surface, so only a session a client is actually looking at may raise it; a
        // session viewed by NOBODY drains its verdict silently (the old foreground-only
        // rule, generalised from the transient `foreground` cursor to the viewed set).
        let is_viewed = state
            .rest
            .sessions
            .get(idx)
            .map(|s| state.rest.viewed_sessions.contains(&s.id))
            .unwrap_or(false);
        let mut keep = true;
        while let Ok(event) = hrx.try_recv() {
            if let StreamEvent::HarnessVerdict { allow, reason } = event {
                if !allow {
                    // Viewed-by-some-client only: surface the advisory toast. A verdict
                    // for a session no client is looking at is drained but parked silently
                    // (dirty still set so the channel teardown is reflected, no visible toast).
                    if is_viewed {
                        let reason = if reason.is_empty() { "flagged".into() } else { reason };
                        state.rest.set_toast(format!("harness flagged: {reason}"));
                    }
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

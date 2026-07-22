//! Async streaming bridge: spawn / abort / finalize a request task.

mod turn;
mod tools;
mod spawn;
mod run;
mod knowledge;

pub(super) use turn::{finish_stream, advance_turn};
pub(crate) use turn::push_image_unsupported_notice;
pub(super) use tools::{dispatch_deferred, process_tools, run_tool};
pub(super) use run::{abort_current, start_stream_task};
pub(crate) use tools::resume_after_subagents;
// Re-exported for the daemon's detached-approval park-timeout (stage 11): the loop
// auto-denies a too-long parked call through the SAME deny path the TUI uses.
pub(crate) use tools::deny_all_pending;
pub(crate) use spawn::{spawn_or_queue, try_start_pending, SpawnFailReason, SpawnOutcome};
// The Phase 8 workspace-mutating primitive, shared by the `cd` tool interception
// (here) and the user `/cd` + `/adddir` commands.
pub(crate) use spawn::apply_workspace_change;
#[allow(unused_imports)]
pub(crate) use spawn::spawn_task;

/// Pick the assistant message content + display-reasoning for a FINAL turn.
/// Normally content is the answer and `reasoning` rides along (rendered gray).
/// But when the model left content empty and streamed its answer into the
/// reasoning channel (e.g. deepseek-v4-flash with reasoning on), promote the
/// reasoning to BE the content so it shows in the foreground and persists.
/// Returns (content, reasoning_to_attach, promoted) where `promoted` is `true`
/// ONLY in the reasoning->content promotion arm below — callers use it to flag
/// the stored assistant message (`ChatMessage::reasoning_promoted`) so
/// `Conversation::history` can exclude the raw chain-of-thought from the wire
/// on replay without touching storage/display. Empty content with no reasoning
/// -> ("", None, false).
///
/// Strips residual inline tool-call markup (`<tool_call>…</tool_call>` spans and
/// orphan tags) BEFORE the empty-content check so the committed assistant message
/// is never polluted by tags leaked from Hermes/Qwen/ChatML-style models. The
/// reasoning-promotion fallback is applied on the CLEANED content, so an all-tags
/// message (empty after stripping) still promotes reasoning correctly.
pub(super) fn final_answer(content: String, reasoning: Option<String>) -> (String, Option<String>, bool) {
    let content = crate::dto::chat::strip_tool_call_tags(&content);
    // Decode any escaped reasoning tag the model echoed back so the COMMITTED /
    // persisted message stores the REAL `<think>` (the outbound wire escape in
    // `dto::chat::escape_reasoning_tags` is transient). No-op when nothing was
    // escaped. Covers `finish_stream` + the no-tools branch of `advance_turn`.
    let content = crate::dto::chat::unescape_reasoning_tags(&content).into_owned();
    if content.trim().is_empty() {
        match reasoning {
            Some(r) if !r.trim().is_empty() => (r, None, true), // reasoning becomes the answer — PROMOTED
            _ => (String::new(), None, false),
        }
    } else {
        (content, reasoning, false) // normal: content is answer, reasoning rendered gray
    }
}

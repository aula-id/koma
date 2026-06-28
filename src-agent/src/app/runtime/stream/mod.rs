//! Async streaming bridge: spawn / abort / finalize a request task.

mod turn;
mod tools;
mod spawn;
mod run;

pub(super) use turn::{finish_stream, advance_turn};
pub(crate) use turn::push_image_unsupported_notice;
pub(super) use tools::{dispatch_deferred, process_tools, run_tool};
pub(super) use run::{abort_current, start_stream_task};
pub(crate) use tools::resume_after_subagents;
// Re-exported for the daemon's detached-approval park-timeout (stage 11): the loop
// auto-denies a too-long parked call through the SAME deny path the TUI uses.
pub(crate) use tools::deny_all_pending;
pub(crate) use spawn::{spawn_or_queue, try_start_pending, SpawnOutcome};
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
/// Returns (content, reasoning_to_attach). Empty content with no reasoning -> ("", None).
///
/// Strips residual inline tool-call markup (`<tool_call>…</tool_call>` spans and
/// orphan tags) BEFORE the empty-content check so the committed assistant message
/// is never polluted by tags leaked from Hermes/Qwen/ChatML-style models. The
/// reasoning-promotion fallback is applied on the CLEANED content, so an all-tags
/// message (empty after stripping) still promotes reasoning correctly.
pub(super) fn final_answer(content: String, reasoning: Option<String>) -> (String, Option<String>) {
    let content = crate::dto::chat::strip_tool_call_tags(&content);
    if content.trim().is_empty() {
        match reasoning {
            Some(r) if !r.trim().is_empty() => (r, None), // reasoning becomes the answer
            _ => (String::new(), None),
        }
    } else {
        (content, reasoning) // normal: content is answer, reasoning rendered gray
    }
}

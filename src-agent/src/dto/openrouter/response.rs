//! Inbound response types for the OpenRouter chat-completions API.
//!
//! Covers both non-streaming (full-body) responses used by `/compact` and
//! per-SSE-frame streaming chunks used by normal chat turns.

use serde::Deserialize;
use super::usage::Usage;

// ---------------------------------------------------------------------------
// Non-streaming response (used by /compact summary)
// ---------------------------------------------------------------------------

/// Top-level non-streaming response envelope.
///
/// OpenRouter returns an array of `choices`; we always take `choices[0]`.
#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
    /// Token/cost accounting (present when the request asked for it). Unused by
    /// the compaction caller today, kept for completeness with the streaming path.
    #[allow(dead_code)]
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// One completion alternative inside a non-streaming response.
#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
}

/// The finished message inside a non-streaming `Choice`.
///
/// `role` is received as a string (e.g. `"assistant"`) but is unused by the
/// caller; only `content` is extracted for the compaction summary.
///
/// `content` is `Option<String>` because some models (e.g. deepseek-v4-flash)
/// return `"content": null` on a non-streaming response instead of an empty
/// string. `#[serde(default)]` additionally handles an absent field. Callers
/// use `.unwrap_or_default()` (or the reasoning fallback) to treat null/absent
/// as an empty string.
///
/// `reasoning` carries a reasoning model's thinking text: some models (e.g. the
/// safeguard classifier) leave `content` empty and return their answer in this
/// field instead, so the classifier path falls back to it. Defaults to `None`
/// for models that don't emit it.
#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    #[allow(dead_code)]
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, alias = "reasoning_content", alias = "thinking")]
    pub reasoning: Option<String>,
}

// ---------------------------------------------------------------------------
// Streaming chunk (one SSE data: line)
// ---------------------------------------------------------------------------

/// One parsed SSE `data:` frame received during a streaming chat turn.
///
/// Most frames carry a one-element `choices` array whose `Delta::content` is
/// appended to the assistant bubble. The FINAL frame instead carries `usage`
/// (token counts + cost) with an empty `choices` array, so both fields are
/// handled independently.
#[derive(Debug, Deserialize)]
pub struct StreamChunk {
    pub choices: Vec<StreamChoice>,
    /// Present only on the terminal chunk: token/cost accounting for the whole
    /// generation. Absent (`None`) on every content-bearing chunk.
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// The single choice inside a streaming chunk.
#[derive(Debug, Deserialize)]
pub struct StreamChoice {
    pub delta: Delta,
    /// Set on the final content frame for this choice: `"stop"`, `"tool_calls"`,
    /// `"length"`, etc. Absent on intermediate frames. Defaults to `None`.
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// Incremental content fragment for the current assistant turn.
///
/// `content` is `None` on the first and last frames (role-only / finish-reason
/// frames); callers should skip `None` values and append `Some(text)` to the
/// growing assistant bubble. `tool_calls` carries incremental function-call
/// deltas (id / name / argument fragments) that the caller accumulates by index.
#[derive(Debug, Deserialize)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<String>,
    /// Incremental reasoning/thinking fragment for the current assistant turn,
    /// streamed in a SEPARATE channel from `content` when reasoning is enabled
    /// (via `/effort`). Accumulated into the assistant message's display-only
    /// reasoning block; never echoed back to the API. `None` on frames the model
    /// doesn't think on (and absent entirely for non-reasoning models).
    #[serde(default, alias = "reasoning_content", alias = "thinking")]
    pub reasoning: Option<String>,
    /// Incremental tool-call fragments. The model streams a tool call across
    /// several frames: the first carries the `id` + function `name`, subsequent
    /// frames append `arguments` text. Each entry's `index` selects the slot to
    /// merge into.
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// One streamed fragment of a tool call. `index` identifies which tool call in
/// the assistant turn this fragment belongs to (the model may stream several in
/// parallel). `id` and `function.name` arrive once; `function.arguments` is the
/// fragment to append to that call's growing argument string.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<FunctionDelta>,
}

/// The `function` slice of a [`ToolCallDelta`]: either the call's `name` (sent
/// on the first fragment) or a chunk of its JSON-encoded `arguments` string.
#[derive(Debug, Clone, Deserialize)]
pub struct FunctionDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

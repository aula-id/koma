//! Typed parser for the Responses API's Server-Sent Event stream.
//!
//! The SSE wire alternates `event:` and `data:` lines; the `data:` payload JSON
//! always duplicates the event name in its own `"type"` field, so we IGNORE the
//! `event:` lines entirely and internally-tag on `data.type`. Every enum carries
//! a `#[serde(other)] Other` arm (and every field defaults) so an unknown event,
//! a new field, or a partial usage object never fails the parse — we simply skip
//! what we don't model. Purely declarative + unit-tested; no I/O here.

use serde::Deserialize;
use serde_json::Value;

/// One parsed `data:` payload. Internally tagged on `type`; unknown types → `Other`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum ResponsesEvent {
    /// A chunk of assistant answer text.
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta { delta: String },
    /// A chunk of the model's reasoning-summary text (display-only).
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta { delta: String },
    /// A completed output item (a function call, or a reasoning item carrying the
    /// encrypted blob). Text items arrive via the deltas above, so a "message"
    /// item here is ignored.
    #[serde(rename = "response.output_item.done")]
    OutputItemDone { item: OutputItem },
    /// Terminal success: carries the final usage accounting.
    #[serde(rename = "response.completed")]
    Completed { response: CompletedResponse },
    /// Terminal failure: the payload's `response.error.message` (if present) is
    /// the human cause.
    #[serde(rename = "response.failed")]
    Failed {
        #[serde(default)]
        response: Option<Value>,
    },
    /// Transport-level error event.
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        code: Option<String>,
    },
    #[serde(other)]
    Other,
}

/// The `item` of a `response.output_item.done` event. Internally tagged on `type`;
/// a "message" item is unmodelled (`Other`) because its text already streamed.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum OutputItem {
    #[serde(rename = "function_call")]
    FunctionCall {
        name: String,
        arguments: String,
        call_id: String,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        encrypted_content: Option<String>,
    },
    #[serde(other)]
    Other,
}

/// The `response` object of a `response.completed` event.
#[derive(Debug, Default, Deserialize)]
pub(super) struct CompletedResponse {
    #[serde(default)]
    pub usage: Option<RespUsage>,
}

/// Token accounting on a completed response. All fields default so a partial or
/// absent usage object still deserializes.
#[derive(Debug, Default, Deserialize)]
pub(super) struct RespUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: Option<InputTokensDetails>,
}

/// The `input_tokens_details` sub-object: the cache-hit share of the input tokens.
#[derive(Debug, Default, Deserialize)]
pub(super) struct InputTokensDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}

/// Parse one `data:` payload (JSON) into a [`ResponsesEvent`]. Returns `None` on
/// non-JSON / unparseable input (a partial keepalive line, `[DONE]`, etc.), which
/// the caller simply skips.
pub(super) fn parse_event(data: &str) -> Option<ResponsesEvent> {
    serde_json::from_str(data).ok()
}

#[cfg(test)]
#[path = "sse_test.rs"]
mod tests;

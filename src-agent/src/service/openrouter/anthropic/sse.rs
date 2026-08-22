//! Typed parser for the Anthropic Messages API's Server-Sent Event stream.
//!
//! The SSE wire alternates `event:` and `data:` lines; the `data:` payload JSON
//! always duplicates the event name in its own `"type"` field, so we IGNORE the
//! `event:` lines entirely and internally-tag on `data.type` (exactly like the
//! codex transport). Every enum carries a `#[serde(other)] Other` arm (and every
//! field defaults) so an unknown event, a new field, or a `ping` keepalive never
//! fails the parse. `thinking` and `redacted_thinking` content blocks ARE
//! modeled (parsed and replayed on continuation requests) — only truly
//! unrecognized variants fall through to `Other` and are skipped. Purely
//! declarative + unit-tested; no I/O here.

use serde::Deserialize;

/// One parsed `data:` payload. Internally tagged on `type`; unknown types (incl.
/// `ping`) → `Other`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum AnthropicEvent {
    /// Opens the response: carries the input-side usage accounting.
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartBody },
    /// Opens one content block (text, tool_use, thinking, or redacted_thinking).
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlockStart,
    },
    /// A delta for the block at `index` (answer text, or a tool-input fragment).
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: BlockDelta },
    /// Closes the block at `index`. A tool_use block's accumulated input buffer is
    /// complete at this point (the driver also flushes at `message_stop`/EOF).
    #[serde(rename = "content_block_stop")]
    ContentBlockStop {
        #[allow(dead_code)]
        index: usize,
    },
    /// Top-level delta: the terminal `stop_reason` + the output-side usage.
    #[serde(rename = "message_delta")]
    MessageDelta {
        #[serde(default)]
        #[allow(dead_code)]
        delta: MessageDeltaBody,
        #[serde(default)]
        usage: Option<DeltaUsage>,
    },
    /// Terminal success sentinel.
    #[serde(rename = "message_stop")]
    MessageStop,
    /// Transport / API error event.
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        error: Option<ErrorBody>,
    },
    #[serde(other)]
    Other,
}

/// The `message` object of a `message_start` event.
#[derive(Debug, Default, Deserialize)]
pub(super) struct MessageStartBody {
    #[serde(default)]
    pub usage: Option<StartUsage>,
}

/// Input-side token accounting. All fields default so a partial usage object
/// still deserializes. `cache_read_input_tokens` is the cache-hit share.
#[derive(Debug, Default, Deserialize)]
pub(super) struct StartUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// The `content_block` of a `content_block_start` event. Internally tagged; any
/// unmodelled block type → `Other`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum ContentBlockStart {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        #[allow(dead_code)]
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
    /// Opens a thinking block. `thinking` seeds the text accumulator (usually ""
    /// — the body arrives via `thinking_delta`); `signature` is usually absent at
    /// start and streamed later via `signature_delta`.
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
    /// Opens a redacted (encrypted) thinking block; `data` is the opaque blob
    /// replayed verbatim on a continuation request.
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
    #[serde(other)]
    Other,
}

/// The `delta` of a `content_block_delta` event. Internally tagged; any
/// unmodelled delta type → `Other`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum BlockDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    /// A fragment of a thinking block's text — routed to the reasoning channel for
    /// live display AND accumulated (with the block's signature) for replay.
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    /// The cryptographic signature over a thinking block, streamed at its close.
    /// Accumulated (never displayed) so the block replays intact on continuation.
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
    #[serde(other)]
    Other,
}

/// The `delta` of a `message_delta` event (carries the stop reason). Parsed for
/// wire completeness / forward-compat; the driver treats `message_stop` as the
/// terminal, so `stop_reason` is not currently branched on.
#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct MessageDeltaBody {
    #[serde(default)]
    pub stop_reason: Option<String>,
}

/// Output-side token accounting on a `message_delta` event.
#[derive(Debug, Default, Deserialize)]
pub(super) struct DeltaUsage {
    #[serde(default)]
    pub output_tokens: u64,
}

/// The `error` object of an `error` event.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ErrorBody {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

/// Parse one `data:` payload (JSON) into an [`AnthropicEvent`]. Returns `None` on
/// non-JSON / unparseable input (a partial line, `[DONE]`, etc.), which the caller
/// simply skips.
pub(super) fn parse_event(data: &str) -> Option<AnthropicEvent> {
    serde_json::from_str(data).ok()
}

#[cfg(test)]
#[path = "sse_test.rs"]
mod tests;

//! Typed parser for the Anthropic Messages API's Server-Sent Event stream.
//!
//! The SSE wire alternates `event:` and `data:` lines; the `data:` payload JSON
//! always duplicates the event name in its own `"type"` field, so we IGNORE the
//! `event:` lines entirely and internally-tag on `data.type` (exactly like the
//! codex transport). Every enum carries a `#[serde(other)] Other` arm (and every
//! field defaults) so an unknown event, a new field, a `ping` keepalive, or a
//! `thinking` block we don't model never fails the parse — we simply skip what we
//! don't model. Purely declarative + unit-tested; no I/O here.

use serde::Deserialize;

/// One parsed `data:` payload. Internally tagged on `type`; unknown types (incl.
/// `ping`) → `Other`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum AnthropicEvent {
    /// Opens the response: carries the input-side usage accounting.
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartBody },
    /// Opens one content block (text, tool_use, or an ignored thinking block).
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

/// The `content_block` of a `content_block_start` event. Internally tagged; a
/// `thinking` / `redacted_thinking` block (we don't request thinking) → `Other`.
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
    #[serde(other)]
    Other,
}

/// The `delta` of a `content_block_delta` event. Internally tagged; a
/// `thinking_delta` / `signature_delta` (ignored) → `Other`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum BlockDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
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
mod tests {
    use super::*;

    #[test]
    fn message_start_captures_input_and_cache_usage() {
        let e = parse_event(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":100,"cache_read_input_tokens":80}}}"#,
        )
        .unwrap();
        match e {
            AnthropicEvent::MessageStart { message } => {
                let u = message.usage.unwrap();
                assert_eq!(u.input_tokens, 100);
                assert_eq!(u.cache_read_input_tokens, 80);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn message_start_usage_defaults_when_partial() {
        // Missing cache field defaults to 0; missing usage object → None.
        let e = parse_event(r#"{"type":"message_start","message":{"usage":{"input_tokens":5}}}"#)
            .unwrap();
        match e {
            AnthropicEvent::MessageStart { message } => {
                let u = message.usage.unwrap();
                assert_eq!(u.input_tokens, 5);
                assert_eq!(u.cache_read_input_tokens, 0);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn content_block_start_tool_use() {
        let e = parse_event(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"read","input":{}}}"#,
        )
        .unwrap();
        match e {
            AnthropicEvent::ContentBlockStart {
                index,
                content_block: ContentBlockStart::ToolUse { id, name },
            } => {
                assert_eq!(index, 1);
                assert_eq!(id, "tu_1");
                assert_eq!(name, "read");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn thinking_block_start_is_other() {
        let e = parse_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            AnthropicEvent::ContentBlockStart {
                content_block: ContentBlockStart::Other,
                ..
            }
        ));
    }

    #[test]
    fn text_delta_and_input_json_delta() {
        let e = parse_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
        )
        .unwrap();
        match e {
            AnthropicEvent::ContentBlockDelta {
                delta: BlockDelta::TextDelta { text },
                ..
            } => assert_eq!(text, "hello"),
            other => panic!("wrong variant: {other:?}"),
        }
        let e2 = parse_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"p\":"}}"#,
        )
        .unwrap();
        match e2 {
            AnthropicEvent::ContentBlockDelta {
                index,
                delta: BlockDelta::InputJsonDelta { partial_json },
            } => {
                assert_eq!(index, 1);
                assert_eq!(partial_json, "{\"p\":");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn thinking_delta_is_other() {
        let e = parse_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"x"}}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            AnthropicEvent::ContentBlockDelta {
                delta: BlockDelta::Other,
                ..
            }
        ));
    }

    #[test]
    fn message_delta_captures_output_usage_and_stop_reason() {
        let e = parse_event(
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}"#,
        )
        .unwrap();
        match e {
            AnthropicEvent::MessageDelta { delta, usage } => {
                assert_eq!(delta.stop_reason.as_deref(), Some("tool_use"));
                assert_eq!(usage.unwrap().output_tokens, 42);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn message_stop_and_content_block_stop() {
        assert!(matches!(
            parse_event(r#"{"type":"message_stop"}"#).unwrap(),
            AnthropicEvent::MessageStop
        ));
        assert!(matches!(
            parse_event(r#"{"type":"content_block_stop","index":1}"#).unwrap(),
            AnthropicEvent::ContentBlockStop { index: 1 }
        ));
    }

    #[test]
    fn error_event_carries_message() {
        let e = parse_event(
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )
        .unwrap();
        match e {
            AnthropicEvent::Error { error } => {
                let b = error.unwrap();
                assert_eq!(b.message.as_deref(), Some("Overloaded"));
                assert_eq!(b.kind.as_deref(), Some("overloaded_error"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn ping_and_unknown_are_tolerated() {
        assert!(matches!(
            parse_event(r#"{"type":"ping"}"#).unwrap(),
            AnthropicEvent::Other
        ));
        assert!(matches!(
            parse_event(r#"{"type":"message_brand_new_event"}"#).unwrap(),
            AnthropicEvent::Other
        ));
    }

    #[test]
    fn non_json_returns_none() {
        assert!(parse_event("[DONE]").is_none());
        assert!(parse_event("").is_none());
    }
}

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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn output_text_delta() {
        let e = parse_event(r#"{"type":"response.output_text.delta","delta":"hello"}"#).unwrap();
        match e {
            ResponsesEvent::OutputTextDelta { delta } => assert_eq!(delta, "hello"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reasoning_summary_delta() {
        let e = parse_event(
            r#"{"type":"response.reasoning_summary_text.delta","delta":"thinking"}"#,
        )
        .unwrap();
        match e {
            ResponsesEvent::ReasoningSummaryTextDelta { delta } => assert_eq!(delta, "thinking"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn function_call_item() {
        let e = parse_event(
            r#"{"type":"response.output_item.done","item":{"type":"function_call","name":"read","arguments":"{\"path\":\"a\"}","call_id":"call_1"}}"#,
        )
        .unwrap();
        match e {
            ResponsesEvent::OutputItemDone {
                item: OutputItem::FunctionCall { name, arguments, call_id },
            } => {
                assert_eq!(name, "read");
                assert_eq!(arguments, "{\"path\":\"a\"}");
                assert_eq!(call_id, "call_1");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reasoning_item_with_encrypted_content() {
        let e = parse_event(
            r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","encrypted_content":"BLOB"}}"#,
        )
        .unwrap();
        match e {
            ResponsesEvent::OutputItemDone {
                item: OutputItem::Reasoning { id, encrypted_content },
            } => {
                assert_eq!(id.as_deref(), Some("rs_1"));
                assert_eq!(encrypted_content.as_deref(), Some("BLOB"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reasoning_item_without_encrypted_content() {
        let e = parse_event(
            r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_2"}}"#,
        )
        .unwrap();
        match e {
            ResponsesEvent::OutputItemDone {
                item: OutputItem::Reasoning { id, encrypted_content },
            } => {
                assert_eq!(id.as_deref(), Some("rs_2"));
                assert_eq!(encrypted_content, None);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn completed_with_usage_and_cache() {
        let e = parse_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":100,"output_tokens":40,"input_tokens_details":{"cached_tokens":80}}}}"#,
        )
        .unwrap();
        match e {
            ResponsesEvent::Completed { response } => {
                let u = response.usage.unwrap();
                assert_eq!(u.input_tokens, 100);
                assert_eq!(u.output_tokens, 40);
                assert_eq!(u.input_tokens_details.unwrap().cached_tokens, 80);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn completed_usage_defaults_when_absent() {
        // No usage object at all → None, and the defaults hold when partial.
        let e = parse_event(r#"{"type":"response.completed","response":{}}"#).unwrap();
        match e {
            ResponsesEvent::Completed { response } => assert!(response.usage.is_none()),
            other => panic!("wrong variant: {other:?}"),
        }
        // Partial usage: missing output_tokens + details default to 0/None.
        let e2 = parse_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":5}}}"#,
        )
        .unwrap();
        match e2 {
            ResponsesEvent::Completed { response } => {
                let u = response.usage.unwrap();
                assert_eq!(u.input_tokens, 5);
                assert_eq!(u.output_tokens, 0);
                assert!(u.input_tokens_details.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn failed_event() {
        let e = parse_event(
            r#"{"type":"response.failed","response":{"error":{"message":"boom"}}}"#,
        )
        .unwrap();
        match e {
            ResponsesEvent::Failed { response } => {
                let msg = response.unwrap()["error"]["message"].as_str().unwrap().to_string();
                assert_eq!(msg, "boom");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn error_event() {
        let e =
            parse_event(r#"{"type":"error","message":"rate limited","code":"429"}"#).unwrap();
        match e {
            ResponsesEvent::Error { message, code } => {
                assert_eq!(message.as_deref(), Some("rate limited"));
                assert_eq!(code.as_deref(), Some("429"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn unknown_event_is_tolerated() {
        // A brand-new event type we don't model must parse to `Other`, not fail.
        let e = parse_event(r#"{"type":"response.output_item.added","item":{}}"#).unwrap();
        assert!(matches!(e, ResponsesEvent::Other));
        // An unknown output-item type inside a done event → OutputItem::Other.
        let e2 = parse_event(
            r#"{"type":"response.output_item.done","item":{"type":"message","role":"assistant"}}"#,
        )
        .unwrap();
        assert!(matches!(
            e2,
            ResponsesEvent::OutputItemDone { item: OutputItem::Other }
        ));
    }

    #[test]
    fn non_json_returns_none() {
        assert!(parse_event("[DONE]").is_none());
        assert!(parse_event("").is_none());
    }
}

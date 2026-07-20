//! NDJSON line parser for the Command Code `/alpha/generate` response stream.
//!
//! Tolerates bare NDJSON lines, `data:` SSE prefixes, comments (`:`), and
//! `[DONE]` sentinels — matching pi-commandcode-provider's `parseStreamEventLine`.

use serde::Deserialize;
use serde_json::Value;

/// A parsed NDJSON event from the `/alpha/generate` stream.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum CcEvent {
    TextDelta { text: String },
    ReasoningStart,
    ReasoningDelta { text: String },
    ReasoningEnd,
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: Value,
    },
    Finish {
        finish_reason: Option<String>,
        total_usage: Option<CcUsage>,
    },
    Error {
        error: Value,
    },
    Unknown,
}

/// Usage information from a `finish` event.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CcUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub input_token_details: Option<CcInputTokenDetails>,
}

/// Input token breakdown.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CcInputTokenDetails {
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
}

/// Parse a single line from the NDJSON stream.
///
/// Mirrors pi-commandcode-provider's `parseStreamEventLine`:
/// 1. Trim the line.
/// 2. Skip empty lines, comments (`:`), `event:` lines.
/// 3. Strip optional `data:` prefix.
/// 4. Skip `[DONE]`.
/// 5. Parse as JSON and match on the `type` field.
pub(super) fn parse_line(line: &str) -> Option<CcEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Skip comments and event: lines (SSE compatibility).
    if trimmed.starts_with(':') || trimmed.starts_with("event:") {
        return None;
    }
    // Strip optional data: prefix.
    let trimmed = if let Some(rest) = trimmed.strip_prefix("data:") {
        rest.trim()
    } else {
        trimmed
    };
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return None;
    }
    // Parse JSON.
    let v: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let obj = match v.as_object() {
        Some(o) => o,
        None => return Some(CcEvent::Unknown),
    };
    let typ = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "text-delta" => {
            let text = obj
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some(CcEvent::TextDelta { text })
        }
        "reasoning-start" => Some(CcEvent::ReasoningStart),
        "reasoning-delta" => {
            let text = obj
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some(CcEvent::ReasoningDelta { text })
        }
        "reasoning-end" => Some(CcEvent::ReasoningEnd),
        "tool-call" => {
            let tool_call_id = obj
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_name = obj
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input = obj
                .get("input")
                .or_else(|| obj.get("args"))
                .or_else(|| obj.get("arguments"))
                .cloned()
                .unwrap_or(serde_json::json!({}));
            Some(CcEvent::ToolCall {
                tool_call_id,
                tool_name,
                input,
            })
        }
        "finish" => {
            let finish_reason = obj
                .get("finishReason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let total_usage = obj
                .get("totalUsage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            Some(CcEvent::Finish {
                finish_reason,
                total_usage,
            })
        }
        "error" => {
            let error = obj.get("error").cloned().unwrap_or(serde_json::json!(null));
            Some(CcEvent::Error { error })
        }
        _ => Some(CcEvent::Unknown),
    }
}

/// Map a CC `finishReason` to koma's stop semantics:
/// - `tool-calls` → tools were called (caller checks tool_acc)
/// - `length` / `max_tokens` / `max-tokens` → hit token limit
/// - anything else → normal stop
#[allow(dead_code)] // kept for parity with pi; stream driver currently ignores reason
pub(super) fn map_finish_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("tool-calls") => "tool_calls",
        Some("length") | Some("max_tokens") | Some("max-tokens") | Some("max_output_tokens") => {
            "length"
        }
        _ => "stop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_delta() {
        let event = parse_line(r#"{"type":"text-delta","text":"Hello"}"#).unwrap();
        assert_eq!(
            event,
            CcEvent::TextDelta {
                text: "Hello".to_string()
            }
        );
    }

    #[test]
    fn parse_with_data_prefix() {
        let event = parse_line(r#"data: {"type":"text-delta","text":"Hi"}"#).unwrap();
        assert_eq!(
            event,
            CcEvent::TextDelta {
                text: "Hi".to_string()
            }
        );
    }

    #[test]
    fn parse_skip_done() {
        assert!(parse_line("[DONE]").is_none());
    }

    #[test]
    fn parse_skip_empty() {
        assert!(parse_line("").is_none());
        assert!(parse_line("  ").is_none());
    }

    #[test]
    fn parse_skip_comments() {
        assert!(parse_line(": this is a comment").is_none());
        assert!(parse_line("event: message").is_none());
    }

    #[test]
    fn parse_reasoning_events() {
        assert_eq!(
            parse_line(r#"{"type":"reasoning-start"}"#).unwrap(),
            CcEvent::ReasoningStart
        );
        assert_eq!(
            parse_line(r#"{"type":"reasoning-delta","text":"think"}"#).unwrap(),
            CcEvent::ReasoningDelta {
                text: "think".to_string()
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"reasoning-end"}"#).unwrap(),
            CcEvent::ReasoningEnd
        );
    }

    #[test]
    fn parse_tool_call() {
        let event = parse_line(
            r#"{"type":"tool-call","toolCallId":"c1","toolName":"read","input":{"path":"x"}}"#,
        )
        .unwrap();
        match event {
            CcEvent::ToolCall {
                tool_call_id,
                tool_name,
                input,
            } => {
                assert_eq!(tool_call_id, "c1");
                assert_eq!(tool_name, "read");
                assert_eq!(input, serde_json::json!({"path": "x"}));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn parse_finish() {
        let event = parse_line(
            r#"{"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":100,"outputTokens":50}}"#,
        )
        .unwrap();
        match event {
            CcEvent::Finish {
                finish_reason,
                total_usage,
            } => {
                assert_eq!(finish_reason.as_deref(), Some("stop"));
                let u = total_usage.unwrap();
                assert_eq!(u.input_tokens, 100);
                assert_eq!(u.output_tokens, 50);
            }
            _ => panic!("expected Finish"),
        }
    }

    #[test]
    fn parse_error() {
        let event = parse_line(r#"{"type":"error","error":{"message":"fail"}}"#).unwrap();
        assert!(matches!(event, CcEvent::Error { .. }));
    }

    #[test]
    fn map_finish_reason_values() {
        assert_eq!(map_finish_reason(Some("stop")), "stop");
        assert_eq!(map_finish_reason(Some("tool-calls")), "tool_calls");
        assert_eq!(map_finish_reason(Some("length")), "length");
        assert_eq!(map_finish_reason(Some("max_tokens")), "length");
        assert_eq!(map_finish_reason(None), "stop");
    }
}

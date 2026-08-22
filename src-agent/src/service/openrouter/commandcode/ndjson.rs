//! NDJSON line parser for the Command Code `/alpha/generate` response stream.
//!
//! Tolerates bare NDJSON lines, `data:` SSE prefixes, comments (`:`), and
//! `[DONE]` sentinels — matching pi-commandcode-provider's `parseStreamEventLine`.

use serde::Deserialize;
use serde_json::Value;

/// A parsed NDJSON event from the `/alpha/generate` stream.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum CcEvent {
    TextDelta {
        text: String,
    },
    ReasoningStart,
    ReasoningDelta {
        text: String,
    },
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
#[path = "ndjson_test.rs"]
mod tests;

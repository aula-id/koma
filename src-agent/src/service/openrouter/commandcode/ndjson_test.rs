#![allow(clippy::unwrap_used, clippy::expect_used)]
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

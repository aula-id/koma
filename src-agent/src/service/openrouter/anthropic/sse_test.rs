#![allow(clippy::unwrap_used, clippy::expect_used)]
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
fn thinking_block_start_parses_with_seed_and_signature() {
    let e = parse_event(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"seed","signature":"sig0"}}"#,
    )
    .unwrap();
    match e {
        AnthropicEvent::ContentBlockStart {
            index,
            content_block:
                ContentBlockStart::Thinking {
                    thinking,
                    signature,
                },
        } => {
            assert_eq!(index, 0);
            assert_eq!(thinking, "seed");
            assert_eq!(signature.as_deref(), Some("sig0"));
        }
        other => panic!("wrong variant: {other:?}"),
    }
    // The common empty-seed / no-signature start still parses (fields default).
    let e2 = parse_event(
        r#"{"type":"content_block_start","index":2,"content_block":{"type":"thinking","thinking":""}}"#,
    )
    .unwrap();
    assert!(matches!(
        e2,
        AnthropicEvent::ContentBlockStart {
            content_block: ContentBlockStart::Thinking {
                signature: None,
                ..
            },
            ..
        }
    ));
}

#[test]
fn redacted_thinking_block_start_parses() {
    let e = parse_event(
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"AAAA"}}"#,
    )
    .unwrap();
    match e {
        AnthropicEvent::ContentBlockStart {
            content_block: ContentBlockStart::RedactedThinking { data },
            ..
        } => assert_eq!(data, "AAAA"),
        other => panic!("wrong variant: {other:?}"),
    }
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
fn thinking_delta_and_signature_delta_parse() {
    let e = parse_event(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"x"}}"#,
    )
    .unwrap();
    match e {
        AnthropicEvent::ContentBlockDelta {
            index,
            delta: BlockDelta::ThinkingDelta { thinking },
        } => {
            assert_eq!(index, 0);
            assert_eq!(thinking, "x");
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let e2 = parse_event(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
    )
    .unwrap();
    match e2 {
        AnthropicEvent::ContentBlockDelta {
            delta: BlockDelta::SignatureDelta { signature },
            ..
        } => assert_eq!(signature, "sig"),
        other => panic!("wrong variant: {other:?}"),
    }
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

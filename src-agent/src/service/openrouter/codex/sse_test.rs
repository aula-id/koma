#![allow(clippy::unwrap_used, clippy::expect_used)]
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
    let e =
        parse_event(r#"{"type":"response.reasoning_summary_text.delta","delta":"thinking"}"#)
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
            item:
                OutputItem::FunctionCall {
                    name,
                    arguments,
                    call_id,
                },
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
            item:
                OutputItem::Reasoning {
                    id,
                    encrypted_content,
                },
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
            item:
                OutputItem::Reasoning {
                    id,
                    encrypted_content,
                },
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
    let e2 =
        parse_event(r#"{"type":"response.completed","response":{"usage":{"input_tokens":5}}}"#)
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
    let e =
        parse_event(r#"{"type":"response.failed","response":{"error":{"message":"boom"}}}"#)
            .unwrap();
    match e {
        ResponsesEvent::Failed { response } => {
            let msg = response.unwrap()["error"]["message"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(msg, "boom");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn error_event() {
    let e = parse_event(r#"{"type":"error","message":"rate limited","code":"429"}"#).unwrap();
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
        ResponsesEvent::OutputItemDone {
            item: OutputItem::Other
        }
    ));
}

#[test]
fn non_json_returns_none() {
    assert!(parse_event("[DONE]").is_none());
    assert!(parse_event("").is_none());
}

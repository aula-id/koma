#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Unit tests for the Anthropic request-mapping layer ([`super`]). Split out of
//! `request.rs` (loaded via `#[path] mod tests;`) to keep each source file within
//! the repo's ≤600-line budget; `use super::*` gives access to the private
//! request-shaping items exactly as an inline `mod tests` would.

use super::*;
use crate::dto::chat::{ChatMessage, FunctionCall, ReasoningDetail, Role, ToolCall};

fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        kind: "function".to_string(),
        function: FunctionCall {
            name: name.to_string(),
            arguments: args.to_string(),
        },
    }
}

/// Build a `thinking`-typed [`ReasoningDetail`] (the shape the SSE stream mints
/// for replay). `sig = None` models an unsigned block (must be dropped on replay).
fn thinking_detail(text: &str, sig: Option<&str>) -> ReasoningDetail {
    ReasoningDetail {
        kind: Some("thinking".to_string()),
        text: Some(text.to_string()),
        signature: sig.map(|s| s.to_string()),
        ..Default::default()
    }
}

#[test]
fn system_role_becomes_claude_code_head_plus_content() {
    let (system, msgs) = build_messages(
        vec![
            ChatMessage::new(Role::System, "PROJECT RULES"),
            ChatMessage::new(Role::User, "hi"),
        ],
        None,
    );
    // Head is always the Claude Code identity; the koma system content follows.
    assert_eq!(system[0].text, CLAUDE_CODE_SYSTEM);
    assert_eq!(system[1].text, "PROJECT RULES");
    // System never leaks into messages; only the user turn is present.
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(
        msgs[0].content,
        vec![Block::Text {
            text: "hi".to_string()
        }]
    );
}

#[test]
fn cache_split_mark_is_stripped_from_system() {
    let sys = format!("HEAD{}TAIL", crate::dto::chat::CACHE_SPLIT_MARK);
    let (system, _msgs) = build_messages(
        vec![
            ChatMessage::new(Role::System, sys),
            ChatMessage::new(Role::User, "hi"),
        ],
        None,
    );
    // The boundary marker is removed; head + tail concatenate into one block.
    assert_eq!(system[1].text, "HEADTAIL");
}

#[test]
fn assistant_text_precedes_tool_use_and_input_is_object() {
    let asst = ChatMessage::assistant_with_tools(
        "let me look".to_string(),
        vec![tool_call("call_1", "read", r#"{"path":"a"}"#)],
    );
    let (_system, msgs) = build_messages(
        vec![
            ChatMessage::new(Role::System, "sys"),
            ChatMessage::new(Role::User, "go"),
            asst,
        ],
        None,
    );
    // msgs[0] = user "go", msgs[1] = assistant.
    assert_eq!(msgs[1].role, "assistant");
    assert_eq!(
        msgs[1].content[0],
        Block::Text {
            text: "let me look".to_string()
        }
    );
    match &msgs[1].content[1] {
        Block::ToolUse { id, name, input } => {
            assert_eq!(id, "call_1");
            assert_eq!(name, "read");
            // Arguments stored as a STRING are parsed to a JSON OBJECT.
            assert_eq!(input, &serde_json::json!({"path": "a"}));
        }
        other => panic!("expected tool_use, got {other:?}"),
    }
}

#[test]
fn parallel_tool_results_coalesce_into_one_user_message() {
    let asst = ChatMessage::assistant_with_tools(
        String::new(),
        vec![
            tool_call("c1", "read", "{}"),
            tool_call("c2", "grep", "{}"),
        ],
    );
    let (_system, msgs) = build_messages(
        vec![
            ChatMessage::new(Role::System, "sys"),
            ChatMessage::new(Role::User, "go"),
            asst,
            ChatMessage::tool_result("c1".to_string(), "body one".to_string()),
            ChatMessage::tool_result("c2".to_string(), "body two".to_string()),
        ],
        None,
    );
    // user "go", assistant (2 tool_use), ONE user turn with 2 tool_result blocks.
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2].role, "user");
    assert_eq!(msgs[2].content.len(), 2, "two tool results must coalesce");
    assert!(matches!(
        &msgs[2].content[0],
        Block::ToolResult { tool_use_id, .. } if tool_use_id == "c1"
    ));
    assert!(matches!(
        &msgs[2].content[1],
        Block::ToolResult { tool_use_id, .. } if tool_use_id == "c2"
    ));
}

#[test]
fn tool_result_then_user_text_merge_into_one_user_turn() {
    let asst = ChatMessage::assistant_with_tools(
        String::new(),
        vec![tool_call("c1", "read", "{}")],
    );
    let (_system, msgs) = build_messages(
        vec![
            ChatMessage::new(Role::System, "sys"),
            ChatMessage::new(Role::User, "go"),
            asst,
            ChatMessage::tool_result("c1".to_string(), "result".to_string()),
            ChatMessage::new(Role::User, "and now this"),
        ],
        None,
    );
    // user "go", assistant, then ONE user turn = [tool_result, text].
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2].role, "user");
    assert_eq!(msgs[2].content.len(), 2);
    assert!(matches!(&msgs[2].content[0], Block::ToolResult { .. }));
    assert_eq!(
        msgs[2].content[1],
        Block::Text {
            text: "and now this".to_string()
        }
    );
}

#[test]
fn empty_history_gets_user_placeholder() {
    let (_system, msgs) = build_messages(vec![], None);
    assert_eq!(
        msgs,
        vec![Message {
            role: "user",
            content: vec![Block::Text {
                text: "...".to_string()
            }],
        }]
    );
}

#[test]
fn user_marks_are_stripped() {
    let marked = format!("{}$ ls\nfile.txt", crate::dto::chat::SHELL_MARK);
    let (_system, msgs) = build_messages(vec![ChatMessage::new(Role::User, marked)], None);
    assert_eq!(
        msgs[0].content,
        vec![Block::Text {
            text: "$ ls\nfile.txt".to_string()
        }]
    );
}

#[test]
fn tool_input_edge_cases() {
    // A non-object argument string collapses to an empty object.
    assert_eq!(tool_use_input("\"just a string\""), serde_json::json!({}));
    assert_eq!(tool_use_input(""), serde_json::json!({}));
    // Duplicate-fragment args are repaired then parsed.
    assert_eq!(
        tool_use_input(r#"{"a":1}{"a":1}"#),
        serde_json::json!({"a": 1})
    );
}

#[test]
fn normalize_schema_defaults_empty_to_object() {
    assert_eq!(
        normalize_schema(Value::Null),
        serde_json::json!({"type": "object", "properties": {}})
    );
    assert_eq!(
        normalize_schema(serde_json::json!({})),
        serde_json::json!({"type": "object", "properties": {}})
    );
    // A real schema is passed through untouched.
    let real = serde_json::json!({"type": "object", "properties": {"x": {"type": "string"}}});
    assert_eq!(normalize_schema(real.clone()), real);
}

#[test]
fn image_source_parses_media_type_and_data() {
    let src = image_source_from_data_url("data:image/png;base64,QUJD").unwrap();
    assert_eq!(src.kind, "base64");
    assert_eq!(src.media_type, "image/png");
    assert_eq!(src.data, "QUJD");
    // Malformed URLs yield None.
    assert!(image_source_from_data_url("not-a-data-url").is_none());
    assert!(image_source_from_data_url("data:image/png;base64,").is_none());
}

#[test]
fn block_serializes_with_type_tag() {
    let v = serde_json::to_value(Block::ToolResult {
        tool_use_id: "c1".to_string(),
        content: "out".to_string(),
        is_error: None,
    })
    .unwrap();
    assert_eq!(v["type"], "tool_result");
    assert_eq!(v["tool_use_id"], "c1");
    assert_eq!(v["content"], "out");
    // is_error omitted when None.
    assert!(v.get("is_error").is_none());
}

// ----- extended thinking -----

#[test]
fn thinking_params_default_is_adaptive_no_effort() {
    let adaptive = serde_json::json!({"type": "adaptive", "display": "summarized"});
    // Empty, "default", whitespace, and any unknown token all mean "adaptive, no
    // explicit effort" (thinking on, context_management always None, output_config
    // omitted).
    for token in ["", "default", "  ", "weird-token"] {
        let (thinking, cm, oc) = thinking_params(token, false);
        assert_eq!(thinking, Some(adaptive.clone()), "token {token:?}");
        assert_eq!(cm, None, "token {token:?}");
        assert_eq!(oc, None, "token {token:?}");
    }
}

#[test]
fn thinking_params_off_sends_nothing() {
    // Adaptive can't be disabled and effort can't be pinned without an unsent
    // beta, so "off"/"none" suppress all three, same as forced tool_choice.
    for token in ["off", "none"] {
        assert_eq!(
            thinking_params(token, false),
            (None, None, None),
            "token {token:?}"
        );
    }
}

#[test]
fn thinking_params_effort_levels_echo_verbatim() {
    let adaptive = serde_json::json!({"type": "adaptive", "display": "summarized"});
    for lvl in ["low", "medium", "high", "xhigh", "max"] {
        let (thinking, cm, oc) = thinking_params(lvl, false);
        assert_eq!(thinking, Some(adaptive.clone()), "level {lvl}");
        assert_eq!(cm, None, "level {lvl} has no context_management");
        assert_eq!(oc, Some(serde_json::json!({"effort": lvl})), "level {lvl}");
    }
}

#[test]
fn thinking_params_forced_tool_choice_sends_nothing() {
    // A forced tool_choice deletes thinking; suppress all three regardless of effort.
    for token in ["", "high", "off", "max"] {
        assert_eq!(
            thinking_params(token, true),
            (None, None, None),
            "token {token:?}"
        );
    }
}

#[test]
fn assistant_blocks_replays_signed_thinking_before_text_and_tool_use() {
    let msg = ChatMessage::assistant_with_tools(
        "answer text".to_string(),
        vec![tool_call("c1", "read", "{}")],
    )
    .with_reasoning_details(Some(vec![thinking_detail("deep thought", Some("sig-abc"))]));
    let blocks = assistant_blocks(&msg);
    // Ordering: thinking → text → tool_use.
    assert_eq!(
        blocks[0],
        Block::Thinking {
            thinking: "deep thought".to_string(),
            signature: "sig-abc".to_string(),
        }
    );
    assert_eq!(
        blocks[1],
        Block::Text {
            text: "answer text".to_string()
        }
    );
    assert!(matches!(blocks[2], Block::ToolUse { .. }));
}

#[test]
fn assistant_blocks_drops_unsigned_thinking() {
    // Real Anthropic requires the signature; an unsigned thinking detail is dropped
    // (never sent as `signature: ""`). Only the tool_use survives (content empty).
    let msg = ChatMessage::assistant_with_tools(
        String::new(),
        vec![tool_call("c1", "read", "{}")],
    )
    .with_reasoning_details(Some(vec![thinking_detail("unsigned", None)]));
    let blocks = assistant_blocks(&msg);
    assert!(!blocks.iter().any(|b| matches!(b, Block::Thinking { .. })));
    assert!(matches!(blocks[0], Block::ToolUse { .. }));
}

#[test]
fn assistant_blocks_replays_redacted_thinking_first() {
    let detail = ReasoningDetail {
        kind: Some("redacted_thinking".to_string()),
        data: Some("ENCRYPTED".to_string()),
        ..Default::default()
    };
    let msg = ChatMessage::assistant_with_tools(
        "hi".to_string(),
        vec![tool_call("c1", "read", "{}")],
    )
    .with_reasoning_details(Some(vec![detail]));
    let blocks = assistant_blocks(&msg);
    assert_eq!(
        blocks[0],
        Block::RedactedThinking {
            data: "ENCRYPTED".to_string()
        }
    );
}

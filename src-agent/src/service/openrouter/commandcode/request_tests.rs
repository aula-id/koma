#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Unit tests for Command Code request mapping.

use super::*;
use crate::dto::chat::{ChatMessage, FunctionCall, Role, ToolCall};

fn user(content: &str) -> ChatMessage {
    ChatMessage::new(Role::User, content)
}

fn assistant(content: &str) -> ChatMessage {
    ChatMessage::new(Role::Assistant, content)
}

fn assistant_with_tools(content: &str, calls: Vec<ToolCall>) -> ChatMessage {
    ChatMessage::assistant_with_tools(content.to_string(), calls)
}

fn tc(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        kind: "function".to_string(),
        function: FunctionCall {
            name: name.to_string(),
            arguments: args.to_string(),
        },
    }
}

fn tool_result(id: &str, content: &str) -> ChatMessage {
    ChatMessage::tool_result(id.to_string(), content.to_string())
}

#[test]
fn system_extracted_to_params_not_messages() {
    let msgs = vec![
        ChatMessage::new(Role::System, "You are helpful."),
        user("Hello"),
    ];
    let system = extract_system(&msgs);
    assert_eq!(system, "You are helpful.");
    let cc = build_messages(&msgs, None);
    assert_eq!(cc.len(), 1);
    assert_eq!(cc[0]["role"], "user");
}

#[test]
fn system_concatenation() {
    let msgs = vec![
        ChatMessage::new(Role::System, "Part 1"),
        user("hi"),
        ChatMessage::new(Role::System, "Part 2"),
    ];
    let system = extract_system(&msgs);
    assert_eq!(system, "Part 1\n\nPart 2");
}

#[test]
fn system_cache_split_mark_stripped() {
    let sys = format!("CACHED{}VOLATILE", crate::dto::chat::CACHE_SPLIT_MARK);
    let msgs = vec![ChatMessage::new(Role::System, sys)];
    assert_eq!(extract_system(&msgs), "CACHEDVOLATILE");
}

#[test]
fn user_text_content() {
    let cc = build_messages(&[user("Hello world")], None);
    assert_eq!(cc.len(), 1);
    assert_eq!(cc[0]["role"], "user");
    assert_eq!(cc[0]["content"], "Hello world");
}

#[test]
fn shell_mark_stripped_from_user() {
    let marked = format!("{}$ ls\nfile.txt", crate::dto::chat::SHELL_MARK);
    let cc = build_messages(&[user(&marked)], None);
    assert_eq!(cc[0]["content"], "$ ls\nfile.txt");
}

#[test]
fn assistant_text_only() {
    let cc = build_messages(&[assistant("Sure thing!")], None);
    assert_eq!(cc.len(), 1);
    assert_eq!(cc[0]["role"], "assistant");
    let blocks = cc[0]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "Sure thing!");
}

#[test]
fn assistant_with_paired_tool_call() {
    let msgs = vec![
        user("read file"),
        assistant_with_tools("let me check", vec![tc("c1", "read", r#"{"path":"x"}"#)]),
        tool_result("c1", "file contents"),
    ];
    let cc = build_messages(&msgs, None);
    assert_eq!(cc.len(), 3);
    assert_eq!(cc[0]["role"], "user");
    assert_eq!(cc[1]["role"], "assistant");
    assert_eq!(cc[2]["role"], "tool");
    let blocks = cc[1]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[1]["type"], "tool-call");
    assert_eq!(blocks[1]["toolCallId"], "c1");
    assert_eq!(blocks[1]["toolName"], "read");
    let tool_blocks = cc[2]["content"].as_array().unwrap();
    assert_eq!(tool_blocks.len(), 1);
    assert_eq!(tool_blocks[0]["type"], "tool-result");
    assert_eq!(tool_blocks[0]["toolCallId"], "c1");
}

#[test]
fn orphan_tool_call_dropped() {
    let msgs = vec![
        user("do something"),
        assistant_with_tools("", vec![tc("orphan", "read", "{}")]),
    ];
    let cc = build_messages(&msgs, None);
    // Orphan tool-call is dropped; empty assistant content means the whole
    // assistant message is omitted (only the user message remains).
    assert_eq!(cc.len(), 1);
    assert_eq!(cc[0]["role"], "user");
}

#[test]
fn empty_user_skipped() {
    assert!(build_messages(&[user("")], None).is_empty());
}

#[test]
fn tools_flatten() {
    let tools = flatten_tools(&["read".to_string(), "write".to_string()], &[]);
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["name"], "read");
    assert_eq!(tools[0]["input_schema"]["type"], "object");
}

#[test]
fn normalize_empty_schema() {
    assert_eq!(
        normalize_schema(serde_json::json!({})),
        serde_json::json!({"type": "object", "properties": {}})
    );
    assert_eq!(
        normalize_schema(serde_json::json!(null)),
        serde_json::json!({"type": "object", "properties": {}})
    );
}

#[test]
fn normalize_nonempty_preserved() {
    let s = serde_json::json!({"type": "object", "properties": {"x": {"type": "string"}}});
    assert_eq!(normalize_schema(s.clone()), s);
}

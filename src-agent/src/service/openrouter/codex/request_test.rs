#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::dto::chat::{ChatMessage, ReasoningDetail, Role, ToolCall};

fn assistant_with_tools(content: &str, calls: Vec<ToolCall>) -> ChatMessage {
    ChatMessage::assistant_with_tools(content.to_string(), calls)
}

fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        kind: "function".to_string(),
        function: crate::dto::chat::FunctionCall {
            name: name.to_string(),
            arguments: args.to_string(),
        },
    }
}

#[test]
fn system_split_head_to_instructions_tail_to_developer() {
    let sys = format!(
        "BASE PROMPT{}VOLATILE TAIL",
        crate::dto::chat::CACHE_SPLIT_MARK
    );
    let (instructions, input) = build_input(
        vec![
            ChatMessage::new(Role::System, sys),
            ChatMessage::new(Role::User, "hi"),
        ],
        None,
    );
    assert_eq!(instructions, "BASE PROMPT");
    // First input item is the developer tail, then the user message.
    assert_eq!(
        input[0],
        InputItem::Message {
            role: "developer",
            content: vec![ContentItem::InputText {
                text: "VOLATILE TAIL".to_string()
            }],
        }
    );
    assert_eq!(
        input[1],
        InputItem::Message {
            role: "user",
            content: vec![ContentItem::InputText {
                text: "hi".to_string()
            }],
        }
    );
}

#[test]
fn system_no_mark_all_to_instructions_no_developer() {
    let (instructions, input) = build_input(
        vec![
            ChatMessage::new(Role::System, "WHOLE SYSTEM"),
            ChatMessage::new(Role::User, "hi"),
        ],
        None,
    );
    assert_eq!(instructions, "WHOLE SYSTEM");
    // No developer message — the user message is first.
    assert_eq!(input.len(), 1);
    assert!(matches!(&input[0], InputItem::Message { role: "user", .. }));
}

#[test]
fn tool_call_ordering_reasoning_then_message_then_function_calls() {
    let mut msg = assistant_with_tools(
        "let me look",
        vec![tool_call("call_1", "read", "{\"path\":\"a\"}")],
    );
    msg.reasoning_details = Some(vec![ReasoningDetail {
        kind: None,
        text: None,
        summary: None,
        data: Some("ENCRYPTED_BLOB".to_string()),
        signature: None,
        id: Some("rs_1".to_string()),
        format: Some("codex_encrypted".to_string()),
        index: None,
        extra: serde_json::Map::new(),
    }]);
    let (_instr, input) = build_input(
        vec![
            ChatMessage::new(Role::System, "sys"),
            msg,
            ChatMessage::tool_result("call_1".to_string(), "file body".to_string()),
        ],
        None,
    );
    // Order: reasoning, assistant message, function_call, function_call_output.
    assert_eq!(
        input[0],
        InputItem::Reasoning {
            encrypted_content: "ENCRYPTED_BLOB".to_string(),
            summary: Vec::new(),
        }
    );
    assert_eq!(
        input[1],
        InputItem::Message {
            role: "assistant",
            content: vec![ContentItem::OutputText {
                text: "let me look".to_string()
            }],
        }
    );
    assert_eq!(
        input[2],
        InputItem::FunctionCall {
            name: "read".to_string(),
            arguments: "{\"path\":\"a\"}".to_string(),
            call_id: "call_1".to_string(),
        }
    );
    assert_eq!(
        input[3],
        InputItem::FunctionCallOutput {
            call_id: "call_1".to_string(),
            output: "file body".to_string(),
        }
    );
}

#[test]
fn non_codex_reasoning_details_dropped() {
    let mut msg = assistant_with_tools("x", vec![tool_call("c1", "read", "{}")]);
    // An OpenRouter-format reasoning detail (not codex_encrypted) must be dropped.
    msg.reasoning_details = Some(vec![ReasoningDetail {
        kind: Some("reasoning.text".to_string()),
        text: Some("thinking".to_string()),
        summary: None,
        data: None,
        signature: Some("sig".to_string()),
        id: None,
        format: None,
        index: None,
        extra: serde_json::Map::new(),
    }]);
    let (_instr, input) = build_input(vec![msg], None);
    // No Reasoning item — the first item is the assistant message.
    assert!(!input
        .iter()
        .any(|i| matches!(i, InputItem::Reasoning { .. })));
    assert!(matches!(
        &input[0],
        InputItem::Message {
            role: "assistant",
            ..
        }
    ));
}

#[test]
fn user_marks_are_stripped() {
    let marked = format!("{}$ ls\nfile.txt", crate::dto::chat::SHELL_MARK);
    let (_instr, input) = build_input(vec![ChatMessage::new(Role::User, marked)], None);
    assert_eq!(
        input[0],
        InputItem::Message {
            role: "user",
            content: vec![ContentItem::InputText {
                text: "$ ls\nfile.txt".to_string()
            }],
        }
    );
}

#[test]
fn empty_input_gets_placeholder() {
    // A history that produces no input items (empty assistant, no system tail).
    let (_instr, input) = build_input(vec![ChatMessage::new(Role::Assistant, "")], None);
    assert_eq!(
        input,
        vec![InputItem::Message {
            role: "user",
            content: vec![ContentItem::InputText {
                text: "...".to_string()
            }],
        }]
    );
}

#[test]
fn effort_map() {
    assert_eq!(codex_effort(""), ("medium".to_string(), true));
    assert_eq!(codex_effort("default"), ("medium".to_string(), true));
    assert_eq!(codex_effort("off"), ("none".to_string(), false));
    assert_eq!(codex_effort("none"), ("none".to_string(), false));
    assert_eq!(codex_effort("minimal"), ("low".to_string(), true));
    assert_eq!(codex_effort("low"), ("low".to_string(), true));
    assert_eq!(codex_effort("high"), ("high".to_string(), true));
    assert_eq!(codex_effort("xhigh"), ("xhigh".to_string(), true));
    assert_eq!(codex_effort("bogus"), ("medium".to_string(), true));
}

#[test]
fn tool_name_truncation() {
    let long = "x".repeat(200);
    let truncated = truncate_name(&long);
    assert_eq!(truncated.chars().count(), 128);
    // A short name is untouched.
    assert_eq!(truncate_name("read"), "read");
}

#[test]
fn to_text_format_is_flattened_shape() {
    let schema = serde_json::json!({"type": "object"});
    let tf = to_text_format("verdict", schema);
    assert_eq!(tf["format"]["type"], "json_schema");
    assert_eq!(tf["format"]["name"], "verdict");
    assert_eq!(tf["format"]["strict"], true);
    assert_eq!(tf["format"]["schema"]["type"], "object");
    // NOT the chat-completions nesting.
    assert!(tf.get("json_schema").is_none());
    // No model → no verbosity (legacy signature).
    assert!(tf.get("verbosity").is_none());
}

#[test]
fn freeform_text_verbosity_gate() {
    let t = freeform_text("gpt-5.5").unwrap();
    assert_eq!(t["verbosity"], "low");
    assert!(t.get("format").is_none());
    assert!(freeform_text("gpt-5.3-codex-spark").is_none());
    assert!(freeform_text("gpt-5-chat-latest").is_none());
    assert!(freeform_text("gpt-4o").is_none());
}

#[test]
fn to_text_format_for_merges_verbosity_with_schema() {
    let schema = serde_json::json!({"type": "object"});
    let tf = to_text_format_for("gpt-5.4", "verdict", schema.clone());
    assert_eq!(tf["format"]["name"], "verdict");
    assert_eq!(tf["verbosity"], "low");
    // codex id skips verbosity
    let no = to_text_format_for("gpt-5.3-codex-spark", "verdict", schema);
    assert!(no.get("verbosity").is_none());
    assert_eq!(no["format"]["type"], "json_schema");
}

#[test]
fn responses_request_serializes_verbosity() {
    let body = ResponsesRequest {
        model: "gpt-5.5".into(),
        instructions: "sys".into(),
        input: vec![],
        tools: None,
        tool_choice: "auto",
        stream: true,
        store: false,
        reasoning: Some(ResponsesReasoning {
            effort: "medium".into(),
            summary: "auto",
        }),
        include: vec!["reasoning.encrypted_content"],
        prompt_cache_key: "sid".into(),
        text: freeform_text("gpt-5.5"),
    };
    let v = serde_json::to_value(&body).unwrap();
    assert_eq!(v["text"]["verbosity"], "low");
    assert_eq!(v["store"], false);
    // Codex backend rejects max_output_tokens — never send either budget field.
    assert!(v.get("max_tokens").is_none());
    assert!(v.get("max_output_tokens").is_none());
}

#[test]
fn input_item_serializes_with_type_tag() {
    let item = InputItem::FunctionCallOutput {
        call_id: "c1".to_string(),
        output: "out".to_string(),
    };
    let v = serde_json::to_value(&item).unwrap();
    assert_eq!(v["type"], "function_call_output");
    assert_eq!(v["call_id"], "c1");
    assert_eq!(v["output"], "out");
    // No stray `id` field ever.
    assert!(v.get("id").is_none());
}

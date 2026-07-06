//! Pure request-mapping layer for the OpenAI Responses API ("Codex").
//!
//! Turns koma's internal [`ChatMessage`](crate::dto::chat::ChatMessage) history +
//! tool set into the `/responses` POST body shape and back-maps the reasoning /
//! effort knobs. Everything here is a pure function of its inputs (no `self`, no
//! network), so it is exhaustively unit-tested at the bottom of the file.
//!
//! ## Wire-shape gotchas (learned the hard way, do not regress)
//!
//! - Input items NEVER carry an `id` field: under `store: false` the backend
//!   404s on server-minted ids we'd echo back.
//! - Reasoning is replayed as `{"type":"reasoning","encrypted_content":<blob>,
//!   "summary":[]}` — the encrypted blob captured from a prior turn, NOT the
//!   human-readable summary.
//! - `text.format` is the FLATTENED Responses structured-output shape
//!   (`{"format":{"type":"json_schema","name":…,"schema":…}}`), not the
//!   chat-completions `response_format` nesting.

use serde::Serialize;

use crate::dto::chat::{ChatMessage, Role};
use crate::dto::openrouter::{ImageWireCtx, ToolDef};

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

/// Top-level `POST /responses` body. `store: false` (stateless), `stream: true`
/// (we always read the SSE wire, even for the one-shot collect path).
#[derive(Debug, Serialize)]
pub(super) struct ResponsesRequest {
    pub model: String,
    /// The stable system/developer instructions (Codex's equivalent of the
    /// system message head).
    pub instructions: String,
    pub input: Vec<InputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<FlatToolDef>>,
    pub tool_choice: &'static str,
    pub stream: bool,
    pub store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponsesReasoning>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<&'static str>,
    pub prompt_cache_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<serde_json::Value>,
}

/// Reasoning directive. Codex REQUIRES this object on every request; the effort
/// token maps through [`codex_effort`] and `summary` is always `"auto"`.
#[derive(Debug, Serialize)]
pub(super) struct ResponsesReasoning {
    pub effort: String,
    pub summary: &'static str,
}

/// One function tool, in the Responses API's FLAT shape (name/description/params
/// hoisted to the top level, unlike chat-completions' nested `function` object).
///
/// `strict: false` — VI-3: if the backend 400s on this field, drop it (some
/// gateways reject `strict` on function tools).
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct FlatToolDef {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub strict: bool,
}

/// One item in the `input[]` array. Internally tagged on `type`. NEVER carries an
/// `id` (server ids 404 under `store: false`).
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
pub(super) enum InputItem {
    #[serde(rename = "message")]
    Message {
        role: &'static str,
        content: Vec<ContentItem>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        name: String,
        arguments: String,
        call_id: String,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput { call_id: String, output: String },
    #[serde(rename = "reasoning")]
    Reasoning {
        encrypted_content: String,
        /// Always `[]` on replay — the human summary is display-only and never
        /// re-sent; only the encrypted blob preserves continuity.
        summary: Vec<serde_json::Value>,
    },
}

/// One content part of a [`InputItem::Message`]. Internally tagged on `type`.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
pub(super) enum ContentItem {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "output_text")]
    OutputText { text: String },
    #[serde(rename = "input_image")]
    InputImage { image_url: String },
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

/// Split a System message into `(instructions, developer_tail)`.
///
/// Policy chokepoint (VI-1): today the STABLE cached head (before
/// [`CACHE_SPLIT_MARK`](crate::dto::chat::CACHE_SPLIT_MARK)) becomes the
/// `instructions` and the VOLATILE tail (project files + awareness) becomes a
/// `developer` message. If the backend rejects arbitrary instructions, flip this
/// to return `("<constant instructions>".into(), whole_content)` so the entire
/// system prompt rides as a developer message instead — the rest of
/// [`build_input`] is unaffected.
fn split_system(content: &str) -> (String, String) {
    match content.split_once(crate::dto::chat::CACHE_SPLIT_MARK) {
        Some((head, tail)) => (head.to_string(), tail.to_string()),
        None => (content.to_string(), String::new()),
    }
}

/// Strip a leading `SHELL_MARK` / `BASH_NUDGE_MARK` transcript-render marker so
/// the model reads the clean text — mirrors the chat-completions wire builder.
fn strip_marks(content: &str) -> String {
    content
        .strip_prefix(crate::dto::chat::SHELL_MARK)
        .or_else(|| content.strip_prefix(crate::dto::chat::BASH_NUDGE_MARK))
        .unwrap_or(content)
        .to_string()
}

/// Map a conversation history into the Responses `(instructions, input[])` pair.
///
/// Mirrors the semantics of `to_wire_with_images` (the chat-completions builder)
/// but targets the Responses shape:
/// - System head → `instructions`; its volatile tail → a leading `developer`
///   message. Extra System messages (defensive) → more `developer` messages.
/// - User → `user` message (`input_text`, marks stripped) + one `input_image`
///   per surviving attachment (gated on `image_ctx.model_takes_images`).
/// - Assistant with tool calls → encrypted-reasoning items, then the assistant
///   text (if any), then one `function_call` per tool call — in that order.
/// - Assistant plain → `assistant` message (`output_text`).
/// - Tool result → `function_call_output`.
///
/// An empty result gets one placeholder user message (the backend rejects empty
/// input).
pub(super) fn build_input(
    messages: Vec<ChatMessage>,
    image_ctx: Option<&ImageWireCtx>,
) -> (String, Vec<InputItem>) {
    let mut instructions = String::new();
    let mut input: Vec<InputItem> = Vec::new();
    let mut seen_system = false;

    for m in messages {
        match m.role {
            Role::System => {
                if !seen_system {
                    seen_system = true;
                    let (head, tail) = split_system(&m.content);
                    instructions = head;
                    // The volatile tail becomes the FIRST input item (a developer
                    // message). System is history[0], so pushing here keeps it first.
                    if !tail.trim().is_empty() {
                        input.push(InputItem::Message {
                            role: "developer",
                            content: vec![ContentItem::InputText { text: tail }],
                        });
                    }
                } else if !m.content.trim().is_empty() {
                    // Defensive: additional System messages → developer messages.
                    input.push(InputItem::Message {
                        role: "developer",
                        content: vec![ContentItem::InputText { text: m.content }],
                    });
                }
            }
            Role::User => {
                if m.attachments.is_empty() {
                    let text = strip_marks(&m.content);
                    // Wire-copy reasoning-tag escape (user content is DATA): keep a
                    // literal `<think>` out of the delimiter path. Storage keeps the
                    // real tag; mirrors `strip_marks` as a wire-only transform. The
                    // System/developer branch above is intentionally NOT escaped.
                    let text = crate::dto::chat::escape_reasoning_tags(&text).into_owned();
                    input.push(InputItem::Message {
                        role: "user",
                        content: vec![ContentItem::InputText { text }],
                    });
                } else {
                    // Keep the typed text (with its `[Image #N]` markers) as the
                    // first part; the marker stays visible even if a part is
                    // stripped. Marks are never present on attachment-bearing turns.
                    let mut content = vec![ContentItem::InputText {
                        // Wire-copy reasoning-tag escape (user content is DATA).
                        text: crate::dto::chat::escape_reasoning_tags(&m.content).into_owned(),
                    }];
                    // Gate image parts on the resolved model's capability, exactly
                    // like `attachment_parts`; an unreadable file is skipped.
                    let capable = image_ctx.map(|c| c.model_takes_images).unwrap_or(false);
                    if capable {
                        let ctx = image_ctx.expect("capable implies Some");
                        for att in &m.attachments {
                            if let Some(url) =
                                crate::dto::openrouter::request::data_url_for(&ctx.session_dir, att)
                            {
                                content.push(ContentItem::InputImage { image_url: url });
                            }
                        }
                    }
                    input.push(InputItem::Message {
                        role: "user",
                        content,
                    });
                }
            }
            Role::Assistant => {
                match m.tool_calls {
                    Some(ref calls) if !calls.is_empty() => {
                        // (1) Encrypted reasoning items first, so the model's signed
                        // chain-of-thought precedes the tool calls it produced.
                        if let Some(details) = &m.reasoning_details {
                            for d in details {
                                if d.format.as_deref() == Some("codex_encrypted") {
                                    if let Some(blob) = &d.data {
                                        input.push(InputItem::Reasoning {
                                            encrypted_content: blob.clone(),
                                            summary: Vec::new(),
                                        });
                                    }
                                }
                            }
                        }
                        // (2) Assistant text, if any.
                        if !m.content.is_empty() {
                            input.push(InputItem::Message {
                                role: "assistant",
                                content: vec![ContentItem::OutputText {
                                    // Wire-copy reasoning-tag escape (assistant text
                                    // replayed from history is DATA). Storage keeps
                                    // the real tag (decoded before persist).
                                    text: crate::dto::chat::escape_reasoning_tags(&m.content)
                                        .into_owned(),
                                }],
                            });
                        }
                        // (3) One function_call per tool call, arguments repaired.
                        for tc in calls {
                            input.push(InputItem::FunctionCall {
                                name: tc.function.name.clone(),
                                arguments: crate::dto::chat::sanitize_tool_arguments(
                                    &tc.function.arguments,
                                ),
                                call_id: tc.id.clone(),
                            });
                        }
                    }
                    // Plain assistant turn (skip when empty).
                    _ => {
                        if !m.content.is_empty() {
                            input.push(InputItem::Message {
                                role: "assistant",
                                // Wire-copy reasoning-tag escape (assistant text is DATA).
                                content: vec![ContentItem::OutputText {
                                    text: crate::dto::chat::escape_reasoning_tags(&m.content)
                                        .into_owned(),
                                }],
                            });
                        }
                    }
                }
            }
            Role::Tool => {
                input.push(InputItem::FunctionCallOutput {
                    call_id: m.tool_call_id.unwrap_or_default(),
                    // Wire-copy reasoning-tag escape (tool output is DATA — this is
                    // exactly the git-log-of-commit-messages case). Storage keeps the
                    // real tag verbatim.
                    output: crate::dto::chat::escape_reasoning_tags(&m.content).into_owned(),
                });
            }
        }
    }

    // The backend rejects an empty input array.
    if input.is_empty() {
        input.push(InputItem::Message {
            role: "user",
            content: vec![ContentItem::InputText {
                text: "...".to_string(),
            }],
        });
    }

    (instructions, input)
}

/// Flatten the advertised built-in tools (`all_tools()` ∩ `advertise`) plus the
/// caller-supplied MCP tool defs into Responses [`FlatToolDef`]s. Same filter as
/// the chat-completions stream builder; MCP defs are appended verbatim.
pub(super) fn flatten_tools(advertise: &[String], mcp_tools: &[ToolDef]) -> Vec<FlatToolDef> {
    let mut out: Vec<FlatToolDef> = crate::tool::all_tools()
        .iter()
        .filter(|t| advertise.iter().any(|n| n == t.name()))
        .map(|t| FlatToolDef {
            kind: "function",
            name: truncate_name(t.name()),
            description: t.description().to_string(),
            parameters: t.parameters(),
            strict: false,
        })
        .collect();
    for md in mcp_tools {
        out.push(FlatToolDef {
            kind: "function",
            name: truncate_name(&md.function.name),
            description: md.function.description.clone(),
            parameters: md.function.parameters.clone(),
            strict: false,
        });
    }
    out
}

/// Truncate a tool name to the Responses API's 128-character limit (by chars, so
/// a multi-byte name never splits mid-codepoint).
fn truncate_name(name: &str) -> String {
    name.chars().take(128).collect()
}

/// Map a stored effort token to `(responses_effort, include_encrypted_reasoning)`.
///
/// - `""` / `"default"` → `("medium", true)`
/// - `"off"` / `"none"` → `("none", false)` (drop the encrypted-reasoning include)
/// - `"minimal"` → `("low", true)` (Responses has no "minimal")
/// - `"low"` / `"medium"` / `"high"` / `"xhigh"` → passthrough, `true`
/// - anything else → `("medium", true)`
pub(super) fn codex_effort(effort: &str) -> (String, bool) {
    match effort.trim() {
        "" | "default" => ("medium".to_string(), true),
        "off" | "none" => ("none".to_string(), false),
        "minimal" => ("low".to_string(), true),
        e @ ("low" | "medium" | "high" | "xhigh") => (e.to_string(), true),
        _ => ("medium".to_string(), true),
    }
}

/// Build the Responses structured-output directive (`text` field). This is the
/// FLATTENED shape `{"format":{"type":"json_schema","name":…,"strict":true,
/// "schema":…}}` — NOT the chat-completions `response_format` nesting.
///
/// VI-2: if the backend 400s on this, fall back to schema-in-prompt.
pub(in crate::service::openrouter) fn to_text_format(
    name: &str,
    schema: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "format": {
            "type": "json_schema",
            "name": name,
            "strict": true,
            "schema": schema,
        }
    })
}

#[cfg(test)]
mod tests {
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
        let sys = format!("BASE PROMPT{}VOLATILE TAIL", crate::dto::chat::CACHE_SPLIT_MARK);
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
        assert!(matches!(
            &input[0],
            InputItem::Message { role: "user", .. }
        ));
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
            InputItem::Message { role: "assistant", .. }
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
}

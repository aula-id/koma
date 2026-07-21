//! Pure request-mapping layer for the Command Code `/alpha/generate` wire.
//!
//! Turns koma's internal [`ChatMessage`](crate::dto::chat::ChatMessage) history +
//! tool set into the `/alpha/generate` POST body shape. Everything here is a pure
//! function of its inputs (no `self`, no network), so it is unit-tested at the
//! bottom of the file.
//!
//! ## Wire-shape gotchas
//!
//! - System messages go to `params.system`, NOT in the `messages` array.
//! - Tool definitions use the Anthropic-ish `{type, name, description, input_schema}`
//!   shape, NOT OpenAI's `{type, function: {name, description, parameters}}`.
//! - Assistant tool calls are sent as content blocks: `{type: "tool-call", toolCallId, toolName, input}`.
//! - Tool results are separate messages: `{role: "tool", content: [{type: "tool-result", ...}]}`.
//! - Orphan tool calls (no matching tool result) are dropped.
//! - `params.max_tokens` is capped at 64,000 (pi default).

use serde::Serialize;
use serde_json::{json, Value};

use crate::dto::chat::{ChatMessage, Role};
use crate::dto::openrouter::{ImageWireCtx, ToolDef};
use crate::dto::openrouter::request::data_url_for;

/// Top-level `POST /alpha/generate` body.
#[derive(Debug, Serialize)]
pub(super) struct GenerateRequest {
    pub config: RequestConfig,
    pub memory: Value,
    pub taste: Value,
    pub skills: Value,
    pub params: RequestParams,
    #[serde(rename = "threadId")]
    pub thread_id: String,
}

/// The `config` block — project metadata.
#[derive(Debug, Serialize)]
pub(super) struct RequestConfig {
    #[serde(rename = "workingDir")]
    pub working_dir: String,
    pub date: String,
    pub environment: &'static str,
    pub structure: Vec<Value>,
    #[serde(rename = "isGitRepo")]
    pub is_git_repo: bool,
    #[serde(rename = "currentBranch")]
    pub current_branch: String,
    #[serde(rename = "mainBranch")]
    pub main_branch: String,
    #[serde(rename = "gitStatus")]
    pub git_status: String,
    #[serde(rename = "recentCommits")]
    pub recent_commits: Vec<Value>,
}

/// The `params` block — model + messages + tools + system.
#[derive(Debug, Serialize)]
pub(super) struct RequestParams {
    pub model: String,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
    pub system: String,
    #[serde(rename = "max_tokens")]
    pub max_tokens: u32,
    pub temperature: f64,
    pub stream: bool,
}

/// Default generate max tokens (matches pi-commandcode-provider).
pub(super) const DEFAULT_GENERATE_MAX_TOKENS: u32 = 64_000;

/// Strip a leading `SHELL_MARK` / `BASH_NUDGE_MARK` / `EXT_PROMPT_MARK`
/// transcript-render marker so the model reads the clean text.
fn strip_marks(content: &str) -> String {
    content
        .strip_prefix(crate::dto::chat::SHELL_MARK)
        .or_else(|| content.strip_prefix(crate::dto::chat::BASH_NUDGE_MARK))
        .or_else(|| content.strip_prefix(crate::dto::chat::EXT_PROMPT_MARK))
        .unwrap_or(content)
        .to_string()
}

/// Compute the set of tool-call IDs that have both a call AND a matching result.
/// Used to drop orphan tool calls (those without results) from the wire.
fn complete_tool_call_ids(messages: &[ChatMessage]) -> std::collections::HashSet<String> {
    let mut call_ids = std::collections::HashSet::new();
    let mut result_ids = std::collections::HashSet::new();

    for m in messages {
        if m.role == Role::Assistant {
            if let Some(calls) = &m.tool_calls {
                for tc in calls {
                    if !tc.id.is_empty() {
                        call_ids.insert(tc.id.clone());
                    }
                }
            }
        } else if m.role == Role::Tool {
            if let Some(id) = &m.tool_call_id {
                if !id.is_empty() {
                    result_ids.insert(id.clone());
                }
            }
        }
    }

    call_ids
        .into_iter()
        .filter(|id| result_ids.contains(id))
        .collect()
}

/// Concatenate all system messages into one string for `params.system`.
/// Strips `CACHE_SPLIT_MARK` boundaries (koma-internal, never rides the wire).
pub(super) fn extract_system(messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    for m in messages {
        if m.role == Role::System {
            let text = m
                .content
                .replace(crate::dto::chat::CACHE_SPLIT_MARK, "");
            if !text.trim().is_empty() {
                parts.push(text);
            }
        }
    }
    parts.join("\n\n")
}

/// Map koma messages into the CC wire format (JSON values).
///
/// - System messages are EXCLUDED from the messages array (they go to `params.system`).
/// - User → `{role: "user", content: string}` or `{role: "user", content: [text, image_url, ...]}` when attachments present.
/// - Assistant with text + tool calls → `{role: "assistant", content: [text, tool-call, ...]}`
/// - Tool result → `{role: "tool", content: [tool-result]}`
/// - Orphan tool calls (no matching tool result) are dropped.
pub(super) fn build_messages(messages: &[ChatMessage], image_ctx: Option<&ImageWireCtx>) -> Vec<Value> {
    let paired = complete_tool_call_ids(messages);
    let mut out = Vec::new();

    for m in messages {
        match m.role {
            Role::System => {} // extracted to params.system
            Role::User => {
                let text = strip_marks(&m.content);
                if text.trim().is_empty() {
                    continue;
                }
                // When attachments are present and the model can read images,
                // emit content as an array of parts (text + image_url blocks).
                let capable = image_ctx.map(|c| c.model_takes_images).unwrap_or(false);
                if capable && !m.attachments.is_empty() {
                    if let Some(ctx) = image_ctx {
                        let mut content_parts: Vec<Value> = vec![json!({
                            "type": "text",
                            "text": text,
                        })];
                        for att in &m.attachments {
                            if let Some(url) = data_url_for(&ctx.session_dir, att) {
                                // Extract mime from the data URL itself —
                                // data_url_for re-sniffs and may downscale
                                // (PNG→JPEG), so att.mime could be stale.
                                let mime = url
                                    .strip_prefix("data:")
                                    .and_then(|s| s.split_once(';'))
                                    .map(|(m, _)| m.to_string())
                                    .unwrap_or_else(|| att.mime.clone());
                                content_parts.push(json!({
                                    "type": "image",
                                    "image": url,
                                    "mediaType": mime,
                                }));
                            }
                        }
                        out.push(json!({
                            "role": "user",
                            "content": content_parts,
                        }));
                    }
                } else {
                    out.push(json!({
                        "role": "user",
                        "content": text,
                    }));
                }
            }
            Role::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": m.content,
                    }));
                }
                if let Some(calls) = &m.tool_calls {
                    for tc in calls {
                        if !paired.contains(&tc.id) {
                            continue;
                        }
                        let input_str =
                            crate::dto::chat::sanitize_tool_arguments(&tc.function.arguments);
                        let input: Value =
                            serde_json::from_str(&input_str).unwrap_or(json!({}));
                        blocks.push(json!({
                            "type": "tool-call",
                            "toolCallId": tc.id,
                            "toolName": tc.function.name,
                            "input": input,
                        }));
                    }
                }
                if !blocks.is_empty() {
                    out.push(json!({
                        "role": "assistant",
                        "content": blocks,
                    }));
                }
            }
            Role::Tool => {
                let call_id = m.tool_call_id.clone().unwrap_or_default();
                if call_id.is_empty() || !paired.contains(&call_id) {
                    continue;
                }
                let tool_name = find_tool_name(messages, &call_id);
                out.push(json!({
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": call_id,
                        "toolName": tool_name,
                        "output": {
                            "type": "text",
                            "value": m.content,
                        }
                    }],
                }));
            }
        }
    }

    out
}

/// Find the tool name for a given call ID from the message history.
fn find_tool_name(messages: &[ChatMessage], call_id: &str) -> String {
    for m in messages {
        if m.role == Role::Assistant {
            if let Some(calls) = &m.tool_calls {
                for tc in calls {
                    if tc.id == call_id {
                        return tc.function.name.clone();
                    }
                }
            }
        }
    }
    String::new()
}

/// Flatten the advertised built-in tools (`all_tools()` ∩ `advertise`) plus the
/// caller-supplied MCP tool defs into CC wire-format tool values.
pub(super) fn flatten_tools(advertise: &[String], mcp_tools: &[ToolDef]) -> Vec<Value> {
    let mut out: Vec<Value> = crate::tool::all_tools()
        .iter()
        .filter(|t| advertise.iter().any(|n| n == t.name()))
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name(),
                "description": t.description(),
                "input_schema": normalize_schema(t.parameters()),
            })
        })
        .collect();
    for md in mcp_tools {
        out.push(json!({
            "type": "function",
            "name": md.function.name,
            "description": md.function.description,
            "input_schema": normalize_schema(md.function.parameters.clone()),
        }));
    }
    out
}

/// Ensure a tool's `input_schema` is a non-empty JSON object.
fn normalize_schema(schema: Value) -> Value {
    match &schema {
        Value::Object(m) if !m.is_empty() => schema,
        _ => json!({"type": "object", "properties": {}}),
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;

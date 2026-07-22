//! Pure request-mapping layer for the native Anthropic Messages API.
//!
//! Turns koma's internal [`ChatMessage`](crate::dto::chat::ChatMessage) history +
//! tool set into the `POST /v1/messages` body shape. Everything here is a pure
//! function of its inputs (no `self`, no network), so it is exhaustively
//! unit-tested at the bottom of the file.
//!
//! ## Wire-shape gotchas (learned from the spec, do not regress)
//!
//! - Anthropic allows ONLY `user` / `assistant` roles and requires STRICT
//!   alternation — two consecutive same-role turns are rejected. System content
//!   rides a separate top-level `system` array (never in `messages`), and
//!   consecutive tool results (parallel tool calls) plus any following user text
//!   must be COALESCED into ONE user message (see [`push_user`]).
//! - Within an assistant turn ALL non-`tool_use` blocks (text) MUST precede any
//!   `tool_use` block ([`assistant_blocks`] partitions them).
//! - `tool_use.input` is a JSON OBJECT; koma stores tool-call arguments as a JSON
//!   STRING (OpenAI style), so it is parsed back to a Value ([`tool_use_input`]).
//! - `tool.input_schema` must be a JSON-Schema object; a missing/empty schema
//!   falls back to `{"type":"object","properties":{}}`.

use serde::Serialize;
use serde_json::Value;

use crate::dto::chat::{ChatMessage, Role};
use crate::dto::openrouter::{ImageWireCtx, ToolDef};

use super::CLAUDE_CODE_SYSTEM;

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

/// Top-level `POST /v1/messages` body. `stream: true` (we always read the SSE
/// wire, even for the one-shot collect path); `max_tokens` is REQUIRED.
#[derive(Debug, Serialize)]
pub(super) struct MessagesRequest {
    pub model: String,
    /// Identity/behaviour blocks. Block 0 is always [`CLAUDE_CODE_SYSTEM`]
    /// (Anthropic rejects OAuth requests whose system prompt isn't Claude Code).
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
    /// Omitted entirely (skip `None`) when the caller advertises no tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicTool>>,
    /// `{"type":"auto"}` for the interactive path; `{"type":"tool","name":…}` for
    /// the forced-tool structured-output path. Omitted when there are no tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    pub max_tokens: u32,
    /// Extended-thinking controls, in canonical order (after `max_tokens`, before
    /// `stream`): `thinking` selects the adaptive mode + summarized display,
    /// `context_management` clears prior thinking from the model's context, and
    /// `output_config` pins the effort. All three are omitted (skip `None`) on the
    /// off / forced-tool_choice paths. Built by [`thinking_params`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_management: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<Value>,
    pub stream: bool,
}

/// One `system` text block.
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct SystemBlock {
    #[serde(rename = "type")]
    pub kind: &'static str, // "text"
    pub text: String,
}

/// One `messages[]` entry. `role` is `"user"` or `"assistant"` only.
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct Message {
    pub role: &'static str,
    pub content: Vec<Block>,
}

/// One content block of a [`Message`]. Internally tagged on `type`.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
pub(super) enum Block {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        /// The tool arguments as a JSON OBJECT (parsed from koma's stored string).
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        /// koma does not structurally track tool-result failure, so this is
        /// always omitted in v1 (absent == success on the Anthropic wire).
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// A REPLAYED extended-thinking block. [`assistant_blocks`] emits it BEFORE any
    /// text/tool_use so an interleaved-thinking tool loop keeps the model's signed
    /// chain-of-thought. `signature` is REQUIRED by real Anthropic — an unsigned
    /// thinking block 400s, so callers drop rather than send an empty signature.
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    /// A REPLAYED redacted (encrypted) thinking block; `data` is echoed verbatim.
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
}

/// A base64 image source for an [`Block::Image`].
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct ImageSource {
    #[serde(rename = "type")]
    pub kind: &'static str, // "base64"
    pub media_type: String,
    pub data: String,
}

/// One function tool in the Anthropic shape (name / description / input_schema).
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

/// Strip a leading `SHELL_MARK` / `BASH_NUDGE_MARK` / `EXT_PROMPT_MARK`
/// transcript-render marker so the model reads the clean text — mirrors the
/// codex/chat-completions builders.
fn strip_marks(content: &str) -> String {
    content
        .strip_prefix(crate::dto::chat::SHELL_MARK)
        .or_else(|| content.strip_prefix(crate::dto::chat::BASH_NUDGE_MARK))
        .or_else(|| content.strip_prefix(crate::dto::chat::EXT_PROMPT_MARK))
        .unwrap_or(content)
        .to_string()
}

/// Map a conversation history into the Anthropic `(system[], messages[])` pair.
///
/// - System → appended to the `system` array (after the fixed Claude Code head);
///   the internal `CACHE_SPLIT_MARK` boundary is stripped (never rides the wire).
/// - User → a `user` message: one text block (marks stripped) + one `image`
///   block per surviving attachment (gated on `image_ctx.model_takes_images`).
/// - Assistant with tool calls → an `assistant` message: the text block (if any)
///   FIRST, then one `tool_use` block per call (arguments parsed to an object).
/// - Assistant plain → an `assistant` message (skipped when empty).
/// - Tool result → a `tool_result` block placed in a `user` message, COALESCED
///   with an adjacent user turn so alternation is never violated.
///
/// The returned `messages` is guaranteed non-empty and to START with a `user`
/// turn (Anthropic requires both); a degenerate history gets a `"..."`
/// placeholder, mirroring codex's empty-input guard.
pub(super) fn build_messages(
    messages: Vec<ChatMessage>,
    image_ctx: Option<&ImageWireCtx>,
) -> (Vec<SystemBlock>, Vec<Message>) {
    let mut system: Vec<SystemBlock> = vec![SystemBlock {
        kind: "text",
        text: CLAUDE_CODE_SYSTEM.to_string(),
    }];
    let mut out: Vec<Message> = Vec::new();

    for m in messages {
        match m.role {
            Role::System => {
                // Flat system array: drop the cache-split boundary marker (it's a
                // koma-internal char) and append the whole content as one block.
                let text = m.content.replace(crate::dto::chat::CACHE_SPLIT_MARK, "");
                if !text.trim().is_empty() {
                    system.push(SystemBlock { kind: "text", text });
                }
            }
            Role::User => {
                let blocks = user_blocks(&m, image_ctx);
                push_user(&mut out, blocks);
            }
            Role::Assistant => {
                let blocks = assistant_blocks(&m);
                if !blocks.is_empty() {
                    out.push(Message {
                        role: "assistant",
                        content: blocks,
                    });
                }
            }
            Role::Tool => {
                let block = Block::ToolResult {
                    tool_use_id: m.tool_call_id.clone().unwrap_or_default(),
                    content: m.content.clone(),
                    is_error: None,
                };
                push_user(&mut out, vec![block]);
            }
        }
    }

    // Anthropic requires a non-empty `messages` array whose FIRST entry is a
    // `user` turn. koma history opens with System (→ system) then a User turn, so
    // this only fires on a degenerate/edge history (empty, or leading assistant).
    if out.first().map(|m| m.role) != Some("user") {
        out.insert(
            0,
            Message {
                role: "user",
                content: vec![Block::Text {
                    text: "...".to_string(),
                }],
            },
        );
    }

    (system, out)
}

/// Push a `user` message, COALESCING into the previous entry when it is also a
/// `user` turn. Anthropic forbids two consecutive same-role messages, so parallel
/// tool results (each a `Role::Tool` message) — and a user text turn that follows
/// them — must ride ONE user message with multiple content blocks. Walk order
/// keeps `tool_result` blocks ahead of any trailing user text, which is the order
/// koma's history already produces.
fn push_user(out: &mut Vec<Message>, mut blocks: Vec<Block>) {
    if let Some(last) = out.last_mut() {
        if last.role == "user" {
            last.content.append(&mut blocks);
            return;
        }
    }
    out.push(Message {
        role: "user",
        content: blocks,
    });
}

/// Build the content blocks for a `user` message: the text (marks stripped) plus
/// one base64 `image` block per surviving attachment. Image parts are gated on
/// the resolved model's capability (fail-closed here; the send path already
/// warned the user), and an unreadable file is skipped.
fn user_blocks(m: &ChatMessage, image_ctx: Option<&ImageWireCtx>) -> Vec<Block> {
    if m.attachments.is_empty() {
        let text = strip_marks(&m.content);
        if !text.trim().is_empty() {
            return vec![Block::Text { text }];
        }
        return vec![Block::Text {
            text: "...".to_string(),
        }];
    }
    // Keep the typed text (with its `[Image #N]` markers) as the first block; the
    // marker stays visible even when a part is stripped. Marks are never present
    // on attachment-bearing turns.
    let mut blocks = Vec::new();
    let text = m.content.clone();
    if !text.trim().is_empty() {
        blocks.push(Block::Text { text });
    }
    let capable = image_ctx.map(|c| c.model_takes_images).unwrap_or(false);
    if capable {
        let ctx = match image_ctx {
            Some(c) => c,
            None => return blocks,
        };
        for att in &m.attachments {
            if let Some(url) = crate::dto::openrouter::request::data_url_for(&ctx.session_dir, att)
            {
                if let Some(source) = image_source_from_data_url(&url) {
                    blocks.push(Block::Image { source });
                }
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(Block::Text {
            text: "...".to_string(),
        });
    }
    blocks
}

/// Build the content blocks for an `assistant` message. Ordering follows the
/// Anthropic validator requirement: any replayed `thinking` / `redacted_thinking`
/// blocks FIRST, then text, then `tool_use`. Returns empty for a content-less,
/// tool-less, thinking-less turn so the caller can skip it.
fn assistant_blocks(m: &ChatMessage) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    // Thinking must LEAD the assistant turn (Anthropic requires thinking → text →
    // tool_use). Replay the signed thinking captured for THIS turn (in-memory
    // `reasoning_details`, never persisted) so an interleaved-thinking tool loop
    // doesn't 400 on the continuation request. Skip an unsigned thinking detail:
    // real Anthropic REQUIRES the signature, so dropping it is safer than sending
    // `signature: ""`.
    if let Some(details) = &m.reasoning_details {
        for d in details {
            match d.kind.as_deref() {
                Some("thinking") => {
                    let sig = d.signature.clone().unwrap_or_default();
                    if !sig.is_empty() {
                        blocks.push(Block::Thinking {
                            thinking: d.text.clone().unwrap_or_default(),
                            signature: sig,
                        });
                    }
                }
                Some("redacted_thinking") => {
                    if let Some(data) = &d.data {
                        blocks.push(Block::RedactedThinking { data: data.clone() });
                    }
                }
                _ => {}
            }
        }
    }
    if !m.content.is_empty() {
        blocks.push(Block::Text {
            text: m.content.clone(),
        });
    }
    if let Some(calls) = &m.tool_calls {
        for tc in calls {
            blocks.push(Block::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input: tool_use_input(&tc.function.arguments),
            });
        }
    }
    blocks
}

/// Parse koma's stored JSON-STRING tool arguments into the JSON OBJECT Anthropic
/// wants for `tool_use.input`. Arguments are first repaired (duplicate-fragment
/// collapse) via the shared sanitiser; an empty or non-object result becomes
/// `{}` (Anthropic's `input` must be an object).
fn tool_use_input(arguments: &str) -> Value {
    let cleaned = crate::dto::chat::sanitize_tool_arguments(arguments);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return serde_json::json!({});
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(v) if v.is_object() => v,
        _ => serde_json::json!({}),
    }
}

/// Split a `data:<media_type>;base64,<data>` URL (as produced by
/// [`data_url_for`](crate::dto::openrouter::request::data_url_for)) into an
/// Anthropic base64 [`ImageSource`]. `None` on a malformed URL.
fn image_source_from_data_url(url: &str) -> Option<ImageSource> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if media_type.is_empty() || data.is_empty() {
        return None;
    }
    Some(ImageSource {
        kind: "base64",
        media_type: media_type.to_string(),
        data: data.to_string(),
    })
}

/// Flatten the advertised built-in tools (`all_tools()` ∩ `advertise`) plus the
/// caller-supplied MCP tool defs into Anthropic [`AnthropicTool`]s. Same filter as
/// the other transports; names are NOT prefixed. A missing/empty schema falls
/// back to a minimal empty-object schema.
pub(super) fn flatten_tools(advertise: &[String], mcp_tools: &[ToolDef]) -> Vec<AnthropicTool> {
    let mut out: Vec<AnthropicTool> = crate::tool::all_tools()
        .iter()
        .filter(|t| advertise.iter().any(|n| n == t.name()))
        .map(|t| AnthropicTool {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: normalize_schema(t.parameters()),
        })
        .collect();
    for md in mcp_tools {
        out.push(AnthropicTool {
            name: md.function.name.clone(),
            description: md.function.description.clone(),
            input_schema: normalize_schema(md.function.parameters.clone()),
        });
    }
    out
}

/// Ensure a tool's `input_schema` is a non-empty JSON object; otherwise substitute
/// the minimal `{"type":"object","properties":{}}` schema Anthropic requires.
fn normalize_schema(schema: Value) -> Value {
    match &schema {
        Value::Object(m) if !m.is_empty() => schema,
        _ => serde_json::json!({"type": "object", "properties": {}}),
    }
}

/// The exact Anthropic `output_config.effort` levels koma's effort token maps to
/// 1:1 (mirrors this endpoint's `supported_efforts` in `models.json`). A token
/// outside this set — and outside the `off` / empty / `default` special cases — is
/// treated as "adaptive thinking, no explicit effort".
const ANTHROPIC_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Compute the `(thinking, context_management, output_config)` request-body triple
/// for the Anthropic Messages API from koma's stored effort token.
///
/// Target = CURRENT Claude models (Opus 4.8 / Sonnet 5) over OAuth = ADAPTIVE
/// thinking. These shapes are 400-sensitive — do not reorder or rename fields.
///
/// `context_management` is always `None`: the `clear_thinking_20251015` edit
/// requires the `context-management-2025-06-27` beta, which koma does not send,
/// so the field is dropped entirely rather than gated behind an unsent beta.
///
/// - `tool_choice_forced` (the oneshot forced-`respond` structured-output path):
///   Anthropic DELETES thinking under a forced `tool_choice`, so send none of the
///   three → `(None, None, None)`.
/// - `"off"` / `"none"`: adaptive thinking can't be disabled on these models, and
///   pinning `output_config.effort` requires the (unsent) `effort-2025-11-24`
///   beta, so this is the same all-None shape as forced → `(None, None, None)`.
/// - `"low"`/`"medium"`/`"high"`/`"xhigh"`/`"max"`: adaptive thinking WITH the
///   effort echoed verbatim into `output_config.effort`.
/// - `""` / `"default"` / any UNKNOWN token: adaptive thinking, NO explicit effort
///   (`output_config = None`).
///
/// The caller sets the header `thinking_on` flag to `thinking.is_some()`. Since
/// `output_config` is only ever `Some` alongside `thinking = Some` (the effort-level
/// branch above), `thinking_on` correctly gates the `effort-2025-11-24` beta too
/// (see [`anthropic_headers`](super::anthropic_headers)).
pub(super) fn thinking_params(
    effort: &str,
    tool_choice_forced: bool,
) -> (Option<Value>, Option<Value>, Option<Value>) {
    if tool_choice_forced {
        return (None, None, None);
    }
    let adaptive = || serde_json::json!({"type": "adaptive", "display": "summarized"});
    match effort.trim() {
        // Adaptive thinking can't be disabled on these models, and pinning
        // output_config.effort here would need an unsent beta — same as forced.
        "off" | "none" => (None, None, None),
        level if ANTHROPIC_EFFORTS.contains(&level) => (
            Some(adaptive()),
            None,
            Some(serde_json::json!({"effort": level})),
        ),
        // "", "default", and any unknown token → adaptive, no explicit effort.
        _ => (Some(adaptive()), None, None),
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;

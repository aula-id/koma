//! [`ChatMessage`] — a single turn in a conversation.

use serde::{Deserialize, Serialize};

use super::attachment::Attachment;
use super::role::Role;
use super::tool::ToolCall;

/// A single turn in a conversation: who spoke and what they said.
///
/// Serialised to / from JSON so it can be stored in `messages.json` and sent
/// directly in `ChatRequest::messages` without a mapping step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Present on an assistant message that requested one or more tool calls.
    /// Serialised as the OpenAI/OpenRouter `tool_calls` array; omitted entirely
    /// on plain messages so existing `messages.json` files stay compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Present on a `tool`-role message: the id of the assistant `tool_calls`
    /// entry this message answers. Omitted on every other message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Image attachments carried by this (user) message. Each links to an
    /// on-disk file under `<session>/images/` and matches an `[Image #N]` marker
    /// in `content`. Serialised only when non-empty — a message without
    /// attachments writes BYTE-IDENTICAL `messages.json` to before this field
    /// existed, and old files (no `attachments` key) deserialise to an empty vec
    /// via `#[serde(default)]`. The bytes are never inlined here; the wire
    /// builder re-reads them from disk at send time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Display-only reasoning/thinking text accumulated from the model's
    /// `delta.reasoning` channel during streaming. `#[serde(skip)]` means it is
    /// NEVER serialised — not into a `ChatRequest` body nor `messages.json` — and
    /// always defaults to `None` on deserialise. This keeps reasoning purely a
    /// render-time concern: it shows above the answer but never re-enters the
    /// conversation the model sees, and never touches disk.
    #[serde(skip)]
    pub reasoning: Option<String>,
}

impl ChatMessage {
    /// Construct a plain message, accepting any `Into<String>` for convenience.
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            attachments: Vec::new(),
            reasoning: None,
        }
    }

    /// Construct an assistant message that requested tool calls. `content` may
    /// be empty (the model often emits tool calls with no accompanying text).
    pub fn assistant_with_tools(content: String, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            attachments: Vec::new(),
            reasoning: None,
        }
    }

    /// Construct a `tool`-role result message answering a specific tool call.
    pub fn tool_result(tool_call_id: String, content: String) -> Self {
        Self {
            role: Role::Tool,
            content,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            attachments: Vec::new(),
            reasoning: None,
        }
    }

    /// Attach the image attachments collected in the composer onto this message
    /// (builder style). Used at user-submit time to fold the pending composer
    /// attachments onto the message before it enters the conversation.
    pub fn with_attachments(mut self, attachments: Vec<Attachment>) -> Self {
        self.attachments = attachments;
        self
    }

    /// Attach a display-only reasoning block (builder style). An empty/`None`
    /// reasoning leaves the field `None` so no empty thinking block renders.
    /// Used at assistant-commit time to fold the streamed reasoning buffer onto
    /// the message before it enters the conversation.
    pub fn with_reasoning(mut self, reasoning: Option<String>) -> Self {
        self.reasoning = reasoning.filter(|r| !r.is_empty());
        self
    }
}

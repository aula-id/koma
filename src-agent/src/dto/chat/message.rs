//! [`ChatMessage`] — a single turn in a conversation.

use serde::{Deserialize, Serialize};

use super::attachment::Attachment;
use super::role::Role;
use super::tool::ToolCall;

/// One OpenRouter `reasoning_details` array entry. Captured from the response and
/// echoed back UNMODIFIED on tool-continuation requests so reasoning models keep
/// their chain-of-thought across tool calls within a turn. Typed fields cover the
/// documented shape; `extra` (serde flatten) preserves any unknown fields verbatim
/// for byte-fidelity / forward-compat (signatures are load-bearing — never drop them).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReasoningDetail {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Merge streamed `reasoning_details` fragments into an accumulator. OpenRouter
/// streams these in chunks carrying an `index`; entries with the same `Some(index)`
/// are concatenated (text/summary/data appended in arrival order; signature/id/
/// format/kind/extra last-write-wins). Entries with `index == None` (e.g. a
/// non-streamed full array) are appended as-is.
pub fn merge_reasoning_details(acc: &mut Vec<ReasoningDetail>, incoming: Vec<ReasoningDetail>) {
    for d in incoming {
        let slot = if d.index.is_some() {
            acc.iter_mut().find(|e| e.index == d.index)
        } else {
            None
        };
        match slot {
            Some(e) => {
                if let Some(t) = d.text { e.text.get_or_insert_with(String::new).push_str(&t); }
                if let Some(s) = d.summary { e.summary.get_or_insert_with(String::new).push_str(&s); }
                if let Some(dd) = d.data { e.data.get_or_insert_with(String::new).push_str(&dd); }
                if d.signature.is_some() { e.signature = d.signature; }
                if d.id.is_some() { e.id = d.id; }
                if d.format.is_some() { e.format = d.format; }
                if d.kind.is_some() { e.kind = d.kind; }
                for (k, v) in d.extra { e.extra.insert(k, v); }
            }
            None => acc.push(d),
        }
    }
}

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
    /// Display+replay-only OpenRouter reasoning_details for THIS assistant turn.
    /// `#[serde(skip)]` — never touches `messages.json` and never re-enters the
    /// conversation via disk; it lives only in-memory within an active turn so the
    /// wire builder can echo it back on tool-continuation requests (OpenRouter only).
    #[serde(skip)]
    pub reasoning_details: Option<Vec<ReasoningDetail>>,
    /// True when this assistant message's `content` was PROMOTED from the
    /// `reasoning` channel by `final_answer` (the model left `content` empty and
    /// streamed its whole answer into reasoning — e.g. some xAI/grok reasoning
    /// turns on the generic OpenAI path). Promoted turns are still stored and
    /// displayed normally, but EXCLUDED from the wire history on replay
    /// (`Conversation::history`) so raw chain-of-thought never few-shot-
    /// conditions other models into skipping tool use. `#[serde(default)]` so
    /// old `messages.json` files without this key deserialise to `false`;
    /// `skip_serializing_if` keeps ordinary turns byte-identical on disk.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning_promoted: bool,
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
            reasoning_details: None,
            reasoning_promoted: false,
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
            reasoning_details: None,
            reasoning_promoted: false,
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
            reasoning_details: None,
            reasoning_promoted: false,
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

    /// Attach OpenRouter reasoning_details (builder style). Empty → `None`.
    pub fn with_reasoning_details(mut self, details: Option<Vec<ReasoningDetail>>) -> Self {
        self.reasoning_details = details.filter(|d| !d.is_empty());
        self
    }

    /// Mark this assistant message's content as promoted from reasoning
    /// (builder style). Used at assistant-commit time so `Conversation::history`
    /// can exclude it from the wire while storage/display keep it untouched.
    pub fn with_reasoning_promoted(mut self, promoted: bool) -> Self {
        self.reasoning_promoted = promoted;
        self
    }
}

//! Outbound request types for the OpenRouter chat-completions API.
//!
//! Covers prompt-caching wire types, provider routing, reasoning config,
//! tool definitions, and the top-level `ChatRequest` POST body.

use std::path::{Path, PathBuf};

use serde::Serialize;
use crate::dto::chat::ChatMessage;

// ---------------------------------------------------------------------------
// Prompt-caching wire layer
// ---------------------------------------------------------------------------
//
// OpenRouter prompt caching wants ONE `cache_control: {"type":"ephemeral"}`
// breakpoint on the LAST content block of the stable prefix (here: the system
// message). `cache_control` can only ride a content block, so a cached message
// must serialise its `content` as an ARRAY of parts rather than a plain string.
//
// `ChatMessage.content` stays a `String` (it's used everywhere); this is a
// serialise-only mirror used solely when building the request body. Non-system
// messages serialise `content` as a plain string — byte-identical to the old
// wire format — so nothing about existing behaviour changes for them.

/// `cache_control` marker placed on a content part to open a cache breakpoint.
/// Serialises to `{"type":"ephemeral"}`. `pub(crate)` only to satisfy the
/// reachability of the wire types it's nested under; never named outside this
/// module.
#[derive(Serialize)]
pub(crate) struct CacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
}

/// One content part of a multi-part message `content` array. Carries the text
/// plus an optional `cache_control` breakpoint (set only on the last part of
/// the stable prefix). `pub(crate)` only to satisfy the reachability of the wire
/// types it's nested under; never named outside this module.
#[derive(Serialize)]
pub(crate) struct ContentPart {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

/// The `image_url` object of an [`ImagePart`]. `url` is a `data:<mime>;base64,…`
/// data-URL built from the on-disk image bytes at send time (the bytes are NOT
/// stored in the message — see [`crate::dto::chat::Attachment`]).
#[derive(Serialize)]
pub(crate) struct ImageUrl {
    url: String,
}

/// One `image_url` content part: `{"type":"image_url","image_url":{"url":"data:…"}}`.
/// Emitted only for a user message whose attachments survive the capability gate
/// (the current model can read images). `pub(crate)` only for reachability under
/// the wire types; never named outside this module.
#[derive(Serialize)]
pub(crate) struct ImagePart {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: ImageUrl,
}

/// One element of a multi-part message `content` array: either a `text` part
/// (the existing cache-aware [`ContentPart`]) or an `image_url` part. `#[serde(untagged)]`
/// so each serialises to its own shape inline in the array, matching the
/// OpenAI/OpenRouter content-parts format.
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum WirePart {
    Text(ContentPart),
    Image(ImagePart),
}

/// Message `content` on the wire: either a plain string (no caching, identical
/// to the old format) or an array of parts (when a `cache_control` breakpoint
/// must be attached). `#[serde(untagged)]` makes the string serialise as a bare
/// JSON string and the parts as a JSON array, matching the API's accepted shapes.
///
/// `pub(crate)` only to match the reachability of `WireMessage::content` (which
/// is `pub` on a `pub(crate)` struct); it's never constructed outside this module
/// — `to_wire` is the only producer.
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum WireContent {
    Text(String),
    Parts(Vec<WirePart>),
}

/// Serialise-only mirror of [`ChatMessage`] for the request body. Field names /
/// serde attrs match `ChatMessage` exactly (`tool_calls` / `tool_call_id` with
/// `skip_serializing_if`) so the wire format is identical, except `content` can
/// now carry a `cache_control` breakpoint via [`WireContent::Parts`]. The
/// display-only `reasoning` field is intentionally absent (it's `#[serde(skip)]`
/// on `ChatMessage`, i.e. never sent).
#[derive(Serialize)]
pub struct WireMessage {
    pub role: crate::dto::chat::Role,
    pub content: WireContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<crate::dto::chat::ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Convert a conversation history into wire messages, placing the single prompt-
/// caching breakpoint on the system message (the stable prefix).
///
/// The System message gets `WireContent::Parts`; every other message serialises as
/// `WireContent::Text` — a plain `"content":"…"` string, byte-identical to the
/// pre-caching wire format. If there is no System message, no breakpoint is set
/// (all messages stay plain strings).
///
/// The System content may carry a [`CACHE_SPLIT_MARK`](crate::dto::chat::CACHE_SPLIT_MARK)
/// boundary that separates the STABLE cached head (base prompt + plan-word steer)
/// from the VOLATILE tail (project file listing + awareness summary, which change
/// across sessions/turns). When present, the content is split once at the mark
/// (which is stripped, never sent):
/// - the head becomes a `ContentPart` WITH the `cache_control: ephemeral`
///   breakpoint — only this byte-stable prefix is cached, so file changes in the
///   tail never bust the cache,
/// - the tail becomes a SECOND `ContentPart` WITHOUT `cache_control`, emitted only
///   when non-empty (an empty tail collapses to the single cached part).
///
/// When the System content has NO mark (e.g. secondary/utility calls that build a
/// fresh system message), the whole content is emitted as one cached part — the
/// original pre-split behaviour.
pub fn to_wire(messages: Vec<ChatMessage>) -> Vec<WireMessage> {
    to_wire_with_images(messages, None)
}

/// Image-attachment context for [`to_wire_with_images`]: where the on-disk images
/// live + whether the target model can read them. Built by the streaming send
/// path; the secondary/oneshot paths pass `None` (they never carry attachments).
pub struct ImageWireCtx {
    /// The SESSION directory `<…>/<uuid>/`. Each attachment's `rel_path`
    /// (`images/NN-name.ext`) is resolved against this to read the bytes back.
    pub session_dir: PathBuf,
    /// Whether the resolved Main model accepts image inputs (from
    /// `model_takes_images` over the cached catalogue). When false, image parts
    /// are silently stripped (the submit-time guard in handle_submit already
    /// posted a user-facing notice before the wire build runs).
    pub model_takes_images: bool,
}

/// Convert a conversation history into wire messages, attaching the prompt-cache
/// breakpoint to the System message AND — when `image_ctx` is `Some` — rendering
/// any user-message image attachments as `image_url` content parts (or stripping
/// them with a model-visible warning when the model can't read images).
///
/// The System message gets `WireContent::Parts`; a user message WITH attachments
/// also becomes `WireContent::Parts` (a text part + one image_url part per
/// surviving attachment); EVERY message without attachments serialises as
/// `WireContent::Text` — a plain `"content":"…"` string, byte-identical to the
/// pre-attachment wire format, so nothing regresses for them.
///
/// The System content may carry a [`CACHE_SPLIT_MARK`](crate::dto::chat::CACHE_SPLIT_MARK)
/// boundary that separates the STABLE cached head (base prompt + plan-word steer)
/// from the VOLATILE tail (project file listing + awareness summary, which change
/// across sessions/turns). When present, the content is split once at the mark
/// (which is stripped, never sent):
/// - the head becomes a `ContentPart` WITH the `cache_control: ephemeral`
///   breakpoint — only this byte-stable prefix is cached, so file changes in the
///   tail never bust the cache,
/// - the tail becomes a SECOND `ContentPart` WITHOUT `cache_control`, emitted only
///   when non-empty (an empty tail collapses to the single cached part).
///
/// When the System content has NO mark (e.g. secondary/utility calls that build a
/// fresh system message), the whole content is emitted as one cached part — the
/// original pre-split behaviour.
pub fn to_wire_with_images(
    messages: Vec<ChatMessage>,
    image_ctx: Option<&ImageWireCtx>,
) -> Vec<WireMessage> {
    messages
        .into_iter()
        .map(|m| {
            let content = if m.role == crate::dto::chat::Role::System {
                WireContent::Parts(system_parts(m.content))
            } else if !m.attachments.is_empty() {
                // A message carrying image attachments becomes a parts array: the
                // typed text (marker included) + image_url / warning parts. When
                // there is no image context (secondary/oneshot path) the text
                // still rides as a parts array with no images — harmless, but in
                // practice those paths never carry attachments.
                WireContent::Parts(attachment_parts(&m.content, &m.attachments, image_ctx))
            } else {
                // Strip a leading `!`-shell SHELL_MARK so the model reads the clean
                // `$ <cmd>\n<output>` text (the invisible mark is a transcript-render
                // device only). A no-op for every other message. `strip_prefix`
                // returns the slice after the mark, or `None` (→ unchanged) otherwise.
                let text = m
                    .content
                    .strip_prefix(crate::dto::chat::SHELL_MARK)
                    .map(str::to_string)
                    .unwrap_or(m.content);
                WireContent::Text(text)
            };
            // Repair any tool-call argument string on the way OUT. A provider that
            // violated the streaming-delta contract may have persisted a malformed
            // `{...}{...}` arguments value into a stored assistant message; sending it
            // verbatim makes the provider's prefill/validation reject the whole
            // request ("unexpected content after document"), wedging the session.
            // `m.tool_calls` here is an owned clone of the stored message (the caller
            // passes `conversation.history()` clones), so cleaning it touches ONLY this
            // wire copy — the stored `ChatMessage` / `messages.json` is never mutated.
            // A single clean value is left semantically unchanged (no-op).
            let tool_calls = m.tool_calls.map(|mut calls| {
                for call in &mut calls {
                    call.function.arguments =
                        crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
                }
                calls
            });
            WireMessage {
                role: m.role,
                content,
                tool_calls,
                tool_call_id: m.tool_call_id,
            }
        })
        .collect()
}

/// Build a user message's content parts: the typed text first, then — per
/// attachment — either an `image_url` part (model CAN read images) or nothing on
/// the image rail plus ONE appended warning text part naming every stripped image
/// (model CANNOT read images, or no context). The text part is always present so
/// the `[Image #N]` markers the user typed stay visible to the model.
fn attachment_parts(
    text: &str,
    attachments: &[crate::dto::chat::Attachment],
    image_ctx: Option<&ImageWireCtx>,
) -> Vec<WirePart> {
    let mut parts: Vec<WirePart> = Vec::with_capacity(1 + attachments.len());
    parts.push(WirePart::Text(ContentPart {
        kind: "text",
        text: text.to_string(),
        cache_control: None,
    }));

    let capable = image_ctx.map(|c| c.model_takes_images).unwrap_or(false);
    if capable {
        // SOURCE OF RECORD IS DISK: read each image off disk and base64-encode it
        // into a data-URL at send time, so resume re-derives it from the file.
        // An unreadable file is silently skipped (no crash) — the marker still
        // shows in the text part so the model knows an image was intended.
        let ctx = image_ctx.expect("capable implies Some");
        for att in attachments {
            if let Some(url) = data_url_for(&ctx.session_dir, att) {
                parts.push(WirePart::Image(ImagePart {
                    kind: "image_url",
                    image_url: ImageUrl { url },
                }));
            }
        }
    } else {
        // SILENT STRIP: the submit-time guard (handle_submit) already blocks
        // sending images to a non-vision model and shows a user-facing chat
        // notice; here we just omit the image parts (safety net for a
        // cold-cache / capability-flip race).
        let _ = attachments; // nothing to push
    }
    parts
}

/// Read an attachment's on-disk bytes and build its `data:<mime>;base64,<…>` URL,
/// or `None` when the file can't be read. `rel_path` (`images/NN-name.ext`) is
/// resolved against the session dir — the bytes are NEVER taken from the message.
fn data_url_for(session_dir: &Path, att: &crate::dto::chat::Attachment) -> Option<String> {
    use base64::Engine;
    let path = session_dir.join(&att.rel_path);
    let bytes = std::fs::read(&path).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{};base64,{}", att.mime, b64))
}

/// Build the System message's wire content parts, splitting the stable cached head
/// from the volatile tail on [`CACHE_SPLIT_MARK`](crate::dto::chat::CACHE_SPLIT_MARK).
///
/// - mark present → `[head WITH cache_control, tail WITHOUT cache_control]`, the
///   tail part included only if non-empty; the mark itself is stripped.
/// - mark absent (or an empty tail) → a single cached part holding the whole
///   content — the original behaviour.
fn system_parts(content: String) -> Vec<WirePart> {
    match content.split_once(crate::dto::chat::CACHE_SPLIT_MARK) {
        Some((head, tail)) if !tail.is_empty() => vec![
            WirePart::Text(ContentPart {
                kind: "text",
                text: head.to_string(),
                cache_control: Some(CacheControl { kind: "ephemeral" }),
            }),
            WirePart::Text(ContentPart {
                kind: "text",
                text: tail.to_string(),
                cache_control: None,
            }),
        ],
        // No mark, or an empty tail: one cached part holding the head (mark
        // stripped). `split_once` returns the head before the mark; with no mark
        // the whole string is the head.
        Some((head, _)) => vec![WirePart::Text(ContentPart {
            kind: "text",
            text: head.to_string(),
            cache_control: Some(CacheControl { kind: "ephemeral" }),
        })],
        None => vec![WirePart::Text(ContentPart {
            kind: "text",
            text: content,
            cache_control: Some(CacheControl { kind: "ephemeral" }),
        })],
    }
}

/// OpenRouter provider-routing directive for strict provider pinning.
///
/// When set, the request is routed exclusively through the listed provider with
/// no fallback. Omitting this struct entirely (via `skip_serializing_if`) lets
/// OpenRouter use its default routing logic.
#[derive(Debug, Serialize)]
pub struct ProviderRouting {
    pub only: Vec<String>,
    pub allow_fallbacks: bool,
}

/// Request-side usage accounting directive.
///
/// Serialises to `{"include": true}`. When present on the request body,
/// OpenRouter returns token counts AND the total generation `cost` in the
/// response — including the final streaming chunk (which carries `usage` with
/// an empty `choices` array).
#[derive(Debug, Clone, Serialize)]
pub struct UsageRequest {
    pub include: bool,
}

/// Reasoning/thinking control for the request, serialised as the top-level
/// `"reasoning"` object.
///
/// Only the fields this app uses are modelled; all are skipped when
/// `None` so the on-wire object carries exactly what was set:
/// - `{"reasoning":{"effort":"high"}}` selects a thinking effort level.
/// - `{"reasoning":{"enabled":false}}` turns thinking off entirely.
/// - `{"reasoning":{"exclude":true}}` keeps reasoning mandatory (satisfying
///   endpoints that require it) but strips the `reasoning` field from the
///   response — used by all secondary/utility model calls so their replies
///   are bleed-proof and the chain-of-thought is never persisted.
///
/// Omitting the whole struct (via `skip_serializing_if` on the `ChatRequest`
/// field) lets the model use its own default reasoning behaviour. `effort`,
/// `enabled`, and `exclude` are never set together — see `reasoning_config`
/// in the service.
#[derive(Debug, Serialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

/// JSON-Schema description of one tool's `function`, as required by the
/// OpenAI/OpenRouter `tools` request field. `parameters` is the tool's raw
/// JSON-Schema object (taken verbatim from `Tool::parameters`).
#[derive(Debug, Clone, Serialize)]
pub struct ToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// One entry in the request `tools` array. `kind` is always `"function"`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionDef,
}

/// POST body for `POST /api/v1/chat/completions`.
///
/// `stream: true` triggers SSE delivery; `stream: false` waits for the full
/// response (used only by the compaction summary request).
#[derive(Serialize)]
pub struct ChatRequest {
    pub model: String,
    /// Wire-format messages. Built from a `Vec<ChatMessage>` via [`to_wire`],
    /// which attaches the single prompt-caching `cache_control` breakpoint to
    /// the system message; non-system messages serialise as plain strings.
    pub messages: Vec<WireMessage>,
    pub stream: bool,
    /// Optional provider-routing directive. When `Some`, the request is strictly
    /// pinned to the specified provider. When `None`, the field is omitted and
    /// OpenRouter uses its default routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderRouting>,
    /// Usage accounting directive — always sent as `{"include": true}` so the
    /// response (and final streaming chunk) reports token counts + total cost.
    pub usage: UsageRequest,
    /// Function-calling tool definitions exposed to the model. `Some` on the
    /// streaming chat path (so the model can request tool calls); omitted via
    /// `skip_serializing_if` on the `/compact` summary call, which uses no tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    /// Reasoning/thinking directive. `Some` only on the interactive chat path
    /// (set from the session's `effort`); `None` everywhere else (compaction +
    /// secondary-model calls don't think) and omitted from the body via
    /// `skip_serializing_if`, so the model falls back to its own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    /// Structured-output directive, serialised as the top-level `response_format`
    /// object. `Some` only on the classifier path, where it pins a strict
    /// `json_schema` so the verdict comes back as machine-parseable JSON; `None`
    /// everywhere else (and omitted from the body via `skip_serializing_if`), so
    /// the interactive/compaction calls emit free-form text as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
    /// Hard cap on generated tokens. A generous limit (e.g. 32 000) on the
    /// interactive path guards against runaway generation; small caps (e.g. 2 000)
    /// on classifier/picker calls that return tiny JSON. `None` lets the provider
    /// use its own default. Omitted from the wire body when `None` via
    /// `skip_serializing_if`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

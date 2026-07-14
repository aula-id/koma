//! Streaming chat completion over the Anthropic Messages API SSE wire.

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::dto::chat::{ChatMessage, FunctionCall, ReasoningDetail, ToolCall};
use crate::dto::openrouter::{ImageWireCtx, ToolDef};
use crate::service::StreamEvent;

use super::super::helpers::{clean_error, emit, sanitize_tool_acc};
use super::super::Conn;
use super::super::OpenRouterClient;
use super::request::{build_messages, flatten_tools, thinking_params, MessagesRequest};
use super::sse::{parse_event, AnthropicEvent, BlockDelta, ContentBlockStart};
use super::{anthropic_headers, error_message, CLAUDE_MAX_OUTPUT_TOKENS};

/// A tool_use block being reconstructed from the stream: its id + name (from
/// `content_block_start`) and the incrementally-accumulated input-JSON string
/// (from `input_json_delta` fragments). `index` ties deltas back to this block.
struct PartialToolUse {
    index: usize,
    id: String,
    name: String,
    buf: String,
}

/// Convert the reconstructed tool_use blocks into koma [`ToolCall`]s, in block
/// (arrival) order. The concatenated `partial_json` IS koma's stringified
/// arguments — used verbatim; an empty buffer (a no-argument tool) becomes `{}`.
fn finalize_tools(blocks: &[PartialToolUse]) -> Vec<ToolCall> {
    blocks
        .iter()
        .map(|b| ToolCall {
            id: b.id.clone(),
            kind: "function".to_string(),
            function: FunctionCall {
                name: b.name.clone(),
                arguments: if b.buf.trim().is_empty() {
                    "{}".to_string()
                } else {
                    b.buf.clone()
                },
            },
        })
        .collect()
}

/// A thinking / redacted_thinking block being reconstructed from the stream.
/// `text` accumulates `thinking_delta`s (seeded from the block-start); `sig`
/// accumulates `signature_delta`s (seeded from the block-start); `redacted_data`
/// is `Some` for a `redacted_thinking` block. `index` ties deltas back to it.
struct PartialThinking {
    index: usize,
    text: String,
    sig: String,
    redacted_data: Option<String>,
}

/// Convert the reconstructed thinking blocks into koma [`ReasoningDetail`]s for
/// intra-turn REPLAY (mirrors [`finalize_tools`]). A `thinking` block carries its
/// accumulated text + signature (`None` when unsigned — dropped at replay time in
/// `assistant_blocks`); a `redacted_thinking` block carries its opaque `data`.
/// Block (arrival) order is preserved — thinking must lead the replayed turn.
fn finalize_thinking(blocks: &[PartialThinking]) -> Vec<ReasoningDetail> {
    blocks
        .iter()
        .map(|b| {
            if let Some(data) = &b.redacted_data {
                ReasoningDetail {
                    kind: Some("redacted_thinking".to_string()),
                    data: Some(data.clone()),
                    ..Default::default()
                }
            } else {
                ReasoningDetail {
                    kind: Some("thinking".to_string()),
                    text: Some(b.text.clone()),
                    signature: (!b.sig.is_empty()).then(|| b.sig.clone()),
                    ..Default::default()
                }
            }
        })
        .collect()
}

impl OpenRouterClient {
    /// Streaming completion over the Anthropic Messages API.
    ///
    /// Same [`StreamEvent`] contract as `stream_complete`: every failure emits a
    /// single [`StreamEvent::Error`] and returns `Ok(())`; the spawned caller
    /// discards the return value. `bearer` is passed in from the dispatch branch,
    /// which already ran `fresh_key` — this method does NOT refresh again.
    /// `account_id` is accepted for call-site parity with `codex_stream_complete`
    /// but unused (Anthropic OAuth has no account header); `effort` now drives the
    /// extended-thinking body params via [`thinking_params`](super::request::thinking_params).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::openrouter) async fn anthropic_stream_complete(
        &self,
        conn: Conn<'_>,
        bearer: &str,
        account_id: &str,
        model: &str,
        effort: &str,
        messages: Vec<ChatMessage>,
        advertise: &[String],
        mcp_tools: &[ToolDef],
        image_ctx: Option<ImageWireCtx>,
        tx: UnboundedSender<StreamEvent>,
    ) -> Result<()> {
        let _ = account_id; // reserved for codex-parity signature
        let url = format!("{}/v1/messages?beta=true", conn.endpoint);

        let (system, msgs) = build_messages(messages, image_ctx.as_ref());
        // Empty tool set → omit `tools`/`tool_choice` entirely rather than send [].
        let tools = flatten_tools(advertise, mcp_tools);
        let (tools, tool_choice) = if tools.is_empty() {
            (None, None)
        } else {
            (Some(tools), Some(serde_json::json!({"type": "auto"})))
        };
        // Extended thinking: map the resolved role's effort token to the adaptive
        // thinking body params. The interactive path is never a forced tool_choice.
        // `thinking_on` (a thinking param is present) also gates the beta header.
        let (thinking, context_management, output_config) = thinking_params(effort, false);
        let thinking_on = thinking.is_some();
        let body = MessagesRequest {
            model: model.to_string(),
            system,
            messages: msgs,
            tools,
            tool_choice,
            max_tokens: CLAUDE_MAX_OUTPUT_TOKENS,
            thinking,
            context_management,
            output_config,
            stream: true,
        };

        let rb = anthropic_headers(self.http.post(&url), bearer, thinking_on);
        let resp = match rb.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                if let Some(ctx) = image_ctx.as_ref() {
                    crate::model::store::append_error_log(
                        &ctx.session_dir,
                        "request send failed",
                        &e.to_string(),
                    );
                }
                emit(&tx, StreamEvent::Error(format!("request failed: {e}")));
                return Ok(());
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            if let Some(ctx) = image_ctx.as_ref() {
                crate::model::store::append_error_log(
                    &ctx.session_dir,
                    &format!("HTTP {status} from {} (model {model})", conn.endpoint),
                    &text,
                );
            }
            emit(&tx, StreamEvent::Error(clean_error(status, &text)));
            return Ok(());
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        // tool_use blocks reconstructed by `index` across many SSE frames.
        let mut tool_blocks: Vec<PartialToolUse> = Vec::new();
        // thinking / redacted_thinking blocks reconstructed by `index`; streamed
        // live to the reasoning channel via `thinking_delta`, and finalized into
        // ReasoningDetails for intra-turn replay at message_stop/EOF.
        let mut thinking_blocks: Vec<PartialThinking> = Vec::new();
        // Usage accumulates across events: input (+ cache) at message_start, output
        // at message_delta; emitted together at the terminal message_stop.
        let mut prompt_tokens: u64 = 0;
        let mut cached_tokens: u64 = 0;
        let mut completion_tokens: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(b) => b,
                Err(e) => {
                    if let Some(ctx) = image_ctx.as_ref() {
                        crate::model::store::append_error_log(
                            &ctx.session_dir,
                            "stream read error",
                            &e.to_string(),
                        );
                    }
                    emit(&tx, StreamEvent::Error(format!("stream error: {e}")));
                    return Ok(());
                }
            };
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                let line = line.trim_end(); // strip trailing \r\n
                if line.is_empty() {
                    continue; // SSE event separator
                }
                // Only `data:` lines carry the JSON payload; `event:` lines (and
                // comments/keepalives) are skipped — the payload's own `type` field
                // is authoritative.
                let data = match line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                {
                    Some(d) => d.trim(),
                    None => continue,
                };
                let Some(event) = parse_event(data) else {
                    continue; // partial/unmodelled payload
                };
                match event {
                    AnthropicEvent::MessageStart { message } => {
                        if let Some(u) = message.usage {
                            // koma's `prompt_tokens` is the TOTAL input; Anthropic
                            // reports the cached share separately, so fold it in.
                            prompt_tokens = u.input_tokens + u.cache_read_input_tokens;
                            cached_tokens = u.cache_read_input_tokens;
                        }
                    }
                    AnthropicEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => match content_block {
                        ContentBlockStart::ToolUse { id, name } => {
                            tool_blocks.push(PartialToolUse {
                                index,
                                id,
                                name,
                                buf: String::new(),
                            });
                        }
                        // Open a thinking block; seed text/signature from the start
                        // (usually empty — the body streams via thinking_delta).
                        ContentBlockStart::Thinking { thinking, signature } => {
                            thinking_blocks.push(PartialThinking {
                                index,
                                text: thinking,
                                sig: signature.unwrap_or_default(),
                                redacted_data: None,
                            });
                        }
                        // Open a redacted (encrypted) thinking block.
                        ContentBlockStart::RedactedThinking { data } => {
                            thinking_blocks.push(PartialThinking {
                                index,
                                text: String::new(),
                                sig: String::new(),
                                redacted_data: Some(data),
                            });
                        }
                        // text / other blocks: nothing to open.
                        ContentBlockStart::Text { .. } | ContentBlockStart::Other => {}
                    },
                    AnthropicEvent::ContentBlockDelta { index, delta } => match delta {
                        // Answer text — emitted straight through (Anthropic streams
                        // reasoning in separate thinking blocks, so no ThinkSplit).
                        BlockDelta::TextDelta { text } => {
                            if !text.is_empty() {
                                emit(&tx, StreamEvent::Token(text));
                            }
                        }
                        // A tool-input fragment: append to the matching block's buffer.
                        BlockDelta::InputJsonDelta { partial_json } => {
                            if let Some(b) = tool_blocks.iter_mut().find(|b| b.index == index) {
                                b.buf.push_str(&partial_json);
                            }
                        }
                        // A thinking fragment: accumulate for replay AND stream to the
                        // reasoning (display) channel — never the answer/Token channel.
                        BlockDelta::ThinkingDelta { thinking } => {
                            if let Some(b) = thinking_blocks.iter_mut().find(|b| b.index == index) {
                                b.text.push_str(&thinking);
                            }
                            if !thinking.is_empty() {
                                emit(&tx, StreamEvent::Reasoning(thinking));
                            }
                        }
                        // A signature fragment: accumulate only (load-bearing for
                        // replay; never displayed).
                        BlockDelta::SignatureDelta { signature } => {
                            if let Some(b) = thinking_blocks.iter_mut().find(|b| b.index == index) {
                                b.sig.push_str(&signature);
                            }
                        }
                        BlockDelta::Other => {}
                    },
                    AnthropicEvent::ContentBlockStop { .. } => {}
                    AnthropicEvent::MessageDelta { usage, .. } => {
                        if let Some(u) = usage {
                            completion_tokens = u.output_tokens;
                        }
                    }
                    AnthropicEvent::MessageStop => {
                        // Terminal emission order: Usage, then the replay
                        // ReasoningDetails, then any ToolCalls, then Done (ToolCalls
                        // must land just before Done, MATCHING codex).
                        emit(
                            &tx,
                            StreamEvent::Usage {
                                prompt_tokens,
                                completion_tokens,
                                cached_tokens,
                                // Subscription-billed; no per-request cost.
                                cost: 0.0,
                            },
                        );
                        let details = finalize_thinking(&thinking_blocks);
                        if !details.is_empty() {
                            emit(&tx, StreamEvent::ReasoningDetails(details));
                        }
                        let mut tools = finalize_tools(&tool_blocks);
                        if !tools.is_empty() {
                            sanitize_tool_acc(&mut tools);
                            emit(&tx, StreamEvent::ToolCalls(tools));
                        }
                        emit(&tx, StreamEvent::Done);
                        return Ok(());
                    }
                    AnthropicEvent::Error { error } => {
                        let (message, kind) = match error {
                            Some(e) => (e.message, e.kind),
                            None => (None, None),
                        };
                        let err_msg = error_message(message, kind);
                        if let Some(ctx) = image_ctx.as_ref() {
                            crate::model::store::append_error_log(
                                &ctx.session_dir,
                                "in-band stream error",
                                &err_msg,
                            );
                        }
                        emit(&tx, StreamEvent::Error(err_msg));
                        return Ok(());
                    }
                    AnthropicEvent::Other => {}
                }
            }
        }
        // Stream ended without an explicit `message_stop` (mirrors the codex EOF
        // path): flush any accumulated thinking + tool calls, then Done. No Usage
        // here — a clean run always delivers it at message_stop above.
        let details = finalize_thinking(&thinking_blocks);
        if !details.is_empty() {
            emit(&tx, StreamEvent::ReasoningDetails(details));
        }
        let mut tools = finalize_tools(&tool_blocks);
        if !tools.is_empty() {
            sanitize_tool_acc(&mut tools);
            emit(&tx, StreamEvent::ToolCalls(tools));
        }
        emit(&tx, StreamEvent::Done);
        Ok(())
    }
}

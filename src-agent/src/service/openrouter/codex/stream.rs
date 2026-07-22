//! Streaming chat completion over the Codex Responses API SSE wire.

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::dto::chat::{ChatMessage, FunctionCall, ReasoningDetail, ToolCall};
use crate::dto::openrouter::{ImageWireCtx, ToolDef};
use crate::service::StreamEvent;

use super::super::helpers::{clean_error, emit, sanitize_tool_acc};
use super::super::Conn;
use super::super::OpenRouterClient;
use super::request::{
    build_input, codex_effort, flatten_tools, ResponsesReasoning, ResponsesRequest,
};
use super::sse::{parse_event, OutputItem, ResponsesEvent};
use super::{codex_headers, error_message, failed_message};

impl OpenRouterClient {
    /// Streaming completion over the ChatGPT Codex Responses API.
    ///
    /// Same [`StreamEvent`] contract as `stream_complete`: every failure emits a
    /// single [`StreamEvent::Error`] and returns `Ok(())`; the spawned caller
    /// discards the return value. `bearer` + `account_id` are passed in from the
    /// dispatch branch, which already ran `fresh_key` — this method does NOT
    /// refresh again. The chat-completions `provider` route slug has no meaning
    /// on this transport and is not a parameter.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::openrouter) async fn codex_stream_complete(
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
        let url = format!("{}/responses", conn.endpoint);

        let (instructions, input) = build_input(messages, image_ctx.as_ref());
        // Empty tool set → omit `tools` entirely (skip None) rather than sending
        // an empty array.
        let tools = flatten_tools(advertise, mcp_tools);
        let tools = if tools.is_empty() { None } else { Some(tools) };
        // Codex REQUIRES a reasoning object; the effort maps through and gates the
        // encrypted-reasoning `include`.
        let (eff, include_encrypted) = codex_effort(effort);
        let include: Vec<&'static str> = if include_encrypted {
            vec!["reasoning.encrypted_content"]
        } else {
            Vec::new()
        };
        let body = ResponsesRequest {
            model: model.to_string(),
            instructions,
            input,
            tools,
            tool_choice: "auto",
            stream: true,
            store: false,
            reasoning: Some(ResponsesReasoning {
                effort: eff,
                summary: "auto",
            }),
            include,
            prompt_cache_key: self.codex_session_id().to_string(),
            text: None,
        };

        let rb = codex_headers(
            self.http.post(&url),
            bearer,
            account_id,
            self.codex_session_id(),
        );
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
        // Function calls arrive COMPLETE on their `output_item.done` event, so no
        // index-merge is needed — each is pushed whole.
        let mut tool_acc: Vec<ToolCall> = Vec::new();
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
                // comments/keepalives) are skipped — the payload's own `type`
                // field is authoritative.
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
                    // Answer text — emitted straight through (Codex never inlines
                    // `<think>` tags, so there is no ThinkSplit here).
                    ResponsesEvent::OutputTextDelta { delta } => {
                        if !delta.is_empty() {
                            emit(&tx, StreamEvent::Token(delta));
                        }
                    }
                    // Reasoning-summary text → the display-only reasoning channel.
                    ResponsesEvent::ReasoningSummaryTextDelta { delta } => {
                        if !delta.is_empty() {
                            emit(&tx, StreamEvent::Reasoning(delta));
                        }
                    }
                    ResponsesEvent::OutputItemDone { item } => match item {
                        OutputItem::FunctionCall {
                            name,
                            arguments,
                            call_id,
                        } => {
                            tool_acc.push(ToolCall {
                                id: call_id,
                                kind: "function".to_string(),
                                function: FunctionCall { name, arguments },
                            });
                        }
                        // Encrypted reasoning blob → a `codex_encrypted`
                        // reasoning_detail the runtime folds onto the assistant
                        // message and `build_input` replays on the next turn.
                        OutputItem::Reasoning {
                            id,
                            encrypted_content: Some(blob),
                        } => {
                            emit(
                                &tx,
                                StreamEvent::ReasoningDetails(vec![ReasoningDetail {
                                    kind: None,
                                    text: None,
                                    summary: None,
                                    data: Some(blob),
                                    signature: None,
                                    id,
                                    format: Some("codex_encrypted".to_string()),
                                    index: None,
                                    extra: serde_json::Map::new(),
                                }]),
                            );
                        }
                        // A reasoning item without a blob, or any other item type,
                        // carries nothing we replay.
                        OutputItem::Reasoning { .. } | OutputItem::Other => {}
                    },
                    ResponsesEvent::Completed { response } => {
                        let (prompt_tokens, completion_tokens, cached_tokens) = response
                            .usage
                            .map(|u| {
                                let cached =
                                    u.input_tokens_details.map(|d| d.cached_tokens).unwrap_or(0);
                                (u.input_tokens, u.output_tokens, cached)
                            })
                            .unwrap_or((0, 0, 0));
                        emit(
                            &tx,
                            StreamEvent::Usage {
                                prompt_tokens,
                                completion_tokens,
                                cached_tokens,
                                // Codex is subscription-billed; no per-request cost.
                                cost: 0.0,
                            },
                        );
                        if !tool_acc.is_empty() {
                            sanitize_tool_acc(&mut tool_acc);
                            emit(&tx, StreamEvent::ToolCalls(tool_acc.clone()));
                        }
                        emit(&tx, StreamEvent::Done);
                        return Ok(());
                    }
                    ResponsesEvent::Failed { response } => {
                        let err_msg = failed_message(response);
                        if let Some(ctx) = image_ctx.as_ref() {
                            crate::model::store::append_error_log(
                                &ctx.session_dir,
                                "in-band stream error (failed)",
                                &err_msg,
                            );
                        }
                        emit(&tx, StreamEvent::Error(err_msg));
                        return Ok(());
                    }
                    ResponsesEvent::Error { message, code } => {
                        let err_msg = error_message(message, code);
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
                    ResponsesEvent::Other => {}
                }
            }
        }
        // Stream ended without an explicit `response.completed` (mirrors the
        // chat-completions EOF path): flush any accumulated tool calls, then Done.
        if !tool_acc.is_empty() {
            sanitize_tool_acc(&mut tool_acc);
            emit(&tx, StreamEvent::ToolCalls(tool_acc.clone()));
        }
        emit(&tx, StreamEvent::Done);
        Ok(())
    }
}

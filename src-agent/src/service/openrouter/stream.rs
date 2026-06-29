//! Streaming chat completion over Server-Sent Events (SSE).

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{APP_TITLE, HTTP_REFERER};
use crate::dto::chat::{ChatMessage, ToolCall};
use crate::dto::openrouter::{
    to_wire_with_images, ChatRequest, ImageWireCtx, StreamChunk, ToolDef, ToolFunctionDef,
    UsageRequest,
};
use crate::service::StreamEvent;

use super::helpers::{clean_error, emit, provider_routing_for, reasoning_config, sanitize_tool_acc};
use super::client::OpenRouterClient;
use super::types::Conn;

impl OpenRouterClient {
    /// Streaming chat completion over Server-Sent Events.
    ///
    /// POSTs with `stream: true`, then reads the byte stream line-by-line:
    /// bytes are buffered until a `\n`, each complete line is stripped of its
    /// `data:` prefix, and the JSON payload is parsed into a `StreamChunk`.
    /// Each non-empty delta is emitted as [`StreamEvent::Token`]; a `[DONE]`
    /// sentinel (or stream EOF) emits [`StreamEvent::Done`]. Non-`data:` lines
    /// (SSE comments / keepalives) and unparseable partial JSON are skipped.
    ///
    /// Never panics: every failure emits [`StreamEvent::Error`] and returns
    /// `Ok(())`. The caller (a spawned task) discards the return value.
    #[allow(clippy::too_many_arguments)]
    pub async fn stream_complete(
        &self,
        conn: Conn<'_>,
        model: &str,
        provider: &str,
        effort: &str,
        messages: Vec<ChatMessage>,
        advertise: &[String],
        mcp_tools: &[ToolDef],
        image_ctx: Option<ImageWireCtx>,
        tx: UnboundedSender<StreamEvent>,
    ) -> Result<()> {
        // The plan-word steer is now injected into the System message upstream in
        // `start_stream_task`, BEFORE the volatile project-files/awareness tail and
        // ahead of the `CACHE_SPLIT_MARK` boundary, so it stays inside the cached
        // (byte-stable) head. `to_wire` splits the System content on that mark and
        // puts the cache breakpoint on the head only.
        let url = format!("{}/chat/completions", conn.endpoint);
        // Expose the requested subset of the built-in tool set to the model. The
        // caller passes the exact tool names to advertise (`advertise`): the main
        // chat loop advertises `crate::tool::main_tool_names` (everything not in
        // `INTERNAL_ONLY`, currently empty), and each sub-agent advertises only
        // its effective allow-list. Each retained tool maps to an OpenAI/OpenRouter
        // `function` definition (name + description + raw JSON-Schema parameters).
        let mut tools: Vec<ToolDef> = crate::tool::all_tools()
            .iter()
            .filter(|t| advertise.iter().any(|n| n == t.name()))
            .map(|t| ToolDef {
                kind: "function".into(),
                function: ToolFunctionDef {
                    name: t.name().into(),
                    description: t.description().into(),
                    parameters: t.parameters(),
                },
            })
            .collect();
        // Append the caller-supplied MCP tool definitions (already wire-shaped
        // `ToolDef`s built from the manager's discovered tools). These are namespaced
        // `mcp__<server>__<tool>` and were already added to `advertise` at the call
        // site, so the runtime's dispatch filter accepts the model's calls to them.
        // Empty (the common case: no MCP servers) ⇒ this is a no-op and the request
        // body is byte-identical to before MCP existed.
        tools.extend(mcp_tools.iter().cloned());
        let body = ChatRequest {
            model: model.to_string(),
            // Wrap into wire messages: the system message gets the single prompt-
            // caching breakpoint; a user message carrying image attachments becomes
            // a parts array (text + image_url, or text + strip-warning when the
            // model can't read images); everything else serialises as a plain string.
            messages: to_wire_with_images(messages, image_ctx.as_ref()),
            stream: true,
            provider: provider_routing_for(provider),
            usage: UsageRequest { include: true },
            tools: Some(tools),
            // Interactive chat is the only path that thinks; map the resolved
            // role's effort token to a `reasoning` directive (None = model default).
            reasoning: reasoning_config(effort),
            // Free-form text reply; structured output is classifier-only.
            response_format: None,
            // Generous runaway cap for the interactive path.
            max_tokens: Some(32_000),
        };

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", conn.api_key))
            .header("HTTP-Referer", HTTP_REFERER)
            .header("X-Title", APP_TITLE)
            .json(&body)
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                emit(&tx, StreamEvent::Error(format!("request failed: {e}")));
                return Ok(());
            }
        };

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            emit(
                &tx,
                StreamEvent::Error(clean_error(status, &text)),
            );
            return Ok(());
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        // Tool calls stream across many frames, one (or more) per `index`. Each
        // frame contributes the id / name once and appends argument fragments;
        // we merge them here and emit the assembled set at finalisation.
        let mut tool_acc: Vec<ToolCall> = Vec::new();
        // Last `finish_reason` seen on the active choice. OpenAI/OpenRouter set
        // it to `"tool_calls"` on the frame that closes a tool-calling turn; we
        // record it so finalisation can confirm the model wants tools run.
        let mut finished_tool_calls = false;
        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(b) => b,
                Err(e) => {
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
                let data = match line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                {
                    Some(d) => d.trim(),
                    None => continue, // comments/keepalive
                };
                if data == "[DONE]" {
                    // Finalise: any accumulated tool calls go out just before
                    // Done so the runtime can run them. The `finished_tool_calls`
                    // flag (finish_reason == "tool_calls") is the protocol-level
                    // confirmation; non-empty `tool_acc` is the data we actually
                    // need, so either being set means "run the tools".
                    if !tool_acc.is_empty() || finished_tool_calls {
                        // Repair argument strings before they leave the client:
                        // some providers re-send the FULL arguments per chunk, so
                        // blind delta concatenation yields `{...}{...}`. Collapse
                        // to one clean value so the runtime + persistence never see
                        // a malformed (and later prefill-rejected) string.
                        sanitize_tool_acc(&mut tool_acc);
                        emit(&tx, StreamEvent::ToolCalls(tool_acc.clone()));
                    }
                    emit(&tx, StreamEvent::Done);
                    return Ok(());
                }
                if let Ok(c) = serde_json::from_str::<StreamChunk>(data) {
                    // A chunk carries content / tool-call deltas OR usage (the
                    // terminal chunk has an empty `choices` array + a `usage`
                    // object). Handle each independently so a usage-bearing
                    // chunk isn't skipped.
                    if let Some(choice) = c.choices.first() {
                        if choice.finish_reason.as_deref() == Some("tool_calls") {
                            finished_tool_calls = true;
                        }
                        if let Some(t) = &choice.delta.content {
                            if !t.is_empty() {
                                emit(&tx, StreamEvent::Token(t.clone()));
                            }
                        }
                        // Reasoning rides a separate delta channel (present only
                        // when reasoning is enabled); accumulate it as a display-
                        // only block, mirroring the content handling above.
                        if let Some(r) = &choice.delta.reasoning {
                            if !r.is_empty() {
                                emit(&tx, StreamEvent::Reasoning(r.clone()));
                            }
                        }
                        if let Some(tcs) = &choice.delta.tool_calls {
                            for d in tcs {
                                // Grow the accumulator so `index` is in range,
                                // then merge this fragment into its slot.
                                while tool_acc.len() <= d.index {
                                    tool_acc.push(ToolCall {
                                        kind: "function".into(),
                                        ..Default::default()
                                    });
                                }
                                let acc = &mut tool_acc[d.index];
                                if let Some(id) = &d.id {
                                    acc.id = id.clone();
                                }
                                if let Some(f) = &d.function {
                                    if let Some(n) = &f.name {
                                        acc.function.name = n.clone();
                                    }
                                    if let Some(a) = &f.arguments {
                                        acc.function.arguments.push_str(a);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(u) = c.usage {
                        // Cache hit count lives in the optional details object;
                        // absent/null → 0 (cold prefix or no cache reporting).
                        let cached_tokens = u
                            .prompt_tokens_details
                            .as_ref()
                            .map(|d| d.cached_tokens)
                            .unwrap_or(0);
                        emit(
                            &tx,
                            StreamEvent::Usage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                cached_tokens,
                                cost: u.cost,
                            },
                        );
                    }
                }
                // unparseable JSON (partial keepalive) is ignored
            }
        }
        // Stream ended without an explicit [DONE]: same finalisation order —
        // tool calls (if any), then Done. Same argument repair as the [DONE]
        // path so a non-delta provider that never sends [DONE] is also covered.
        if !tool_acc.is_empty() || finished_tool_calls {
            sanitize_tool_acc(&mut tool_acc);
            emit(&tx, StreamEvent::ToolCalls(tool_acc.clone()));
        }
        emit(&tx, StreamEvent::Done);
        Ok(())
    }
}

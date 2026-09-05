//! Streaming chat completion over Server-Sent Events (SSE).

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::dto::chat::{ChatMessage, ToolCall};
use crate::dto::openrouter::{
    to_wire_with_images, ChatRequest, ImageWireCtx, StreamChunk, StreamOptions, ToolDef,
    ToolFunctionDef, UsageRequest,
};
use crate::model::app_config::ApiType;
use crate::service::StreamEvent;

use super::client::OpenRouterClient;
use super::helpers::{
    apply_tool_call_delta, auth_headers, backoff_delay, clean_error, emit, interactive_max_tokens,
    is_openrouter, is_retryable_send_err, is_retryable_status, provider_routing_for,
    reasoning_config, sanitize_tool_acc, wants_openrouter_usage, MAX_ATTEMPTS,
};
use super::think_split::{Emit as ThinkEmit, ThinkSplit};
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
        // Send-time OAuth refresh hook: resolve a (possibly just-refreshed) bearer
        // token + provider account/org id. Non-OAuth conns (empty `oauth_uuid`)
        // fast-path to `(api_key, "")` with zero locking.
        let (mut bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        // Prefer the manager's cached account (authoritative post-refresh); fall
        // back to whatever the route carried.
        let effective_account = if !acct.is_empty() {
            acct.as_str()
        } else {
            conn.account_id
        };

        // Codex speaks the OpenAI Responses API — a different wire protocol —
        // handled by the dedicated transport. `provider` (the OpenRouter route
        // slug) is meaningless there and ignored.
        if conn.api_type == ApiType::Codex {
            return self
                .codex_stream_complete(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    effort,
                    messages,
                    advertise,
                    mcp_tools,
                    image_ctx,
                    tx,
                )
                .await;
        }

        // Claude (Anthropic) speaks the native Messages API — a different wire
        // protocol — handled by the dedicated transport. `provider` (the OpenRouter
        // route slug) is meaningless there and ignored.
        if conn.api_type == ApiType::AnthropicCompatible {
            return self
                .anthropic_stream_complete(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    effort,
                    messages,
                    advertise,
                    mcp_tools,
                    image_ctx,
                    tx,
                )
                .await;
        }

        // Command Code speaks the `/alpha/generate` NDJSON wire — a different
        // protocol from chat-completions. `provider` is meaningless there.
        if conn.api_type == ApiType::CommandCode {
            return self
                .commandcode_stream_complete(
                    conn, &bearer, model, messages, advertise, mcp_tools, image_ctx, tx,
                )
                .await;
        }

        // The plan-word steer is now injected into the System message upstream in
        // `start_stream_task`, BEFORE the volatile project-files/awareness tail and
        // ahead of the `CACHE_SPLIT_MARK` boundary, so it stays inside the cached
        // (byte-stable) head. `to_wire` splits the System content on that mark and
        // puts the cache breakpoint on the head only.
        let url = format!("{}/chat/completions", conn.endpoint);
        // Expose the requested subset of the built-in tool set to the model. The
        // caller passes the exact tool names to advertise (`advertise`): the main
        // chat loop advertises `crate::tool::main_tool_names` (everything not in
        // `INTERNAL_ONLY`, currently `seqthink`/`plan_enter` — pushed back on mode-
        // gated instead), and each sub-agent advertises only its effective
        // allow-list. Each retained tool maps to an OpenAI/OpenRouter `function`
        // definition (name + description + raw JSON-Schema parameters).
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
            // On OpenRouter-dialect endpoints, echo captured `reasoning_details`
            // back on tool-continuation requests so a reasoning model keeps its
            // signed chain-of-thought across tool calls; stripped elsewhere (safe
            // regenerate default). Same endpoint gate `reasoning_config` uses below.
            messages: to_wire_with_images(
                messages.clone(),
                image_ctx.as_ref(),
                is_openrouter(conn.endpoint),
            ),
            stream: true,
            provider: provider_routing_for(provider),
            // OpenRouter-dialect only. Direct OpenAI hosts 400 on unknown `usage`.
            usage: if wants_openrouter_usage(&conn) {
                Some(UsageRequest { include: true })
            } else {
                None
            },
            // OpenAI-standard streaming usage for direct hosts only. Mutually
            // exclusive with OpenRouter `usage` above.
            stream_options: if wants_openrouter_usage(&conn) {
                None
            } else {
                Some(StreamOptions {
                    include_usage: true,
                })
            },
            tools: Some(tools),
            // Interactive chat is the only path that thinks; map the resolved
            // role's effort token to a `reasoning` directive (None = model default).
            reasoning: reasoning_config(effort, conn.endpoint),
            // Free-form text reply; structured output is classifier-only.
            response_format: None,
            // Runaway cap: 32k default; raised for direct xAI (see interactive_max_tokens).
            max_tokens: Some(interactive_max_tokens(conn.endpoint)),
        };

        // Opt-in request dump (KOMA_DEBUG_LLM=1).
        {
            let auth = format!("Bearer {bearer}");
            let mut hdrs: Vec<(&str, &str)> = vec![
                ("Authorization", auth.as_str()),
                ("HTTP-Referer", crate::config::HTTP_REFERER),
                ("X-Title", crate::config::APP_TITLE),
            ];
            // account header only meaningful for Kilo-style org id on OpenAI-compat.
            if conn.api_type == ApiType::OpenAiCompatible && !conn.account_id.is_empty() {
                hdrs.push(("X-Kilocode-OrganizationID", conn.account_id));
            }
            super::debug_dump::dump_outbound(&url, &hdrs, &body);
        }

        // ── 5xx / 429 retry with exponential backoff ─────────────────────
        // Track whether we've already done a 401→force-refresh→retry so we
        // don't loop infinitely on a genuinely dead token.
        let mut auth_refreshed = false;
        let resp: reqwest::Response = 'retry: {
            for attempt in 1u32..=MAX_ATTEMPTS {
                let send = auth_headers(
                    self.http.post(&url),
                    &conn,
                    &bearer,
                    self.codex_session_id(),
                )
                .json(&body)
                .send()
                .await;
                let r = match send {
                    Ok(r) => r,
                    Err(e) if is_retryable_send_err(&e) && attempt < MAX_ATTEMPTS => {
                        if let Some(ctx) = image_ctx.as_ref() {
                            crate::model::store::append_error_log(
                                &ctx.session_dir,
                                "request send failed",
                                &e.to_string(),
                            );
                        }
                        let d = backoff_delay(attempt);
                        emit(
                            &tx,
                            StreamEvent::Retrying {
                                attempt: attempt + 1,
                                max: MAX_ATTEMPTS,
                                delay_ms: d.as_millis() as u64,
                            },
                        );
                        tokio::time::sleep(d).await;
                        continue;
                    }
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

                if r.status().is_success() {
                    break 'retry r;
                }

                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                if let Some(ctx) = image_ctx.as_ref() {
                    crate::model::store::append_error_log(
                        &ctx.session_dir,
                        &format!("HTTP {status} from {} (model {model})", conn.endpoint),
                        &text,
                    );
                }

                // CommandCode 403 transport switch: permanent plan denial, NOT retried.
                if !conn.oauth_uuid.is_empty()
                    && crate::service::oauth::commandcode::is_provider_api_denied(status, &text)
                    && conn.endpoint.contains("api.commandcode.ai/provider/v1")
                {
                    crate::service::oauth::commandcode::remember_chat_pref(
                        conn.oauth_uuid,
                        crate::service::oauth::commandcode::CHAT_NDJSON,
                    );
                    let ndjson_endpoint = crate::service::oauth::registry::COMMANDCODE_CHAT_BASE;
                    let ndjson_conn = Conn {
                        endpoint: ndjson_endpoint,
                        api_key: conn.api_key,
                        api_type: ApiType::CommandCode,
                        account_id: conn.account_id,
                        oauth_uuid: conn.oauth_uuid,
                        install_id: conn.install_id,
                    };
                    return self
                        .commandcode_stream_complete(
                            ndjson_conn,
                            &bearer,
                            model,
                            messages,
                            advertise,
                            mcp_tools,
                            image_ctx,
                            tx,
                        )
                        .await;
                }

                // 401 → force-refresh + single retry: the most common cause is a
                // rotating refresh token — the OAuth keepalive daemon or another
                // session refreshed, invalidating our cached access_token. Evict
                // the stale cache entry, re-seed from disk (where the daemon
                // already wrote the new token), re-fetch fresh_key, and retry
                // exactly once. Non-OAuth conns (empty oauth_uuid) or a second
                // 401 after refresh fall through to the final error.
                if status == reqwest::StatusCode::UNAUTHORIZED
                    && !conn.oauth_uuid.is_empty()
                    && !auth_refreshed
                {
                    crate::service::oauth::manager::force_refresh(conn.oauth_uuid).await;
                    let (new_bearer, _new_acct) =
                        crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key)
                            .await;
                    if !new_bearer.is_empty() {
                        bearer = new_bearer;
                        auth_refreshed = true;
                        emit(
                            &tx,
                            StreamEvent::Retrying {
                                attempt: attempt + 1,
                                max: MAX_ATTEMPTS,
                                delay_ms: 0,
                            },
                        );
                        continue;
                    }
                }

                // Retryable + attempts remaining → sleep + retry
                if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                    let d = backoff_delay(attempt);
                    emit(
                        &tx,
                        StreamEvent::Retrying {
                            attempt: attempt + 1,
                            max: MAX_ATTEMPTS,
                            delay_ms: d.as_millis() as u64,
                        },
                    );
                    tokio::time::sleep(d).await;
                    continue;
                }

                // FINAL failure — KomaFree 429 friendly message (last attempt only)
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    && conn.api_type == ApiType::KomaFree
                {
                    emit(
                        &tx,
                        StreamEvent::Error(
                            "koma free tier is busy right now - retry in a moment, or set up a provider/custom model in /settings".to_string(),
                        ),
                    );
                    return Ok(());
                }

                // General final error
                emit(&tx, StreamEvent::Error(clean_error(status, &text)));
                return Ok(());
            }
            emit(
                &tx,
                StreamEvent::Error("all retry attempts exhausted".into()),
            );
            return Ok(());
        };

        // Command Code provider/v1 succeeded — remember so we keep hitting it.
        if !conn.oauth_uuid.is_empty() && conn.endpoint.contains("api.commandcode.ai/provider/v1") {
            crate::service::oauth::commandcode::remember_chat_pref(
                conn.oauth_uuid,
                crate::service::oauth::commandcode::CHAT_PROVIDER_V1,
            );
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        // Splits a leading <think>…</think> block out of delta.content, routing
        // it to the Reasoning channel instead of the Token channel. One instance
        // per stream call so state persists across all delta chunks of this turn.
        let mut think = ThinkSplit::new();
        // Tool calls stream across many frames, one (or more) per `index`. Each
        // frame contributes the id / name once and appends argument fragments;
        // we merge them here and emit the assembled set at finalisation.
        let mut tool_acc: Vec<ToolCall> = Vec::new();
        // Last `finish_reason` seen on the active choice. OpenAI/OpenRouter set
        // it to `"tool_calls"` on the frame that closes a tool-calling turn; we
        // record it so finalisation can confirm the model wants tools run.
        let mut finished_tool_calls = false;
        // Protocol terminal success marker (`data: [DONE]`). Soft EOF without
        // this is incomplete unless tool calls were assembled.
        let mut saw_terminal = false;
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
                let data = match line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                {
                    Some(d) => d.trim(),
                    None => continue, // comments/keepalive
                };
                if data == "[DONE]" {
                    saw_terminal = true;
                    // Flush any buffered think-tag tail (e.g. reasoning text
                    // held back waiting for a partial closer, or a partial
                    // opener that never completed).
                    for e in think.finish() {
                        match e {
                            ThinkEmit::Content(s) if !s.is_empty() => {
                                emit(&tx, StreamEvent::Token(s));
                            }
                            ThinkEmit::Reasoning(s) if !s.is_empty() => {
                                emit(&tx, StreamEvent::Reasoning(s));
                            }
                            _ => {}
                        }
                    }
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
                    // Terminal marker seen; keep emission Done (next leaf may branch).
                    debug_assert!(!stream_ended_incompletely(saw_terminal));
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
                            // Route through the think-tag splitter so a leading
                            // <think>…</think> block is emitted as Reasoning
                            // rather than Token, even when the tag is split
                            // across SSE chunks.
                            for e in think.push(t) {
                                match e {
                                    ThinkEmit::Content(s) if !s.is_empty() => {
                                        emit(&tx, StreamEvent::Token(s));
                                    }
                                    ThinkEmit::Reasoning(s) if !s.is_empty() => {
                                        emit(&tx, StreamEvent::Reasoning(s));
                                    }
                                    _ => {}
                                }
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
                        // OpenRouter also streams structured `reasoning_details`
                        // fragments (typed + signed chain-of-thought). Emit each
                        // chunk's batch so the caller can merge them by index and
                        // replay them on tool-continuation requests.
                        if let Some(details) = &choice.delta.reasoning_details {
                            if !details.is_empty() {
                                emit(&tx, StreamEvent::ReasoningDetails(details.clone()));
                            }
                        }
                        if let Some(tcs) = &choice.delta.tool_calls {
                            // Merge each streamed fragment into the accumulator.
                            // `apply_tool_call_delta` routes by id (coalescing a
                            // re-announced call), then explicit index, then the
                            // in-progress slot for an index-less continuation —
                            // robust to providers that omit `index` or re-announce
                            // an id at a new index, which the old strict "merge by
                            // index" loop turned into phantom empty-argument calls.
                            for d in tcs {
                                apply_tool_call_delta(&mut tool_acc, d);
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
        // Stream ended without an explicit [DONE]: flush the think-tag buffer
        // first (same as the [DONE] path), then either tools+Done (lenient when
        // the provider omitted [DONE] but delivered tool calls) or Error.
        for e in think.finish() {
            match e {
                ThinkEmit::Content(s) if !s.is_empty() => {
                    emit(&tx, StreamEvent::Token(s));
                }
                ThinkEmit::Reasoning(s) if !s.is_empty() => {
                    emit(&tx, StreamEvent::Reasoning(s));
                }
                _ => {}
            }
        }
        // Same argument repair as the [DONE] path so a non-delta provider that
        // never sends [DONE] is also covered.
        if !tool_acc.is_empty() || finished_tool_calls {
            sanitize_tool_acc(&mut tool_acc);
        }
        let has_tools = !tool_acc.is_empty() || finished_tool_calls;
        if soft_eof_is_complete(saw_terminal, has_tools) {
            if has_tools {
                emit(&tx, StreamEvent::ToolCalls(tool_acc.clone()));
            }
            emit(&tx, StreamEvent::Done);
        } else {
            emit(
                &tx,
                StreamEvent::Error(
                    "stream ended incompletely (connection closed before terminal marker)".into(),
                ),
            );
        }
        Ok(())
    }
}

/// Whether a stream soft-EOF lacked its protocol terminal success marker.
///
/// Each transport sets `saw_terminal` when it processes its clean close signal:
/// chat-completions `[DONE]`, Codex `response.completed`, Anthropic
/// `message_stop`, Command Code `finish`. Soft EOF with `!saw_terminal` is an
/// incomplete end unless tool calls were assembled (lenient for providers that
/// omit the terminal marker after a tool-calling turn).
pub(in crate::service::openrouter) fn stream_ended_incompletely(saw_terminal: bool) -> bool {
    !saw_terminal
}

/// Whether soft-EOF finalization should emit tools+Done rather than Error.
///
/// Complete when the transport saw its terminal success marker, or when tool
/// calls were assembled (providers sometimes drop `[DONE]`/`message_stop`/
/// `response.completed`/`finish` after a tool turn). Text-only soft EOF without
/// a terminal marker is incomplete.
pub(in crate::service::openrouter) fn soft_eof_is_complete(
    saw_terminal: bool,
    has_tools: bool,
) -> bool {
    saw_terminal || has_tools
}

#[cfg(test)]
#[path = "stream_test.rs"]
mod tests;

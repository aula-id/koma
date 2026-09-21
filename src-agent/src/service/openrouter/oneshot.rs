//! Non-streaming (one-shot) completion methods: compact, secondary-model calls,
//! and the classifier.

use anyhow::{anyhow, Result};

use super::client::OpenRouterClient;
use super::codex::to_text_format;
use super::helpers::{
    accepts_reasoning_exclude, auth_headers, backoff_delay, clean_error, is_retryable_send_err,
    is_retryable_status, provider_routing_for, wants_openrouter_usage, MAX_ATTEMPTS,
};
use super::types::Conn;
use crate::dto::chat::ChatMessage;
use crate::dto::openrouter::{to_wire, ChatRequest, ChatResponse, ReasoningConfig, UsageRequest};
use crate::model::app_config::ApiType;

/// Shared Command Code API-first fallback for oneshot paths: if `provider/v1`
/// rejects the key as Go-plan, remember NDJSON and collect via `/alpha/generate`.
/// Returns `Some(text)` when the fallback ran (Ok or Err from collect is
/// propagated); `None` when the status is not a plan denial (caller should
/// surface the original error).
async fn commandcode_oneshot_fallback(
    client: &OpenRouterClient,
    conn: Conn<'_>,
    bearer: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    status: reqwest::StatusCode,
    body: &str,
) -> Result<Option<String>> {
    if conn.oauth_uuid.is_empty()
        || !conn.endpoint.contains("api.commandcode.ai/provider/v1")
        || !crate::service::oauth::commandcode::is_provider_api_denied(status, body)
    {
        return Ok(None);
    }
    crate::service::oauth::commandcode::remember_chat_pref(
        conn.oauth_uuid,
        crate::service::oauth::commandcode::CHAT_NDJSON,
    );
    let ndjson_conn = Conn {
        endpoint: crate::service::oauth::registry::COMMANDCODE_CHAT_BASE,
        api_key: conn.api_key,
        api_type: ApiType::CommandCode,
        account_id: conn.account_id,
        oauth_uuid: conn.oauth_uuid,
        install_id: conn.install_id,
    };
    Ok(Some(
        client
            .commandcode_collect(ndjson_conn, bearer, model, messages)
            .await?,
    ))
}

fn remember_commandcode_provider_v1(conn: &Conn<'_>) {
    if !conn.oauth_uuid.is_empty() && conn.endpoint.contains("api.commandcode.ai/provider/v1") {
        crate::service::oauth::commandcode::remember_chat_pref(
            conn.oauth_uuid,
            crate::service::oauth::commandcode::CHAT_PROVIDER_V1,
        );
    }
}

impl OpenRouterClient {
    /// Non-stream completion (used by /compact). Returns assistant content.
    ///
    /// Takes its connection + model + provider-route per call (the Compactor role
    /// resolves to Main today), reusing this client's http; `provider` "" =
    /// default routing.
    pub async fn complete(
        &self,
        conn: Conn<'_>,
        model: &str,
        provider: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<String> {
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() {
            acct.as_str()
        } else {
            conn.account_id
        };
        if conn.api_type == ApiType::Codex {
            // Codex has no non-streaming endpoint: `codex_collect` drains the SSE
            // inline and returns the concatenated text. Default effort, no schema.
            return self
                .codex_collect(conn, &bearer, effective_account, model, "", messages, None)
                .await;
        }
        if conn.api_type == ApiType::AnthropicCompatible {
            // Anthropic streams-only too: `anthropic_collect` drains inline. No
            // effort/schema (plain text summary).
            return self
                .anthropic_collect(conn, &bearer, effective_account, model, "", messages, None)
                .await;
        }
        if conn.api_type == ApiType::CommandCode {
            // Command Code: drain NDJSON inline, no tools/schema.
            return self
                .commandcode_collect(conn, &bearer, model, messages)
                .await;
        }
        let url = format!("{}/chat/completions", conn.endpoint);
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages.clone()),
            stream: false,
            provider: provider_routing_for(provider),
            // OpenRouter-dialect only; direct hosts 400 on unknown `usage`.
            // Oneshot is non-stream — never send stream_options.
            usage: wants_openrouter_usage(&conn).then_some(UsageRequest { include: true }),
            stream_options: None,
            // /compact summarisation uses no tools.
            tools: None,
            // Compaction is a mechanical summary; no thinking needed.
            reasoning: None,
            // Free-form summary text; structured output is classifier-only.
            response_format: None,
            // No cap on compaction: the summary length is bounded by the prompt.
            max_tokens: None,
        };

        let response: reqwest::Response = 'retry: {
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
                match send {
                    Ok(r) => {
                        let status = r.status();
                        if status.is_success() {
                            break 'retry r;
                        }
                        let text = r.text().await.unwrap_or_default();
                        if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                            let d = backoff_delay(attempt);
                            tokio::time::sleep(d).await;
                            continue;
                        }
                        // Final attempt or non-retryable: check commandcode 403 fallback
                        if let Some(out) = commandcode_oneshot_fallback(
                            self, conn, &bearer, model, messages, status, &text,
                        )
                        .await?
                        {
                            return Ok(out);
                        }
                        return Err(anyhow!("{}", clean_error(status, &text)));
                    }
                    Err(e) if is_retryable_send_err(&e) && attempt < MAX_ATTEMPTS => {
                        let d = backoff_delay(attempt);
                        tokio::time::sleep(d).await;
                        continue;
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }
            }
            return Err(anyhow!("all retry attempts exhausted"));
        };
        remember_commandcode_provider_v1(&conn);

        let chat_response: ChatResponse = response.json().await?;
        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.unwrap_or_default())
            .ok_or_else(|| anyhow!("no choices returned"))
    }

    /// One-off non-streaming completion against a DIFFERENT model/provider on the
    /// connection `conn` (its `endpoint` + `api_key`), reusing this client's http.
    /// provider "" = default routing.
    ///
    /// Generic helper for secondary-model calls (project-awareness summaries,
    /// the `models.invoke` extension broker verb). Builds the same body
    /// `complete` does — no tools, `stream: false`, usage on — but with the
    /// caller's `model` and provider pin.
    ///
    /// `json_mode` requests OpenAI-dialect strict JSON output (top-level
    /// `response_format: {"type":"json_object"}`) — honoured ONLY on the
    /// chat-completions branch below (`ApiType::OpenAiCompatible` / `KomaFree`,
    /// which share that wire dialect). The Codex (Responses API) and
    /// Anthropic-compatible dialects have no equivalent wire field for a bare
    /// `json_object` directive, so `json_mode` is silently IGNORED (never an
    /// error) on those two branches — same "gate by which request builder runs"
    /// pattern `accepts_reasoning_exclude` uses for OpenRouter-only fields.
    ///
    /// Returns the assistant content; clean errors, no panics.
    pub async fn complete_with(
        &self,
        conn: Conn<'_>,
        model: &str,
        provider: &str,
        messages: Vec<ChatMessage>,
        json_mode: bool,
    ) -> Result<String> {
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() {
            acct.as_str()
        } else {
            conn.account_id
        };
        if conn.api_type == ApiType::Codex {
            // Default effort (→ medium), no structured-output schema. `json_mode`
            // has no Responses-API equivalent here — ignored, never errors.
            return self
                .codex_collect(conn, &bearer, effective_account, model, "", messages, None)
                .await;
        }
        if conn.api_type == ApiType::AnthropicCompatible {
            // No effort/schema (plain text reply). `json_mode` has no Anthropic
            // wire equivalent here — ignored, never errors.
            return self
                .anthropic_collect(conn, &bearer, effective_account, model, "", messages, None)
                .await;
        }
        if conn.api_type == ApiType::CommandCode {
            // Command Code: drain NDJSON inline, no tools/schema.
            return self
                .commandcode_collect(conn, &bearer, model, messages)
                .await;
        }
        let url = format!("{}/chat/completions", conn.endpoint);
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages.clone()),
            stream: false,
            provider: provider_routing_for(provider),
            // OpenRouter-dialect only; direct hosts 400 on unknown `usage`.
            usage: wants_openrouter_usage(&conn).then_some(UsageRequest { include: true }),
            stream_options: None,
            // Secondary-model calls use no tools.
            tools: None,
            // Secondary-model calls (awareness / classifier) don't think.
            reasoning: None,
            // Free-form reply by default; `json_mode` (models.invoke's
            // `format:"json"`) pins strict `{"type":"json_object"}` — otherwise
            // structured output stays classifier-only.
            response_format: json_mode.then(|| serde_json::json!({ "type": "json_object" })),
            // No cap: awareness summaries can be long.
            max_tokens: None,
        };

        let response: reqwest::Response = 'retry: {
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
                match send {
                    Ok(r) => {
                        let status = r.status();
                        if status.is_success() {
                            break 'retry r;
                        }
                        let text = r.text().await.unwrap_or_default();
                        if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                            let d = backoff_delay(attempt);
                            tokio::time::sleep(d).await;
                            continue;
                        }
                        // Final attempt or non-retryable: check commandcode 403 fallback
                        if let Some(out) = commandcode_oneshot_fallback(
                            self, conn, &bearer, model, messages, status, &text,
                        )
                        .await?
                        {
                            return Ok(out);
                        }
                        return Err(anyhow!("{}", clean_error(status, &text)));
                    }
                    Err(e) if is_retryable_send_err(&e) && attempt < MAX_ATTEMPTS => {
                        let d = backoff_delay(attempt);
                        tokio::time::sleep(d).await;
                        continue;
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }
            }
            return Err(anyhow!("all retry attempts exhausted"));
        };
        remember_commandcode_provider_v1(&conn);

        let chat_response: ChatResponse = response.json().await?;
        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.unwrap_or_default())
            .ok_or_else(|| anyhow!("no choices returned"))
    }

    /// Classifier completion against a DIFFERENT model/provider — the dedicated
    /// path for the safety harness, kept separate from [`Self::complete_with`] so
    /// the awareness summary path is unaffected.
    ///
    /// Same body as `complete_with` (no tools, `stream: false`, usage on, provider
    /// pin from `provider`) but tuned for a deterministic, fast, machine-parseable
    /// verdict:
    /// - `reasoning: {exclude: true}` (chat-completions transport, gated by
    ///   [`accepts_reasoning_exclude`]) / effort `"none"` (Codex transport) keeps
    ///   the verdict landing in `content` rather than a free-form thinking pass.
    ///   NEVER `enabled: false` — that field 400s on some non-OpenRouter upstreams,
    ///   a known landmine; `exclude: true` only HIDES reasoning (a reasoning model
    ///   like `koma/apple` still spends the tokens), it does not skip it, so
    ///   `max_tokens` must leave headroom for reasoning too (see below).
    /// - `response_format` pins a STRICT `json_schema` (`{allow, reason}`,
    ///   `additionalProperties:false`) so the model must return exactly the
    ///   verdict object as JSON. The safeguard model advertises both
    ///   `response_format` and `structured_outputs`, so this is honoured.
    ///
    /// Returns the raw reply for the caller to parse: `message.content` (the JSON
    /// string) when non-empty, else `message.reasoning` (defensive — should be
    /// empty with thinking off), else an error. The HTTP-error path returns
    /// `Err(clean_error(..))` carrying the upstream text — that reason now matters
    /// because the caller surfaces it. Clean errors, no panics.
    pub async fn classify_with(
        &self,
        conn: Conn<'_>,
        model: &str,
        provider: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<String> {
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() {
            acct.as_str()
        } else {
            conn.account_id
        };
        // Strict JSON-schema for the verdict object: exactly
        // `{"allow": <bool>, "reason": <string>}`, `additionalProperties: false`.
        // Built once, reused by both transports.
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["allow", "reason"],
            "properties": {
                "allow": { "type": "boolean" },
                "reason": { "type": "string" }
            }
        });
        if conn.api_type == ApiType::Codex {
            // Reasoning off (effort "none"); pin the verdict schema via the
            // flattened Responses `text.format`. Parsing stays in the caller.
            let raw = self
                .codex_collect(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    "none",
                    messages,
                    Some(to_text_format("verdict", schema.clone())),
                )
                .await?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("empty classifier reply"));
            }
            return Ok(trimmed.to_string());
        }
        if conn.api_type == ApiType::AnthropicCompatible {
            // Forced-tool structured output: pass the RAW verdict schema (the
            // collect driver wraps it as the `respond` tool's input_schema).
            let raw = self
                .anthropic_collect(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    "none",
                    messages,
                    Some(schema.clone()),
                )
                .await?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("empty classifier reply"));
            }
            return Ok(trimmed.to_string());
        }
        if conn.api_type == ApiType::CommandCode {
            // Command Code: plain text collect, no structured output.
            let raw = self
                .commandcode_collect(conn, &bearer, model, messages)
                .await?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("empty classifier reply"));
            }
            return Ok(trimmed.to_string());
        }
        let url = format!("{}/chat/completions", conn.endpoint);
        // `strict: true` + `additionalProperties: false` force the model to emit
        // exactly the verdict object and nothing else.
        let response_format = serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "verdict",
                "strict": true,
                "schema": schema
            }
        });
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages.clone()),
            stream: false,
            provider: provider_routing_for(provider),
            // OpenRouter-dialect only; direct hosts 400 on unknown `usage`.
            usage: wants_openrouter_usage(&conn).then_some(UsageRequest { include: true }),
            stream_options: None,
            // Classifier calls use no tools.
            tools: None,
            // `exclude: true` (strip reasoning, keep it mandatory for gateways that
            // force it) is an OpenRouter-only extension — OpenAI-native gateways 400
            // on it. `accepts_reasoning_exclude` emits the `reasoning` object for an
            // OpenRouter endpoint OR an `ApiType::KomaFree` route (koma.run is an
            // OpenRouter-style proxy fronting a reasoning model that ACCEPTS this
            // field, verified live, even though its endpoint URL isn't "openrouter");
            // elsewhere omit it and rely on the strict `response_format` JSON landing
            // in `content`.
            reasoning: accepts_reasoning_exclude(&conn).then_some(ReasoningConfig {
                effort: None,
                enabled: None,
                exclude: Some(true),
            }),
            // Force the verdict object as strict JSON.
            response_format: Some(response_format),
            // Classifier returns a tiny JSON object; cap prevents runaway. Also
            // doubles as reasoning headroom for a reasoning model behind the route
            // (e.g. koma-free's `koma/apple`, verified live): `exclude: true` only
            // hides reasoning, it still spends tokens on it (~372 tokens observed)
            // before writing the verdict JSON — a much smaller cap (e.g. 60) starves
            // it, yielding `content: null` / `finish_reason: "length"`. 2000 leaves
            // ample headroom for both.
            max_tokens: Some(2_000),
        };

        let response: reqwest::Response = 'retry: {
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
                match send {
                    Ok(r) => {
                        let status = r.status();
                        if status.is_success() {
                            break 'retry r;
                        }
                        let text = r.text().await.unwrap_or_default();
                        if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                            let d = backoff_delay(attempt);
                            tokio::time::sleep(d).await;
                            continue;
                        }
                        // Final attempt or non-retryable: check commandcode 403 fallback
                        if let Some(out) = commandcode_oneshot_fallback(
                            self, conn, &bearer, model, messages, status, &text,
                        )
                        .await?
                        {
                            let trimmed = out.trim();
                            if trimmed.is_empty() {
                                return Err(anyhow!("empty classifier reply"));
                            }
                            return Ok(trimmed.to_string());
                        }
                        return Err(anyhow!("{}", clean_error(status, &text)));
                    }
                    Err(e) if is_retryable_send_err(&e) && attempt < MAX_ATTEMPTS => {
                        let d = backoff_delay(attempt);
                        tokio::time::sleep(d).await;
                        continue;
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }
            }
            return Err(anyhow!("all retry attempts exhausted"));
        };
        remember_commandcode_provider_v1(&conn);

        let chat_response: ChatResponse = response.json().await?;
        let message = chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| anyhow!("no choices returned"))?;
        // `exclude: true` means no `reasoning` field is returned; content-only.
        // `content` may be null on some models — treat null/absent as empty.
        let content = message.content.as_deref().unwrap_or("").trim();
        if !content.is_empty() {
            return Ok(content.to_string());
        }
        Err(anyhow!("empty classifier reply"))
    }
}

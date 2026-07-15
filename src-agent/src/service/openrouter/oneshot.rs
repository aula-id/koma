//! Non-streaming (one-shot) completion methods: compact, secondary-model calls,
//! the classifier, the fold summariser, and the blob-rehydrate router.

use anyhow::{anyhow, Result};

use crate::dto::chat::{ChatMessage, Role};
use crate::dto::openrouter::{
    to_wire, ChatRequest, ChatResponse, ReasoningConfig, UsageRequest,
};
use crate::model::app_config::ApiType;
use super::codex::to_text_format;
use super::helpers::{
    accepts_reasoning_exclude, auth_headers, clean_error, parse_blob_ids, parse_summary,
    provider_routing_for,
};
use super::client::OpenRouterClient;
use super::types::Conn;

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
        let effective_account = if !acct.is_empty() { acct.as_str() } else { conn.account_id };
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
        let url = format!("{}/chat/completions", conn.endpoint);
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages),
            stream: false,
            provider: provider_routing_for(provider),
            usage: UsageRequest { include: true },
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

        let response = auth_headers(self.http.post(&url), &conn, &bearer, self.codex_session_id())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

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
        let effective_account = if !acct.is_empty() { acct.as_str() } else { conn.account_id };
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
        let url = format!("{}/chat/completions", conn.endpoint);
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages),
            stream: false,
            provider: provider_routing_for(provider),
            usage: UsageRequest { include: true },
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

        let response = auth_headers(self.http.post(&url), &conn, &bearer, self.codex_session_id())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

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
        let effective_account = if !acct.is_empty() { acct.as_str() } else { conn.account_id };
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
            messages: to_wire(messages),
            stream: false,
            provider: provider_routing_for(provider),
            usage: UsageRequest { include: true },
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

        let response = auth_headers(self.http.post(&url), &conn, &bearer, self.codex_session_id())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

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

    /// Rolling-summary "fold" completion against a DIFFERENT model/provider — the
    /// dedicated path for the short-send incremental summary (P2), kept separate
    /// from [`Self::complete_with`] so the awareness path is unaffected.
    ///
    /// Takes the fold system prompt + the pre-built user payload directly (a plain
    /// two-message request) rather than a message vec, since the caller always
    /// sends exactly system + user. Same body shape as `complete_with` (no tools,
    /// `stream: false`, usage on, provider pin from `provider`) with two critical
    /// differences:
    /// - `reasoning: {exclude: true}` keeps reasoning mandatory for endpoints that
    ///   require it, but strips the `reasoning` field from the response. The summary
    ///   is PERSISTED and replayed forever — a CoT bleed would poison the
    ///   conversation permanently. Bleed-proof, verdict lands in `content`.
    /// - `response_format` pins a STRICT `json_schema` (`{summary: string}`,
    ///   `additionalProperties: false`) so even weak/4B models must emit exactly
    ///   the summary object as JSON — never a verdict, refusal, or meta-commentary.
    ///
    /// Returns the clean summary string extracted from `{"summary": "..."}`. On
    /// parse failure or an empty `summary` field the function returns
    /// `Err(anyhow!("unparseable summary"))` — no fallback to raw content or the
    /// reasoning field (a model that ignores the schema fails-open; the caller
    /// `update_summary` already swallows the error via `let _ =`, so no summary
    /// is written that turn — acceptable). Clean errors, no panics.
    pub async fn summarize_fold(
        &self,
        conn: Conn<'_>,
        model: &str,
        provider: Option<&str>,
        system_prompt: &str,
        user_payload: &str,
    ) -> Result<String> {
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() { acct.as_str() } else { conn.account_id };
        // Strict JSON-schema for the summary object: exactly `{"summary": "<text>"}`,
        // `additionalProperties: false`. Built once, reused by both transports.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string" }
            },
            "required": ["summary"],
            "additionalProperties": false
        });
        let messages = vec![
            ChatMessage::new(Role::System, system_prompt),
            ChatMessage::new(Role::User, user_payload),
        ];
        if conn.api_type == ApiType::Codex {
            // Reasoning off (effort "none"); pin the summary schema via the
            // flattened Responses `text.format`. Shared parse tail.
            let raw = self
                .codex_collect(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    "none",
                    messages,
                    Some(to_text_format("rolling_summary", schema.clone())),
                )
                .await?;
            return parse_summary(&raw);
        }
        if conn.api_type == ApiType::AnthropicCompatible {
            // Forced-tool structured output: pass the RAW summary schema.
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
            return parse_summary(&raw);
        }
        let url = format!("{}/chat/completions", conn.endpoint);
        // `strict: true` + `additionalProperties: false` force the model to emit
        // exactly the summary object and nothing else.
        let response_format = serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "rolling_summary",
                "strict": true,
                "schema": schema
            }
        });
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages),
            stream: false,
            // `provider_routing_for` treats "" as default routing; a `None`
            // provider behaves the same (no pin).
            provider: provider_routing_for(provider.unwrap_or("")),
            usage: UsageRequest { include: true },
            stream_options: None,
            // Fold calls use no tools.
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
            // Force the summary object as strict JSON.
            response_format: Some(response_format),
            // No cap: fold summaries can be proportionally sized.
            max_tokens: None,
        };

        let response = auth_headers(self.http.post(&url), &conn, &bearer, self.codex_session_id())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

        let chat_response: ChatResponse = response.json().await?;
        let message = chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| anyhow!("no choices returned"))?;
        // Strict-JSON extraction via the shared `parse_summary` helper (identical
        // behaviour for both transports): parse `message.content` as
        // `{"summary": "..."}`. No fallback to raw content or the reasoning field —
        // a model that ignores the schema fails-open (the caller swallows the
        // error): better to skip one turn's summary than to persist garbage.
        let content = message.content.as_deref().unwrap_or("");
        parse_summary(content)
    }

    /// Blob-rehydrate router completion against a DIFFERENT model/provider — the
    /// dedicated path for the short-send retrieval router (P3), kept separate from
    /// [`Self::complete_with`] so the awareness/summary paths are unaffected.
    ///
    /// Takes the router system prompt + a pre-built user payload (the latest user
    /// message plus the candidate blob list) and returns the ids of the blobs whose
    /// full content the router judged necessary. Same body shape as `classify_with`
    /// (no tools, `stream: false`, usage on, provider pin from `provider`):
    /// - `reasoning: {enabled: false}` turns thinking OFF — deterministic, fast,
    ///   and the verdict lands in `content`. `effort` and `enabled` are mutually
    ///   exclusive — only `enabled` is set.
    /// - `response_format` pins a STRICT `json_schema` (`{blob_ids: integer[]}`)
    ///   so the model must return exactly the id list as JSON.
    ///
    /// BLEED GUARD: thinking is off and the reply is parsed as JSON only; no
    /// chain-of-thought is ever read or persisted. The returned ids merely select
    /// already-clean message content from sqlite to rehydrate.
    ///
    /// Best-effort: on ANY error (HTTP failure, empty reply, unparseable JSON) this
    /// returns `Ok(vec![])` so the caller simply rehydrates nothing rather than
    /// breaking the send. The selection is content-or-reasoning extracted, like the
    /// other secondary-model paths.
    pub async fn pick_blobs(
        &self,
        conn: Conn<'_>,
        model: &str,
        provider: &str,
        system_prompt: &str,
        user_payload: &str,
    ) -> Result<Vec<i64>> {
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() { acct.as_str() } else { conn.account_id };
        // Strict JSON-schema for the id list: exactly `{"blob_ids": [<integer>, …]}`,
        // `additionalProperties: false`. Built once, reused by both transports.
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["blob_ids"],
            "properties": {
                "blob_ids": {
                    "type": "array",
                    "items": { "type": "integer" }
                }
            }
        });
        let messages = vec![
            ChatMessage::new(Role::System, system_prompt),
            ChatMessage::new(Role::User, user_payload),
        ];
        if conn.api_type == ApiType::Codex {
            // Best-effort: on ANY Codex failure return an empty selection so the
            // caller simply rehydrates nothing. Reasoning off (effort "none").
            let raw = match self
                .codex_collect(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    "none",
                    messages,
                    Some(to_text_format("blob_selection", schema.clone())),
                )
                .await
            {
                Ok(r) => r,
                Err(_) => return Ok(Vec::new()),
            };
            return Ok(parse_blob_ids(&raw));
        }
        if conn.api_type == ApiType::AnthropicCompatible {
            // Best-effort: any failure → empty selection (rehydrate nothing).
            // Forced-tool structured output: pass the RAW blob-selection schema.
            let raw = match self
                .anthropic_collect(
                    conn,
                    &bearer,
                    effective_account,
                    model,
                    "none",
                    messages,
                    Some(schema.clone()),
                )
                .await
            {
                Ok(r) => r,
                Err(_) => return Ok(Vec::new()),
            };
            return Ok(parse_blob_ids(&raw));
        }
        let url = format!("{}/chat/completions", conn.endpoint);
        // `strict: true` + `additionalProperties: false` force the model to emit
        // exactly the id list and nothing else.
        let response_format = serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "blob_selection",
                "strict": true,
                "schema": schema
            }
        });
        let body = ChatRequest {
            model: model.to_string(),
            messages: to_wire(messages),
            stream: false,
            provider: provider_routing_for(provider),
            usage: UsageRequest { include: true },
            stream_options: None,
            // Router calls use no tools.
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
            // Force the id list as strict JSON.
            response_format: Some(response_format),
            // Picker returns a tiny JSON object; cap prevents runaway.
            max_tokens: Some(2_000),
        };

        // Best-effort: any failure returns an empty selection rather than erroring.
        let response = match auth_headers(self.http.post(&url), &conn, &bearer, self.codex_session_id())
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };

        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let chat_response: ChatResponse = match response.json().await {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        let Some(message) = chat_response.choices.into_iter().next().map(|c| c.message) else {
            return Ok(Vec::new());
        };
        // Prefer `content`; fall back to `reasoning` (some models leave `content`
        // empty/null and put the answer there even with thinking off). Either way
        // it must be the strict JSON object — we never read a CoT.
        let raw = {
            let content = message.content.as_deref().unwrap_or("").trim();
            if !content.is_empty() {
                content.to_string()
            } else {
                message
                    .reasoning
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            }
        };
        if raw.is_empty() {
            return Ok(Vec::new());
        }
        // Parse `{"blob_ids": [..]}` via the shared helper (identical behaviour for
        // both transports). Unparseable → empty.
        Ok(parse_blob_ids(&raw))
    }
}

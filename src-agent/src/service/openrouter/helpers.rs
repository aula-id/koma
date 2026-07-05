//! Private free-function helpers shared across the openrouter submodules.
//!
//! None of these are part of the public API; they exist here so the larger
//! submodules (stream, oneshot) can share them without duplication.

use anyhow::{anyhow, Result};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{APP_TITLE, HTTP_REFERER};
use crate::dto::chat::ToolCall;
use crate::dto::openrouter::ReasoningConfig;
use crate::model::app_config::ApiType;
use crate::service::StreamEvent;

use super::types::Conn;

/// Send one event on the request channel, ignoring a closed receiver (the
/// request was interrupted/superseded, so the event is simply dropped).
pub(super) fn emit(tx: &UnboundedSender<StreamEvent>, event: StreamEvent) {
    let _ = tx.send(event);
}

/// Standard auth + identity headers for chat-completions-dialect requests.
///
/// `bearer` is the (possibly refreshed) token — NOT `conn.api_key`. The caller
/// runs the [`crate::service::oauth::manager::fresh_key`] hook first and passes
/// its result here so an OAuth-backed connection always sends a live token.
/// `session_id` is the client's stable per-session id (used only by the koma-free
/// branch as the `X-Session` header; ignored for every other wire type).
///
/// Kilo OAuth conns (`OpenAiCompatible` wire + a non-empty `account_id`, which
/// carries their organization id) additionally get their organization header so
/// the gateway scopes the request. The Codex Responses transport does NOT use
/// this helper — it sends its own header set via `codex::codex_headers`.
pub(super) fn auth_headers(
    rb: reqwest::RequestBuilder,
    conn: &Conn<'_>,
    bearer: &str,
    session_id: &str,
) -> reqwest::RequestBuilder {
    auth_headers_with_account(rb, conn, bearer, None, session_id)
}

/// auth_headers with optional account_id override (e.g. from OAuth refresh).
pub(super) fn auth_headers_with_account(
    rb: reqwest::RequestBuilder,
    conn: &Conn<'_>,
    bearer: &str,
    effective_account: Option<&str>,
    session_id: &str,
) -> reqwest::RequestBuilder {
    // koma-free: keyless dual-header auth. Send the stable install id (`X-Koma`)
    // + the per-session id (`X-Session`) and NO `Authorization`/org header.
    if conn.api_type == ApiType::KomaFree {
        return rb
            .header("X-Koma", conn.install_id)
            .header("X-Session", session_id)
            .header("HTTP-Referer", HTTP_REFERER)
            .header("X-Title", APP_TITLE);
    }
    let rb = rb
        .header("Authorization", format!("Bearer {bearer}"))
        .header("HTTP-Referer", HTTP_REFERER)
        .header("X-Title", APP_TITLE);
    let account_id = effective_account.unwrap_or(conn.account_id);
    if conn.api_type == ApiType::OpenAiCompatible && !account_id.is_empty() {
        rb.header("X-Kilocode-OrganizationID", account_id)
    } else {
        rb
    }
}

/// Parse a rolling-summary reply (`{"summary": "<text>"}`) into the clean summary
/// string. Shared by the chat-completions and Codex fold transports so both parse
/// byte-identically. `Err("unparseable summary")` on non-JSON, a missing/empty
/// `summary`, or a non-string value — the caller (`update_summary`) swallows the
/// error, skipping one turn's summary rather than persisting garbage.
pub(super) fn parse_summary(raw: &str) -> Result<String> {
    let content = raw.trim();
    let parsed: serde_json::Value =
        serde_json::from_str(content).map_err(|_| anyhow!("unparseable summary"))?;
    let summary = parsed
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("unparseable summary"))?;
    Ok(summary.to_string())
}

/// Parse a blob-selection reply (`{"blob_ids": [<integer>, …]}`) into the id list.
/// Shared by the chat-completions and Codex router transports so both parse
/// byte-identically. Best-effort: an empty/non-JSON reply or a missing/ill-typed
/// `blob_ids` yields an empty vec (the caller rehydrates nothing).
pub(super) fn parse_blob_ids(raw: &str) -> Vec<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    parsed
        .get("blob_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default()
}

/// Sanitise every accumulated tool call before the assembled set leaves the client.
///
/// Three concerns are addressed in one pass:
///
/// 1. **Null-name slots** — when the SSE stream opens a tool-call slot with a
///    `null` function delta the accumulator ends up with `name: ""`. Dispatching
///    that returns `"error: unknown tool ''"` and poisons the conversation history
///    with a junk result. These slots are dropped entirely.
///
/// 2. **Duplicate-fragment arguments** — providers that re-send the FULL arguments
///    per chunk (common on budget routes) make blind delta concatenation yield
///    `{...}{...}`. [`crate::dto::chat::sanitize_tool_arguments`] collapses it to
///    one clean value so the runtime and persistence layers never see a malformed
///    string.
///
/// 3. **Empty tool-call IDs** — some providers omit the `id` field entirely. An
///    empty `tool_call_id` causes an API 400 on the next request because OpenRouter
///    requires non-empty IDs. A UUID-based fallback is generated for each such slot.
pub(super) fn sanitize_tool_acc(tool_acc: &mut Vec<ToolCall>) {
    // Drop slots where the model emitted a tool call with no name (null function
    // delta): dispatching them returns "error: unknown tool ''" and pollutes the
    // conversation with a junk result.
    tool_acc.retain(|c| !c.function.name.is_empty());

    for call in tool_acc.iter_mut() {
        // Repair arguments (duplicate-fragment collapse for budget providers).
        call.function.arguments =
            crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
        // Generate a fallback ID when the provider omits it: an empty tool_call_id
        // causes API 400 on the next request because OpenRouter requires non-empty IDs.
        if call.id.is_empty() {
            call.id = format!("call_{}", uuid::Uuid::new_v4().simple());
        }
    }
}

/// Build a provider-routing directive from a provider slug.
///
/// Returns `None` for an empty slug (OpenRouter default routing) and
/// `Some(ProviderRouting)` with `allow_fallbacks: false` otherwise, strictly
/// pinning the request to that single provider. Free helper so every request
/// path (streaming, `complete`, `complete_with`) shares one routing rule.
pub(super) fn provider_routing_for(
    provider: &str,
) -> Option<crate::dto::openrouter::ProviderRouting> {
    if provider.is_empty() {
        None
    } else {
        Some(crate::dto::openrouter::ProviderRouting {
            only: vec![provider.to_string()],
            allow_fallbacks: false,
        })
    }
}

/// True when `endpoint` speaks OpenRouter's `reasoning` dialect — i.e. accepts
/// the OpenRouter-only `enabled` / `exclude` sub-fields. OpenAI-native gateways
/// (OpenAI itself, a codex/9router gateway, etc.) reject those with
/// `400 Unknown parameter: 'reasoning.exclude'`, so we emit them ONLY here. Groq
/// & friends are reached THROUGH OpenRouter, so they match and keep working.
pub(super) fn is_openrouter(endpoint: &str) -> bool {
    endpoint.to_lowercase().contains("openrouter")
}

/// Map a stored effort token to the request `reasoning` object.
///
/// - `""` / `"default"` → `None`: omit `reasoning` entirely so the model uses
///   its own default thinking behaviour.
/// - `"off"` / `"none"` → OpenRouter: `Some(enabled: false)` (turn thinking off);
///   non-OpenRouter: `None` (the `enabled` field is an OpenRouter-only extension
///   that OpenAI-native gateways 400 on — omit `reasoning` entirely and let the
///   model use its own, often model-name-encoded, default).
/// - any effort token (`minimal`/`low`/`medium`/`high`/`xhigh`/`max`/…) →
///   `Some(effort: <token>)`. `effort` is OpenAI-standard, accepted everywhere.
///
/// Free helper (not a method) so it has no hidden state — what you pass is what
/// you get. Applied only on the interactive chat path.
pub(super) fn reasoning_config(effort: &str, endpoint: &str) -> Option<ReasoningConfig> {
    match effort.trim() {
        "" | "default" => None,
        "off" | "none" => {
            if is_openrouter(endpoint) {
                Some(ReasoningConfig {
                    effort: None,
                    enabled: Some(false),
                    exclude: None,
                })
            } else {
                None
            }
        }
        level => Some(ReasoningConfig {
            effort: Some(level.to_string()),
            enabled: None,
            exclude: None,
        }),
    }
}

/// Turn an OpenRouter error response body into a short human-readable message.
/// OpenRouter returns `{"error":{"message":..,"code":..,"metadata":{"raw":..}}}`;
/// the upstream provider's own text lives in `metadata.raw`, so prefer that, then
/// `message`, then a trimmed slice of the raw body. `status` renders as e.g.
/// "429 Too Many Requests".
pub(super) fn clean_error(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let err = &v["error"];
        let raw = err["metadata"]["raw"].as_str().unwrap_or("");
        let msg = err["message"].as_str().unwrap_or("");
        let detail = if !raw.is_empty() { raw } else { msg };
        if !detail.is_empty() {
            let detail: String = detail.chars().take(200).collect();
            return format!("{status}: {detail}");
        }
    }
    let trimmed: String = body.chars().take(160).collect();
    if trimmed.trim().is_empty() {
        format!("{status}")
    } else {
        format!("{status}: {trimmed}")
    }
}

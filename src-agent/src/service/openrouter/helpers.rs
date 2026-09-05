//! Private free-function helpers shared across the openrouter submodules.
//!
//! None of these are part of the public API; they exist here so the larger
//! submodules (stream, oneshot) can share them without duplication.

use anyhow::{anyhow, Result};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{APP_TITLE, HTTP_REFERER};
use crate::dto::chat::ToolCall;
use crate::dto::openrouter::{ReasoningConfig, ToolCallDelta};
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

/// A fresh tool-call slot, pre-tagged `kind: "function"` to match the wire shape
/// the streaming accumulator emitted before the merge was extracted here. Keeps
/// the assembled [`ToolCall`] byte-identical for the standard path.
fn new_tool_slot() -> ToolCall {
    ToolCall {
        kind: "function".into(),
        ..Default::default()
    }
}

/// Merge one streamed [`ToolCallDelta`] into the growing tool-call accumulator.
///
/// Providers disagree on how they chunk a streamed tool call. The strict
/// "merge by `index`" reconstruction OpenAI documents breaks on two dialects
/// seen on the OpenRouter chat-completions path when a reasoning model
/// interleaves thinking with tool calls:
///
/// * an argument-continuation frame that OMITS `index` (deserialised as
///   `index: None`) — strict merge misroutes its args to slot 0; and
/// * a frame that RE-ANNOUNCES an already-seen call `id` under a *new* `index`
///   — strict merge opens a second, half-empty slot.
///
/// Both manufacture a phantom `tool({})` call with empty arguments that then
/// dispatches and fails ("missing required argument"). This resolver is robust
/// to both by preferring an id match, then an explicit index, then the
/// in-progress (last) slot — while staying byte-identical to the strict path for
/// the standard case (an `index` on every frame, `id` + `name` on the first, and
/// argument-only continuations sharing that same index).
pub(super) fn apply_tool_call_delta(acc: &mut Vec<ToolCall>, d: &ToolCallDelta) {
    // Treat an empty-string id as absent so it never coalesces or clobbers.
    let id = d.id.as_deref().filter(|s| !s.is_empty());

    // 1. Resolve the slot this fragment belongs to.
    let target = if let Some(pos) = id.and_then(|id| acc.iter().position(|c| c.id == id)) {
        // (a) A slot already carries this id → coalesce onto it, whatever `index`
        //     this frame claims. Catches a provider that re-announces the same
        //     call id under a new index (strict index-merge forked a phantom).
        pos
    } else if let Some(i) = d.index {
        // (b) Standard OpenAI path: grow to the announced index and target it.
        while acc.len() <= i {
            acc.push(new_tool_slot());
        }
        i
    } else {
        // (c) No matching id and no index → an index-less continuation of the call
        //     already in progress; append to the last slot (opening one if empty).
        if acc.is_empty() {
            acc.push(new_tool_slot());
        }
        acc.len() - 1
    };

    // 2. Merge this fragment's fields into the resolved slot.
    let slot = &mut acc[target];
    if let Some(id) = id {
        slot.id = id.to_string(); // never overwrite a good id with an empty one
    }
    if let Some(f) = &d.function {
        if let Some(name) = f.name.as_deref().filter(|s| !s.is_empty()) {
            slot.function.name = name.to_string(); // never clobber a good name with empty
        }
        if let Some(args) = &f.arguments {
            slot.function.arguments.push_str(args);
        }
    }
}

/// Build a provider-routing directive from a provider slug.
///
/// Returns `None` for an empty slug (OpenRouter default routing) and
/// `Some(ProviderRouting)` with `allow_fallbacks: false` otherwise, strictly
/// pinning the request to that single provider. Free helper so every request
/// path (streaming, `complete`, `complete_with`) shares one routing rule.
///
/// Delegates the actual normalization to
/// [`crate::model::app_config::ModelEntry::normalize_route`] so both the
/// live-request pin and the persisted config self-heal identically: the
/// literal sentinel `"auto"` (any case, surrounding whitespace ignored) maps
/// to `None`, and a route already poisoned with an OpenRouter endpoint's
/// display LABEL (`"Provider | model-variant"`, e.g.
/// `"Xiaomi | xiaomi/mimo-v2.5-20260422"`) is healed down to just the
/// provider-name prefix — provider names never contain `" | "`.
pub(super) fn provider_routing_for(
    provider: &str,
) -> Option<crate::dto::openrouter::ProviderRouting> {
    let pinned = crate::model::app_config::ModelEntry::normalize_route(Some(provider.to_string()))?;
    Some(crate::dto::openrouter::ProviderRouting {
        only: vec![pinned],
        allow_fallbacks: false,
    })
}

/// True when `endpoint` speaks OpenRouter's `reasoning` dialect — i.e. accepts
/// the OpenRouter-only `enabled` / `exclude` sub-fields. OpenAI-native gateways
/// (OpenAI itself, a codex/9router gateway, etc.) reject those with
/// `400 Unknown parameter: 'reasoning.exclude'`, so we emit them ONLY here. Groq
/// & friends are reached THROUGH OpenRouter, so they match and keep working.
///
/// `pub(crate)` (not `pub(super)`) so `effort_menu`
/// (`app::runtime::commands::effort`) can use the SAME OpenRouter-vs-not
/// precedence rule as the streaming path when deciding whether to trust the
/// live `models_cache` catalogue or the curated `catalogue_overlay` for a
/// non-OpenRouter endpoint (Codex/Claude/xAI/DeepSeek).
pub(crate) fn is_openrouter(endpoint: &str) -> bool {
    endpoint.to_lowercase().contains("openrouter")
}

/// True when `endpoint` is direct xAI (`api.x.ai`). Used to scope max_tokens and
/// effort clamps that must NOT affect OpenRouter `xai/grok-*` routes or other
/// OpenAI-compatible hosts. OAuth and API-key xAI share the same body shape.
pub(crate) fn is_xai(endpoint: &str) -> bool {
    endpoint.to_lowercase().contains("api.x.ai")
}

/// Interactive completion budget for direct xAI (`api.x.ai`). ~300k-class
/// context; 256k leaves headroom without starving hidden reasoning + answer.
/// ChatGPT Codex OAuth does **not** use this — `/responses` rejects
/// `max_output_tokens` (live unsupported-parameter).
pub(crate) const XAI_INTERACTIVE_MAX_TOKENS: u32 = 256_000;

/// Interactive-path `max_tokens` runaway cap.
///
/// Direct xAI (`api.x.ai`, OAuth and API key share the body) uses
/// [`XAI_INTERACTIVE_MAX_TOKENS`]. DeepSeek/custom OpenAI-compat keep 32k.
/// Codex Responses: no output-budget field (backend 400s on it).
pub(super) fn interactive_max_tokens(endpoint: &str) -> u32 {
    if is_xai(endpoint) {
        XAI_INTERACTIVE_MAX_TOKENS
    } else {
        32_000
    }
}

/// Clamp a stored effort token for the request host.
///
/// Direct xAI overlay only advertises `low` / `high`. Tokens carried over from
/// OpenAI/Codex menus (`minimal`/`medium`/`xhigh`/`max`) are mapped so the wire
/// never sends an unsupported value. OpenRouter `xai/grok-*` is NOT clamped
/// (overlay there may allow `medium`). Empty / off / already-valid tokens pass.
pub(super) fn clamp_effort_for_endpoint(endpoint: &str, effort: &str) -> String {
    let e = effort.trim();
    if !is_xai(endpoint) {
        return e.to_string();
    }
    match e {
        "" | "default" | "off" | "none" | "low" | "high" => e.to_string(),
        "minimal" => "low".to_string(),
        // Prefer explicit high over omitting (provider default is opaque).
        "medium" | "xhigh" | "max" => "high".to_string(),
        // Unknown tokens from other providers → high (safe coding default).
        _ => "high".to_string(),
    }
}

/// True when `conn` should receive `reasoning: {exclude: true}` in a oneshot
/// request body (classifier / fold / blob-picker) — i.e. [`is_openrouter`]'s
/// endpoint-substring check, OR `conn.api_type == ApiType::KomaFree`.
///
/// koma.run is an OpenRouter-style proxy (`koma/apple` is a reasoning model
/// behind it) that ACCEPTS the OpenRouter-only `reasoning.exclude` field —
/// verified live against real requests — even though its endpoint URL
/// (`service::koma_free::KOMA_FREE_ENDPOINT`) doesn't contain "openrouter" and so
/// fails the plain [`is_openrouter`] substring check. Without this, a koma-free
/// classifier/fold/blob-picker call never asked the model to hide its reasoning,
/// and `koma/apple` burns a large, unpredictable chunk of `max_tokens` on
/// reasoning before ever writing the verdict JSON.
///
/// Deliberately NOT folded into [`is_openrouter`] itself, and deliberately NOT
/// threaded into [`reasoning_config`]: that function's `"off"/"none"` branch
/// sends `reasoning: {enabled: false}`, and `enabled: false` is a KNOWN LANDMINE
/// that 400s on some upstreams — sending it to koma.run has not been verified and
/// must not be risked. This predicate is scoped to the three `exclude: true`
/// call sites in `oneshot.rs` (`classify_with`, `summarize_fold`, `pick_blobs`),
/// never to the `enabled: false` path.
pub(super) fn accepts_reasoning_exclude(conn: &Conn<'_>) -> bool {
    is_openrouter(conn.endpoint) || conn.api_type == ApiType::KomaFree
}

/// True when chat-completions should send OpenRouter's proprietary
/// `usage: {include: true}` (tokens + cost). OpenRouter URL substring **or**
/// koma-free (same dialect, endpoint lacks `"openrouter"`).
///
/// Direct OpenAI-compatible hosts must **not** get this field — they reject
/// unknown `usage`. Streaming direct hosts use `stream_options.include_usage`
/// instead; oneshots omit both.
pub(super) fn wants_openrouter_usage(conn: &Conn<'_>) -> bool {
    is_openrouter(conn.endpoint) || conn.api_type == ApiType::KomaFree
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
    let effort = clamp_effort_for_endpoint(endpoint, effort);
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

// ── Retry / backoff for transient inference errors ──────────────────────────

/// Maximum number of attempts for a single inference request (the initial
/// attempt plus up to two retries).
pub(super) const MAX_ATTEMPTS: u32 = 3;

/// Per-retry backoff in milliseconds (index 0 = delay before 2nd attempt).
pub(super) const BACKOFF_MS: &[u64] = &[1000, 2000, 4000];

/// Upper bound for the random jitter added on top of each backoff delay.
const JITTER_MS: u64 = 500;

/// True when an HTTP status warrants a retry: server errors (5xx) and
/// rate-limit responses (429). Permanent client errors (4xx, except 429) are
/// NOT retried.
pub(super) fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// True when a reqwest send-level failure is transient (connection refused,
/// DNS lookup failure, connection reset, timeout, etc.). Builder errors and
/// redirect loops are permanent and NOT retried.
pub(super) fn is_retryable_send_err(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout()
}

/// Compute the sleep duration before the *next* attempt. `attempt` is
/// 1-based: the delay after the 1st attempt (before the 2nd) is
/// `BACKOFF_MS[0]`, etc. Includes jitter drawn from the current
/// nanosecond tick so concurrent callers don't stampede.
pub(super) fn backoff_delay(attempt: u32) -> std::time::Duration {
    let idx = ((attempt as usize) - 1).min(BACKOFF_MS.len() - 1);
    let base = BACKOFF_MS[idx];
    // Cheap jitter: fold the current instant's sub-second nanos into
    // [0, JITTER_MS). No external RNG dependency required.
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        % (JITTER_MS + 1);
    std::time::Duration::from_millis(base + jitter)
}

#[cfg(test)]
#[path = "helpers_retry_tests.rs"]
mod retry_tests;

#[cfg(test)]
#[path = "helpers_apply_tool_call_delta_tests.rs"]
mod apply_tool_call_delta_tests;

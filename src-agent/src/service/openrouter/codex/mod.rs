//! The OpenAI Responses API ("Codex") transport.
//!
//! ChatGPT-subscription connections speak a DIFFERENT protocol than the
//! OpenAI/OpenRouter chat-completions wire the rest of `openrouter` uses: a
//! `POST {endpoint}/responses` with a typed SSE event stream, subscription-OAuth
//! auth (a bearer refreshed by [`crate::service::oauth::manager`] plus a
//! `chatgpt-account-id`), stateless `store: false` requests, and encrypted
//! reasoning replayed across tool calls for chain-of-thought continuity.
//!
//! This submodule keeps that protocol wholly self-contained: the openrouter
//! `stream_complete` / oneshot dispatch branches hand off here when
//! `conn.api_type == ApiType::Codex`, and everything Codex-specific
//! (request-shaping in [`request`], SSE parsing in [`sse`], the streaming and
//! collect drivers in [`stream`] / [`oneshot`]) lives under it.
//!
//! ## `max_output_tokens` (large OAuth budget)
//!
//! Chat-completions uses `max_tokens` as a runaway guard (32k default; 256k on
//! direct xAI). Codex Responses uses the sibling field `max_output_tokens`, set
//! to [`crate::service::openrouter::helpers::OAUTH_LARGE_MAX_TOKENS`] (256k) so
//! ChatGPT OAuth gpt-5.* models (~300k context) are not starved on hidden
//! reasoning + visible answer. The terminal `response.completed` event still
//! always arrives; this is a soft ceiling, not a substitute for effort.

mod oneshot;
mod request;
mod sse;
mod stream;

// The one request-mapping helper the parent `openrouter::oneshot` dispatch needs;
// the rest of `request` stays codex-internal.
pub(in crate::service::openrouter) use request::to_text_format;

/// Auth + client-identity headers for a Codex `/responses` request.
///
/// `bearer` is the (possibly just-refreshed) subscription token — NOT
/// `conn.api_key`. The Codex backend fingerprints the official CLI, so we
/// **intentionally spoof** its `originator` / `User-Agent` (`codex_cli_rs` /
/// `codex_cli_rs/0.136.0`) — this is a working acceptance fingerprint, not a
/// branding choice. Do **not** flip to `koma` or `opencode` without a live A/B
/// proving the backend still accepts the request. `session_id` ties the request
/// to this client's stable session (also the `prompt_cache_key`).
/// `chatgpt-account-id` is sent only when known. When the access token carries
/// a real `chatgpt_compute_residency` claim (not empty / `no_constraint`), we
/// also send `x-openai-internal-codex-residency` (OpenCode protocol parity).
/// NO `HTTP-Referer` / `X-Title` (those are OpenRouter-isms the Codex backend
/// doesn't expect).
pub(super) fn codex_headers(
    rb: reqwest::RequestBuilder,
    bearer: &str,
    account_id: &str,
    session_id: &str,
) -> reqwest::RequestBuilder {
    // Fingerprint pin: official CLI identity required for backend routing.
    // Optional experiment later: KOMA_CODEX_ORIGINATOR — out of default path.
    let rb = rb
        .header("Authorization", format!("Bearer {bearer}"))
        .header("originator", "codex_cli_rs")
        .header("User-Agent", "codex_cli_rs/0.136.0")
        .header("session_id", session_id)
        .header("Accept", "text/event-stream");
    let rb = if account_id.is_empty() {
        rb
    } else {
        rb.header("chatgpt-account-id", account_id)
    };
    // Residency from the live bearer (OpenCode extracts the same claim at
    // request rewrite time). Omitted when unknown / no_constraint.
    if let Some(residency) = crate::service::oauth::jwt::codex_residency(bearer) {
        rb.header("x-openai-internal-codex-residency", residency)
    } else {
        rb
    }
}

/// Header name/value pairs as `codex_headers` would send them — for
/// [`super::debug_dump`] only (does not build a RequestBuilder).
pub(super) fn codex_header_pairs<'a>(
    bearer: &'a str,
    account_id: &'a str,
    session_id: &'a str,
) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("Authorization", format!("Bearer {bearer}")),
        ("originator", "codex_cli_rs".to_string()),
        ("User-Agent", "codex_cli_rs/0.136.0".to_string()),
        ("session_id", session_id.to_string()),
        ("Accept", "text/event-stream".to_string()),
    ];
    if !account_id.is_empty() {
        out.push(("chatgpt-account-id", account_id.to_string()));
    }
    if let Some(residency) = crate::service::oauth::jwt::codex_residency(bearer) {
        out.push(("x-openai-internal-codex-residency", residency));
    }
    out
}

/// Map a codex entitlement-error `code` to a human-readable cause. These
/// arrive IN-BAND as SSE `error` / `response.failed` events (not an HTTP 403),
/// so both [`failed_message`] and [`error_message`] check here first before
/// falling back to their generic formatting. `None` for any other code.
fn entitlement_message(code: Option<&str>) -> Option<&'static str> {
    match code {
        Some("usage_not_included") => Some(
            "codex: your ChatGPT plan does not include this model — upgrade or pick another model",
        ),
        Some("insufficient_quota") => {
            Some("codex: quota exceeded — check your ChatGPT plan/billing")
        }
        _ => None,
    }
}

/// Human-readable cause of a `response.failed` event: an entitlement error
/// (via `response.error.code`) first, else the payload's `response.error.message`,
/// else the whole payload trimmed to 200 chars, else a generic fallback. Shared
/// by the streaming + collect drivers.
pub(super) fn failed_message(response: Option<serde_json::Value>) -> String {
    if let Some(v) = response.as_ref() {
        let code = v
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_str());
        if let Some(m) = entitlement_message(code) {
            return m.to_string();
        }
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            if !msg.trim().is_empty() {
                return msg.trim().to_string();
            }
        }
        return v.to_string().chars().take(200).collect();
    }
    "codex response failed".to_string()
}

/// Format a transport-level `error` event's `message` / `code` into one string.
/// An entitlement `code` (`usage_not_included` / `insufficient_quota`) is
/// mapped to a human-readable cause; any other code keeps the previous
/// `"{code}: {message}"`-style formatting.
pub(super) fn error_message(message: Option<String>, code: Option<String>) -> String {
    if let Some(m) = entitlement_message(code.as_deref()) {
        return m.to_string();
    }
    match (message, code) {
        (Some(m), Some(c)) => format!("{c}: {m}"),
        (Some(m), None) => m,
        (None, Some(c)) => format!("codex error ({c})"),
        (None, None) => "codex stream error".to_string(),
    }
}

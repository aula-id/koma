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
//! ## No runaway `max_tokens` cap
//!
//! The chat-completions path sends a generous `max_tokens` (32k interactive, 2k
//! for the tiny classifier/router calls) as a runaway guard. The Responses API
//! has NO equivalent request field in this transport — output length is governed
//! by the reasoning effort + the model's own limits, and the terminal
//! `response.completed` event always arrives — so none is sent. Deliberate.

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
/// `conn.api_key`. The Codex backend fingerprints the official CLI, so we send
/// its `originator` / `User-Agent`; `session_id` ties the request to this
/// client's stable session (also the `prompt_cache_key`). `chatgpt-account-id`
/// is sent only when known. NO `HTTP-Referer` / `X-Title` (those are
/// OpenRouter-isms the Codex backend doesn't expect).
pub(super) fn codex_headers(
    rb: reqwest::RequestBuilder,
    bearer: &str,
    account_id: &str,
    session_id: &str,
) -> reqwest::RequestBuilder {
    let rb = rb
        .header("Authorization", format!("Bearer {bearer}"))
        .header("originator", "codex_cli_rs")
        .header("User-Agent", "codex_cli_rs/0.136.0")
        .header("session_id", session_id)
        .header("Accept", "text/event-stream");
    if account_id.is_empty() {
        rb
    } else {
        rb.header("chatgpt-account-id", account_id)
    }
}

/// Human-readable cause of a `response.failed` event: prefer the payload's
/// `response.error.message`, else the whole payload trimmed to 200 chars, else a
/// generic fallback. Shared by the streaming + collect drivers.
pub(super) fn failed_message(response: Option<serde_json::Value>) -> String {
    if let Some(v) = response.as_ref() {
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
pub(super) fn error_message(message: Option<String>, code: Option<String>) -> String {
    match (message, code) {
        (Some(m), Some(c)) => format!("{c}: {m}"),
        (Some(m), None) => m,
        (None, Some(c)) => format!("codex error ({c})"),
        (None, None) => "codex stream error".to_string(),
    }
}

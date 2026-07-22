//! The native Anthropic Messages API ("Claude") transport.
//!
//! Claude.ai-subscription OAuth connections speak a DIFFERENT protocol than the
//! OpenAI/OpenRouter chat-completions wire the rest of `openrouter` uses: a
//! `POST {endpoint}/v1/messages?beta=true` with a typed SSE event stream,
//! OAuth-bearer auth (an `sk-ant-oat…` token refreshed by
//! [`crate::service::oauth::manager`]), a mandatory Claude Code identity in the
//! `system` array, a separate top-level `system[]` (never a system *message*),
//! and strict user/assistant alternation with coalesced tool results.
//!
//! This submodule keeps that protocol wholly self-contained and MIRRORS the codex
//! layout: the openrouter `stream_complete` / oneshot dispatch branches hand off
//! here when `conn.api_type == ApiType::AnthropicCompatible`, and everything
//! Anthropic-specific (request-shaping in [`request`], SSE parsing in [`sse`], the
//! streaming and collect drivers in [`stream`] / [`oneshot`]) lives under it.
//!
//! ## Scope
//!
//! EXTENDED THINKING (adaptive, current Claude models) is requested on the
//! interactive streaming path — the `thinking` / `context_management` /
//! `output_config` body params plus the interleaved-thinking + effort betas — and
//! the model's thinking blocks are parsed for live display AND REPLAYED (with
//! their signatures) across a tool loop so continuation requests don't 400. The
//! oneshot structured-output paths (classifier/fold/router) send NO thinking: they
//! force a `respond` tool, and Anthropic deletes thinking under a forced
//! `tool_choice`. The X-Stainless / client fingerprint headers are intentionally
//! omitted. If the live backend rejects a request we add fields then.

mod oneshot;
mod request;
mod sse;
mod stream;

/// Required output-token budget for a Messages request (`max_tokens` is a REQUIRED
/// field on this API, unlike the chat-completions runaway guard which is optional).
pub(super) const CLAUDE_MAX_OUTPUT_TOKENS: u32 = 32_000;

/// The load-bearing first `system` block. Anthropic REJECTS OAuth requests whose
/// system prompt does not identify as Claude Code, so this exact string is always
/// block 0 of the `system` array, ahead of koma's own system content.
pub(super) const CLAUDE_CODE_SYSTEM: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// The minimal `anthropic-beta` set. `oauth-2025-04-20` authorizes the
/// `sk-ant-oat…` bearer; `claude-code-20250219` unlocks the Claude Code surface.
/// Always sent; the extended-thinking betas below are appended on top when a
/// request actually carries a `thinking` param.
pub(super) const CLAUDE_BETAS: &str = "oauth-2025-04-20,claude-code-20250219";

/// Extra `anthropic-beta` entries appended (after [`CLAUDE_BETAS`]) ONLY when a
/// request carries extended thinking. `interleaved-thinking-2025-05-14` lets
/// `tool_use` blocks interleave with thinking across a tool loop;
/// `effort-2025-11-24` unlocks the adaptive `output_config.effort` control. Never
/// sent on the off / forced-tool_choice / oneshot paths (which carry no thinking).
pub(super) const CLAUDE_THINKING_BETAS: &str = "interleaved-thinking-2025-05-14,effort-2025-11-24";

/// Auth + client-identity headers for an Anthropic `/v1/messages` request.
///
/// `bearer` is the (possibly just-refreshed) `sk-ant-oat…` subscription token —
/// NOT `conn.api_key`, and NEVER an `x-api-key` (OAuth only). The Claude Code
/// betas + CLI User-Agent/`x-app` identify us to the backend the same way the
/// official CLI does; a fresh `x-client-request-id` is minted per call. The
/// X-Stainless / fingerprint headers are omitted for v1.
///
/// `thinking_on` appends the extended-thinking betas ([`CLAUDE_THINKING_BETAS`]);
/// the caller sets it iff the request body carries a `thinking` param (the
/// streaming path with thinking enabled), never on the off/forced/oneshot paths.
pub(super) fn anthropic_headers(
    rb: reqwest::RequestBuilder,
    bearer: &str,
    thinking_on: bool,
) -> reqwest::RequestBuilder {
    let betas = if thinking_on {
        format!("{CLAUDE_BETAS},{CLAUDE_THINKING_BETAS}")
    } else {
        CLAUDE_BETAS.to_string()
    };
    rb.header("Authorization", format!("Bearer {bearer}"))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", betas)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("anthropic-dangerous-direct-browser-access", "true")
        .header("x-app", "cli")
        .header(
            "User-Agent",
            "claude-cli/2.1.165 (external, local-agent, agent-sdk/0.3.165)",
        )
        .header("x-client-request-id", uuid::Uuid::new_v4().to_string())
}

/// Format an SSE `error` event's `message` / `type` into one human string. Shared
/// by the streaming + collect drivers (mirrors codex's `error_message`).
pub(super) fn error_message(message: Option<String>, kind: Option<String>) -> String {
    let message = message.filter(|m| !m.trim().is_empty());
    let kind = kind.filter(|k| !k.trim().is_empty());
    match (message, kind) {
        (Some(m), Some(k)) => format!("{k}: {m}"),
        (Some(m), None) => m,
        (None, Some(k)) => format!("anthropic error ({k})"),
        (None, None) => "anthropic stream error".to_string(),
    }
}

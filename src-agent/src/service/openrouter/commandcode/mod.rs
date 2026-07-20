//! The Command Code `/alpha/generate` NDJSON transport.
//!
//! Command Code's Go-plan OAuth users speak a DIFFERENT protocol than the
//! OpenAI/OpenRouter chat-completions wire the rest of `openrouter` uses: a
//! `POST https://api.commandcode.ai/alpha/generate` with NDJSON streaming,
//! Bearer API-key auth (`user_…` long-lived key), and a typed event stream
//! (text-delta, reasoning-delta, tool-call, finish) that tolerates `data:` SSE
//! prefixes.
//!
//! This submodule keeps that protocol wholly self-contained and MIRRORS the codex
//! / anthropic layout: the openrouter `stream_complete` / oneshot dispatch
//! branches hand off here when `conn.api_type == ApiType::CommandCode`, and
//! everything CommandCode-specific (request-shaping in [`request`], NDJSON
//! parsing in [`ndjson`], the streaming and collect drivers in [`stream`] /
//! [`oneshot`]) lives under it.
//!
//! ## Scope
//!
//! - Interactive chat streaming (`commandcode_stream_complete`)
//! - One-shot collect for secondary calls (`commandcode_collect`)
//! - Tool-call support (tool definitions in Anthropic-ish `input_schema` shape;
//!   tool-calls arrive atomic per event)
//! - Reasoning support (reasoning-start/delta/end events)

mod ndjson;
mod oneshot;
mod request;
mod stream;

/// Command Code CLI version header value (matches pi-commandcode-provider).
pub(super) const CC_CLI_VERSION: &str = "0.29.0";

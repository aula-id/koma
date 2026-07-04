//! OAuth login + token-refresh service for the two OAuth-backed providers:
//! Codex (ChatGPT) and Kilo Code. Codex speaks a standard PKCE authorization-
//! code flow via a loopback redirect on `127.0.0.1:1455`; Kilo Code speaks a
//! device-authorization flow (poll for approval, no local server). Both flows
//! open the user's system browser (`browser::open_in_browser`) and hand back
//! an [`crate::model::app_config::OAuthConn`] that the caller appends to
//! `AppConfig::oauth_conns` and persists via `AppConfig::save`. Token refresh
//! for Codex is handled by `manager::fresh_key`, the single send-time hook
//! that lazily refreshes a near-expiry access token and re-persists it to
//! `config.json` before a request goes out.

pub mod browser;
pub mod codex;
pub mod jwt;
pub mod kilo;
pub mod loopback;
pub mod manager;
pub mod pkce;
pub mod registry;

use crate::model::app_config::OAuthConn;

/// Lifecycle events emitted by an in-flight `/settings` OAuth submenu connect
/// flow (Codex browser login, or Kilo Code device login). Sent across the
/// `oauth_rx` channel opened by `Action::OAuthStart`'s handler and drained once
/// per tick in `service_global`'s event loop (mirrors `StreamEvent`/`WarmEvent`).
pub enum OAuthEvent {
    /// The Codex flow reached the "open this URL" step (loopback listener is up).
    CodexUrl { url: String },
    /// The Kilo Code flow issued a device code the user must approve.
    KiloCode { user_code: String, verification_url: String },
    /// The flow completed: a ready-to-persist connection.
    Success { conn: OAuthConn },
    /// The flow failed at any stage; `error` is a human-readable reason.
    Failed { error: String },
}

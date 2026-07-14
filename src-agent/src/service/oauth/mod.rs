//! OAuth login + token-refresh service for the OAuth-backed providers: Codex
//! (ChatGPT), Kilo Code, and xAI (Grok). Codex speaks a standard PKCE
//! authorization-code flow via a loopback redirect on `127.0.0.1:1455`; Kilo
//! Code and xAI both speak a device-authorization flow (poll for approval, no
//! local server) — xAI on the standard RFC 8628 dialect with a discovered token
//! endpoint + refresh tokens (see `xai`), Kilo on its own token-less dialect.
//! Every flow opens the user's system browser (`browser::open_in_browser`) and
//! hands back an [`crate::model::app_config::OAuthConn`] that the caller appends
//! to `AppConfig::oauth_conns` and persists via `AppConfig::save`. Token refresh
//! for the expiring providers (Codex + xAI) is handled by `manager::fresh_key`,
//! the single send-time hook that lazily refreshes a near-expiry access token
//! and re-persists it to `config.json` before a request goes out.

pub mod browser;
pub mod claude;
pub mod codex;
pub mod flow;
pub mod jwt;
pub mod kilo;
pub mod komarun;
pub mod loopback;
pub mod manager;
pub mod pkce;
pub mod registry;
pub mod xai;

use crate::model::app_config::OAuthConn;

/// Lifecycle events emitted by an in-flight `/settings` OAuth submenu connect
/// flow (Codex browser login, or a Kilo Code / xAI device login). Sent across
/// the `oauth_rx` channel opened by `Action::OAuthStart`'s handler and drained
/// once per tick in `service_global`'s event loop (mirrors `StreamEvent`/`WarmEvent`).
///
/// `Success` intentionally carries the `OAuthConn` BY VALUE (not boxed): this is a rare,
/// terminal, single-shot event (one per completed login), so the marginal stack size of the
/// large variant is irrelevant, and boxing would force a `Box::new`/deref at every native
/// flow sender + the drain — churn on load-bearing native paths for no runtime benefit.
/// (W11 grew `OAuthConn` by two `Option<String>`s, nudging it past clippy's threshold.)
#[allow(clippy::large_enum_variant)]
pub enum OAuthEvent {
    /// The Codex flow reached the "open this URL" step (loopback listener is up).
    CodexUrl { url: String },
    /// A device flow issued a device code the user must approve. Reused as the
    /// generic device-code carrier for BOTH Kilo Code and xAI (Grok) — both show
    /// a `user_code` + a `verification_url`, and the downstream wait screen / GUI
    /// `waiting_code` push are identical, so no separate variant is warranted.
    KiloCode { user_code: String, verification_url: String },
    /// The flow completed: a ready-to-persist connection.
    Success { conn: OAuthConn },
    /// The flow failed at any stage; `error` is a human-readable reason.
    Failed { error: String },
}

/// A phase transition of an in-flight GUI-INITIATED OAuth flow, queued by the OAuth
/// global drain (`event_loop::global::drains::drain_oauth`) onto
/// [`crate::app::state::AppStateRest::oauth_pushes`] and turned into one
/// [`crate::ipc::proto::DaemonEvent::OAuthState`] frame — addressed to the initiating
/// push client — by the daemon hub's `drain_oauth_pushes`.
///
/// This is a PARALLEL side-channel to the drain's existing per-mode `oauth_flow` fold +
/// config persist: those run UNCHANGED (a TUI client in `Mode::Settings`/`OnboardProvider`
/// still renders the flow off its snapshot), so TUI parity is preserved. The GUI daemon
/// session sits in `Mode::Chat`, where the mode fold is a no-op — hence the webview needs
/// this dedicated push instead.
///
/// Carries ONLY the phase's display fields — NEVER a token. The connection list + provider
/// catalogue the wire event also needs are (re)built hub-side from the live `config` + the
/// provider registry at send time, so no secret ever rides this struct.
pub struct OAuthPushOut {
    /// The hub connection id of the GUI/push client that started the flow.
    pub client_id: u64,
    /// Wire phase for this transition: `"waiting_url"` (Codex browser, `url` set) |
    /// `"waiting_code"` (Kilo device, `user_code` + `verification_url` set) | `"success"` |
    /// `"failed"` (`error` set).
    pub phase: &'static str,
    /// Codex authorization URL, for `"waiting_url"`.
    pub url: Option<String>,
    /// Kilo Code device code the user approves, for `"waiting_code"`.
    pub user_code: Option<String>,
    /// Kilo Code verification URL, for `"waiting_code"`.
    pub verification_url: Option<String>,
    /// Human-readable failure reason, for `"failed"`.
    pub error: Option<String>,
}

//! Static per-provider metadata + OAuth endpoint constants. Mirrors the
//! provider registry entries in 9router's `open-sse/providers/registry/*.js`
//! (the JS reference implementation this flow was ported from) but keeps only
//! what koma's client-side flow needs: endpoints, client id/scope, and the
//! refresh-staleness windows.

use crate::dto::openrouter::ModelInfo;
use crate::model::app_config::OAuthProvider;

/// Models a ChatGPT subscription can use via the codex backend.
/// Source: opencode's ALLOWED_MODELS (plugin/openai/codex.ts) — plain base ids;
/// effort is carried in reasoning.effort, never as an id suffix. 9router's
/// -high/-review/-none ids are ITS OWN aliases and 404/entitlement-fail here.
pub const CODEX_MODELS: &[&str] = &[
    "gpt-5.6",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex-spark",
];

/// Synthesize a `GET /models`-shaped catalogue from [`CODEX_MODELS`] so the
/// EXISTING omnisearch machinery (`filter_models` + the model-modal renderer)
/// serves Codex's static list identically to a fetched network catalogue — no
/// separate filtering/rendering path needed. Cheap to rebuild (~20 entries);
/// called fresh wherever it's needed rather than cached.
pub fn codex_static_catalogue() -> Vec<ModelInfo> {
    CODEX_MODELS
        .iter()
        .map(|id| ModelInfo {
            id: id.to_string(),
            name: None,
            supported_parameters: Vec::new(),
            reasoning: None,
            context_length: None,
            top_provider: None,
            pricing: None,
            architecture: None,
        })
        .collect()
}

// --- Codex OAuth ---

pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CODEX_SCOPE: &str = "openid profile email offline_access";
pub const CODEX_REDIRECT: &str = "http://localhost:1455/auth/callback";
pub const CODEX_PORT: u16 = 1455;

/// How long before actual expiry to proactively refresh (5 days), matching
/// 9router's `refreshLeadMs: 432000000`.
pub const CODEX_REFRESH_LEAD_SECS: u64 = 432_000;
/// Beyond this age since the last successful refresh, treat the token as too
/// stale to keep retrying silently (8 days), matching 9router's
/// `maxRefreshAgeMs: 691200000`.
pub const CODEX_MAX_REFRESH_AGE_SECS: u64 = 691_200;

// --- Kilo Code OAuth ---

pub const KILO_DEVICE_URL: &str = "https://api.kilo.ai/api/device-auth/codes";
pub const KILO_PROFILE_URL: &str = "https://api.kilo.ai/api/profile";

// --- xAI (Grok) OAuth ---

/// xAI's public device-flow client id (no secret).
pub const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
/// OIDC discovery document. The `token_endpoint` used for polling + refresh is
/// read from here FRESH on every login/refresh and validated to an `x.ai` host
/// (see `xai::discover_token_endpoint`) — never hardcoded, never cached.
pub const XAI_DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
/// Device-authorization endpoint (RFC 8628): issues the device + user codes.
pub const XAI_DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";
pub const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

/// Refresh an xAI access token this many seconds before it expires (5 min). xAI
/// access tokens are short-lived, so unlike codex's multi-day lead this is a
/// tight skew; it is applied in `manager::is_stale` (the "5-minute skew" the
/// token stamp itself never bakes in, keeping login + refresh `expires_at`
/// identical).
pub const XAI_REFRESH_LEAD_SECS: u64 = 300;
/// xAI refresh tokens (`offline_access`) are long-lived; `0` DISABLES the
/// "too stale to keep retrying" age cap codex uses, so a still-valid refresh
/// token keeps working no matter how long since the last successful refresh.
pub const XAI_MAX_REFRESH_AGE_SECS: u64 = 0;

// --- Claude (Anthropic) OAuth ---

pub const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const CLAUDE_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
pub const CLAUDE_SCOPE: &str =
    "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
pub const CLAUDE_REDIRECT: &str = "http://localhost:54545/callback";
pub const CLAUDE_PORT: u16 = 54545;

/// Same lead/age windows as Codex (see `CODEX_REFRESH_LEAD_SECS`/`CODEX_MAX_REFRESH_AGE_SECS`).
pub const CLAUDE_REFRESH_LEAD_SECS: u64 = CODEX_REFRESH_LEAD_SECS;
pub const CLAUDE_MAX_REFRESH_AGE_SECS: u64 = CODEX_MAX_REFRESH_AGE_SECS;

// --- Koma (koma.run) OAuth ---

/// koma.run's native-client OAuth: RFC 8252 loopback + PKCE S256, no `client_id`
/// (the client is identified purely by PKCE + loopback redirect) and no `scope`.
pub const KOMA_AUTHORIZE_URL: &str = "https://koma.run/api/v1/auth/oauth/authorize";
pub const KOMA_TOKEN_URL: &str = "https://koma.run/api/v1/auth/oauth/token";
/// Not called yet — no logout/revoke UI wires this flow's connection deletion
/// through to the server; `AppConfig::oauth_conns` removal is purely local. Kept
/// for the future logout affordance.
#[allow(dead_code)]
pub const KOMA_REVOKE_URL: &str = "https://koma.run/api/v1/auth/oauth/revoke";
pub const KOMA_REDIRECT: &str = "http://127.0.0.1:51004/cb";
pub const KOMA_PORT: u16 = 51004;

/// Refresh lead / max-refresh-age windows for koma.run's rotating refresh token.
/// koma.run access tokens are short-lived (24h, see the token response's
/// `expires_in`), so a tighter lead than Codex's multi-day one (5 min, matching
/// xAI's window).
pub const KOMA_REFRESH_LEAD_SECS: u64 = 300;
/// koma.run's refresh token itself expires in 30 days (`refresh_expires_in`);
/// cap silent retries at 20 hours since the last successful refresh, matching
/// Codex's style of a conservative fraction of that window.
pub const KOMA_MAX_REFRESH_AGE_SECS: u64 = 20 * 3_600;

// --- Extension-delegated model providers (W12) ---

/// Refresh an ext-backed access token this many seconds before it expires (5 min, matching
/// xAI/koma's tight skew). koma never bakes provider-specific windows for extension tokens —
/// their lifecycle is data-driven — so a single generic lead is used; whether a refresh
/// actually fires is additionally gated on the conn carrying a `refresh_token_url` (see
/// `manager::fresh_key`'s Extension arm). The age cap is disabled (`0`) since koma has no
/// per-provider knowledge of an extension token's maximum silent-retry window.
pub const EXT_REFRESH_LEAD_SECS: u64 = 300;

/// Per-provider metadata needed to wire an [`OAuthConn`](crate::model::app_config::OAuthConn)
/// into the chat-request resolution boundary.
pub struct OAuthProviderMeta {
    /// Base chat-completions endpoint, stashed on `Resolved::endpoint` once
    /// resolution wires OAuth connections in (a later wave).
    pub chat_endpoint: &'static str,
    /// Model-catalogue endpoint for on-demand fetch; empty string means the
    /// provider has no network catalogue (use a static list instead).
    pub catalogue_endpoint: &'static str,
}

/// The available OAuth login providers as DATA — the SINGLE source of truth the GUI's
/// `GetOAuthState` reply (`DaemonEvent::OAuthState.providers`) is built from, so adding a
/// new provider (a future xAI / Claude) surfaces in the webview by extending THIS list,
/// never by editing a wire builder. Each tuple is `(id, label, kind)`: `id` is the
/// `StartOAuth` wire token, `label` the human name, `kind` the flow shape (`"pkce"` /
/// `"device"` / `"paste"`).
///
/// Derived from the [`OAuthProvider`] enum (each variant's `wire_id`/`label`/`flow_kind`),
/// plus the Codex paste-token option — a third login choice that reuses Codex's connection
/// shape and so has no enum variant of its own. The TUI's hardcoded picker OPTIONS array
/// (`app::mode::settings` / `app::mode::onboard`) stays as-is this wave; folding it onto
/// this same source is a future dedup.
pub fn oauth_providers() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut providers: Vec<(&'static str, &'static str, &'static str)> = [
        OAuthProvider::Codex,
        OAuthProvider::Kilocode,
        OAuthProvider::Xai,
        OAuthProvider::ClaudeAI,
        OAuthProvider::KomaRun,
        OAuthProvider::KomaPremium,
    ]
    .iter()
    .map(|p| (p.wire_id(), p.label(), p.flow_kind()))
    .collect();
    providers.push(("codex_paste", "Codex paste", "paste"));
    providers
}

/// Static metadata for `p`.
pub fn meta(p: OAuthProvider) -> OAuthProviderMeta {
    match p {
        OAuthProvider::Codex => OAuthProviderMeta {
            chat_endpoint: "https://chatgpt.com/backend-api/codex",
            catalogue_endpoint: "",
        },
        OAuthProvider::Kilocode => OAuthProviderMeta {
            chat_endpoint: "https://api.kilo.ai/api/openrouter",
            catalogue_endpoint: "https://api.kilo.ai/api/gateway",
        },
        // xAI is OpenAI-compatible: chat rides `{endpoint}/chat/completions` and the
        // catalogue is `{endpoint}/models` — the same base URL serves both (the client
        // appends the path), landing on `https://api.x.ai/v1/{chat/completions,models}`.
        OAuthProvider::Xai => OAuthProviderMeta {
            chat_endpoint: "https://api.x.ai/v1",
            catalogue_endpoint: "https://api.x.ai/v1",
        },
        OAuthProvider::ClaudeAI => OAuthProviderMeta {
            chat_endpoint: "https://api.anthropic.com",
            catalogue_endpoint: "",
        },
        // account login; not a model provider yet — placeholders until a future
        // extension wires koma.run as an actual chat/catalogue backend.
        OAuthProvider::KomaRun => OAuthProviderMeta {
            chat_endpoint: "https://koma.run/api/v1",
            catalogue_endpoint: "",
        },
        // Koma Premium (koma/peach): uses the same OAuth tokens as KomaRun but routes
        // to the premium endpoint for paid subscribers.
        OAuthProvider::KomaPremium => OAuthProviderMeta {
            chat_endpoint: "https://koma.run/api/v1/koma-premium",
            catalogue_endpoint: "",
        },
        // W12: extension-backed conns are resolved DATA-DRIVEN from the conn's OWN stored
        // meta (endpoint captured at login from the manifest `OAuthProviderDef.chat_endpoint`
        // — see `app::resolve::ext_conn_route` / `OAuthConn::ext_model_route`), NOT this
        // static table, which resolution bypasses for `Extension` conns. These stay empty
        // placeholders (only reached by any non-resolution `meta()` caller); the empty
        // `catalogue_endpoint` also means the OAuth success drain never fires a catalogue
        // fetch for an ext conn.
        OAuthProvider::Extension => OAuthProviderMeta {
            chat_endpoint: "",
            catalogue_endpoint: "",
        },
    }
}

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
    }
}

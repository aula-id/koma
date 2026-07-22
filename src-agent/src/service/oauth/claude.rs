//! The Claude (Anthropic) OAuth flow: PKCE authorization-code grant against
//! `claude.ai`/`api.anthropic.com`, via a loopback callback on port 54545.
//! Mirrors `codex.rs`'s flow shape (same PKCE dance, same loopback listener)
//! but speaks Anthropic's own token endpoint and response shape.

use std::time::{SystemTime, UNIX_EPOCH};

use super::jwt;
use super::pkce::{self, Pkce};
use super::registry::{
    CLAUDE_AUTHORIZE_URL, CLAUDE_CLIENT_ID, CLAUDE_REDIRECT, CLAUDE_SCOPE, CLAUDE_TOKEN_URL,
};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

/// A ready-to-open authorization URL plus the PKCE material needed to
/// complete the exchange once the redirect comes back.
pub struct ClaudeAuthUrl {
    pub url: String,
    pub pkce: Pkce,
}

/// Build the Claude (Anthropic) authorization URL. Query params are hand-rolled
/// (same percent-encoding approach as `codex::build_auth_url`) and emitted in a
/// fixed order, including the unusual `code=true` leading param the Anthropic
/// CLI flow sends.
pub fn build_auth_url() -> ClaudeAuthUrl {
    let pkce = pkce::generate();

    let params: [(&str, &str); 8] = [
        ("code", "true"),
        ("client_id", CLAUDE_CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", CLAUDE_REDIRECT),
        ("scope", CLAUDE_SCOPE),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", &pkce.state),
    ];

    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    ClaudeAuthUrl {
        url: format!("{CLAUDE_AUTHORIZE_URL}?{query}"),
        pkce,
    }
}

/// Percent-encode `s` for a query-string value: unreserved characters
/// (`ALPHA` / `DIGIT` / `-` `.` `_` `~`) pass through; everything else
/// (including space) becomes an uppercase `%XX` escape. Duplicated from
/// `codex::percent_encode` (private there) rather than shared, matching how
/// `xai.rs` also stays self-contained.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Anthropic account identity embedded in the token-exchange response.
#[derive(Debug, serde::Deserialize)]
pub struct ClaudeAccount {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub email_address: String,
}

/// Token-exchange response shape from Anthropic's `/v1/oauth/token` endpoint.
/// Richer than the shared `codex::TokenResponse` (carries an `account` object),
/// so this flow defines its own and only projects the shared fields into
/// `codex::TokenResponse` at the `refresh` boundary the manager needs.
#[derive(Debug, serde::Deserialize)]
pub struct ClaudeTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub account: Option<ClaudeAccount>,
}

/// Exchange an authorization `code` for tokens. The callback's `code` query
/// param may arrive in `code#state` form (Anthropic's flow embeds the state
/// after a `#`, which the loopback parser does NOT strip); split it here
/// before exchanging, and let a non-empty embedded state override the one
/// passed in. JSON body (NOT form-encoded), per Anthropic's token endpoint.
pub async fn exchange_code(
    http: &reqwest::Client,
    code: &str,
    verifier: &str,
    state: &str,
) -> Result<ClaudeTokenResponse, String> {
    let (exchange_code, exchange_state) = match code.split_once('#') {
        Some((c, s)) if !s.is_empty() => (c, s),
        Some((c, _)) => (c, state),
        None => (code, state),
    };

    let resp = http
        .post(CLAUDE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": CLAUDE_CLIENT_ID,
            "code": exchange_code,
            "state": exchange_state,
            "redirect_uri": CLAUDE_REDIRECT,
            "code_verifier": verifier,
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "token exchange failed ({status}): {}",
            truncate(&body, 200)
        ));
    }

    resp.json::<ClaudeTokenResponse>()
        .await
        .map_err(|e| format!("token exchange response parse failed: {e}"))
}

/// Refresh an access token. Returns the SHARED `codex::TokenResponse` shape (flat
/// access_token/refresh_token/id_token/expires_in) — the manager's persist/update
/// path only needs those fields, and Anthropic's extra `account` object (absent
/// from a refresh response anyway) is simply not part of that shape.
pub async fn refresh(
    http: &reqwest::Client,
    refresh_token: &str,
) -> Result<super::codex::TokenResponse, String> {
    let resp = http
        .post(CLAUDE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(
            "User-Agent",
            "anthropic-sdk-typescript/0.94.0 userOAuthProvider",
        )
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CLAUDE_CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|e| format!("refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if body.contains("invalid_grant") || body.contains("refresh_token_reused") {
            return Err("unrecoverable: re-login required".to_string());
        }
        return Err(format!(
            "refresh failed ({status}): {}",
            truncate(&body, 200)
        ));
    }

    resp.json::<super::codex::TokenResponse>()
        .await
        .map_err(|e| format!("refresh response parse failed: {e}"))
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max).collect::<String>())
    }
}

/// Assemble an [`OAuthConn`] from a completed Claude token exchange.
pub fn to_conn(tokens: ClaudeTokenResponse) -> OAuthConn {
    let expires_at = tokens
        .expires_in
        .map(|secs| now_secs() + secs)
        .unwrap_or_else(|| jwt::expiry(&tokens.access_token));

    let account_id = tokens
        .account
        .as_ref()
        .map(|a| a.uuid.clone())
        .unwrap_or_default();
    let mut email = tokens
        .account
        .as_ref()
        .map(|a| a.email_address.clone())
        .unwrap_or_default();
    if email.is_empty() {
        email = jwt::decode_payload(&tokens.access_token)
            .and_then(|v| {
                v.get("email")
                    .and_then(|e| e.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
    }

    let label = if !email.is_empty() {
        email.clone()
    } else if !account_id.is_empty() {
        account_id.chars().take(8).collect()
    } else {
        "unknown".to_string()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("claude ({label})"),
        provider: OAuthProvider::ClaudeAI,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: String::new(),
        expires_at,
        last_refresh: now_secs(),
        account_id,
        org_id: String::new(),
        email,
        plan: String::new(),
        // Native flow — never extension-backed (W11/W12 ext fields stay None; omitted from JSON).
        ext_id: None,
        provider_id: None,
        chat_endpoint: None,
        api_type: None,
        refresh_token_url: None,
        refresh_client_id: None,
        commandcode_chat: None,
    }
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

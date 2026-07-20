//! The Codex (ChatGPT) OAuth flow: PKCE authorization-code grant against
//! `auth.openai.com`, plus the token-exchange and refresh calls against
//! `CODEX_TOKEN_URL`.

use std::time::{SystemTime, UNIX_EPOCH};

use super::jwt;
use super::pkce::{self, Pkce};
use super::registry::{
    CODEX_AUTHORIZE_URL, CODEX_CLIENT_ID, CODEX_REDIRECT, CODEX_SCOPE, CODEX_TOKEN_URL,
};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

/// A ready-to-open authorization URL plus the PKCE material needed to
/// complete the exchange once the redirect comes back.
pub struct CodexAuthUrl {
    pub url: String,
    pub pkce: Pkce,
}

/// Build the Codex authorization URL.
///
/// The query string is hand-rolled rather than built with a URL-encoding
/// crate: the codex backend rejects a `+`-encoded space in `scope` (it wants
/// literal `%20`), which is exactly the encoding most `application/
/// x-www-form-urlencoded`-flavoured encoders default to for query strings.
pub fn build_auth_url() -> CodexAuthUrl {
    let pkce = pkce::generate();

    let params: [(&str, &str); 9] = [
        ("response_type", "code"),
        ("client_id", CODEX_CLIENT_ID),
        ("redirect_uri", CODEX_REDIRECT),
        ("scope", CODEX_SCOPE),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", &pkce.state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
    ];

    let mut query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    query.push_str("&originator=codex_cli_rs");

    CodexAuthUrl {
        url: format!("{CODEX_AUTHORIZE_URL}?{query}"),
        pkce,
    }
}

/// Percent-encode `s` for a query-string value: unreserved characters
/// (`ALPHA` / `DIGIT` / `-` `.` `_` `~`) pass through; everything else
/// (including space) becomes an uppercase `%XX` escape.
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

/// Token-endpoint response shape shared by both the exchange and refresh
/// calls. Optional/omittable fields default so a response missing one (e.g.
/// a refresh that doesn't rotate `refresh_token`) still deserializes.
#[derive(Debug, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// Exchange an authorization `code` for tokens. Form-encoded POST, per the
/// codex token endpoint's expected content type for this grant.
pub async fn exchange_code(
    http: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse, String> {
    let resp = http
        .post(CODEX_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", CODEX_REDIRECT),
            ("client_id", CODEX_CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed ({status}): {}", truncate(&body, 200)));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| format!("token exchange response parse failed: {e}"))
}

/// Refresh an access token. Unlike [`exchange_code`], the codex refresh call
/// wants a JSON body, not form-encoded.
pub async fn refresh(http: &reqwest::Client, refresh_token: &str) -> Result<TokenResponse, String> {
    let resp = http
        .post(CODEX_TOKEN_URL)
        .json(&serde_json::json!({
            "client_id": CODEX_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": CODEX_SCOPE,
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
        return Err(format!("refresh failed ({status}): {}", truncate(&body, 200)));
    }

    resp.json::<TokenResponse>()
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

/// Assemble an [`OAuthConn`] from a completed token exchange (or a
/// hand-pasted raw JWT, in which case `refresh_token`/`id_token` are empty).
pub fn to_conn(tokens: TokenResponse) -> OAuthConn {
    let identity = jwt::codex_identity(&tokens.id_token, &tokens.access_token);

    let expires_at = tokens
        .expires_in
        .map(|secs| now_secs() + secs)
        .unwrap_or_else(|| jwt::expiry(&tokens.access_token));

    let label = if !identity.email.is_empty() {
        identity.email.clone()
    } else if !identity.account_id.is_empty() {
        identity.account_id.chars().take(8).collect()
    } else {
        "unknown".to_string()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("codex ({label})"),
        provider: OAuthProvider::Codex,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens.id_token,
        expires_at,
        last_refresh: now_secs(),
        account_id: identity.account_id,
        org_id: String::new(),
        email: identity.email,
        plan: identity.plan,
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

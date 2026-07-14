//! The Koma (koma.run) account OAuth flow: PKCE authorization-code grant
//! against koma.run's native-client OAuth, via a loopback callback on port
//! 51004. Cloned from `claude.rs`'s flow shape (same PKCE dance, same
//! loopback listener), but koma.run's contract differs in three ways: token
//! exchange is form-encoded (not JSON), there is no `client_id`/`scope` (the
//! native flow identifies the client purely by PKCE + loopback redirect), and
//! the callback `code` is used verbatim (no `code#state` split — that was an
//! Anthropic-specific quirk in `claude.rs`).

use std::time::{SystemTime, UNIX_EPOCH};

use super::jwt;
use super::pkce::{self, Pkce};
use super::registry::{KOMA_AUTHORIZE_URL, KOMA_REDIRECT, KOMA_TOKEN_URL};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

/// A ready-to-open authorization URL plus the PKCE material needed to
/// complete the exchange once the redirect comes back.
pub struct KomaAuthUrl {
    pub url: String,
    pub pkce: Pkce,
}

/// Build the Koma (koma.run) authorization URL. Query params are hand-rolled
/// (same percent-encoding approach as `claude::build_auth_url`); NO
/// `client_id`, NO `scope` — the native loopback flow is identified purely by
/// PKCE + redirect_uri.
pub fn build_auth_url() -> KomaAuthUrl {
    let pkce = pkce::generate();

    let params: [(&str, &str); 4] = [
        ("redirect_uri", KOMA_REDIRECT),
        ("state", &pkce.state),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
    ];

    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    KomaAuthUrl {
        url: format!("{KOMA_AUTHORIZE_URL}?{query}"),
        pkce,
    }
}

/// Percent-encode `s` for a query-string value: unreserved characters
/// (`ALPHA` / `DIGIT` / `-` `.` `_` `~`) pass through; everything else
/// (including space) becomes an uppercase `%XX` escape. Duplicated from
/// `claude::percent_encode` (private there) rather than shared, matching how
/// every other flow in this module stays self-contained.
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

/// Exchange an authorization `code` for tokens. Form-encoded POST (NOT JSON,
/// unlike Anthropic's endpoint), using the `code` verbatim — koma.run has no
/// `code#state` quirk, so `state` is accepted but not threaded into the body.
/// Reuses the shared `codex::TokenResponse` shape for parsing (koma.run's
/// response has no `id_token`, which the shape already defaults to empty).
pub async fn exchange_code(
    http: &reqwest::Client,
    code: &str,
    verifier: &str,
    _state: &str,
) -> Result<super::codex::TokenResponse, String> {
    let resp = http
        .post(KOMA_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", KOMA_REDIRECT),
        ])
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed ({status}): {}", truncate(&body, 200)));
    }

    resp.json::<super::codex::TokenResponse>()
        .await
        .map_err(|e| format!("token exchange response parse failed: {e}"))
}

/// Refresh an access token. Form-encoded POST, grant_type=refresh_token. On
/// `invalid_grant` (koma.run's rotating refresh token was already used, or is
/// no longer valid) treat it as unrecoverable — mirrors `claude::refresh`'s
/// handling.
pub async fn refresh(
    http: &reqwest::Client,
    refresh_token: &str,
) -> Result<super::codex::TokenResponse, String> {
    let resp = http
        .post(KOMA_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|e| format!("refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if body.contains("invalid_grant") {
            return Err("unrecoverable: re-login required".to_string());
        }
        return Err(format!("refresh failed ({status}): {}", truncate(&body, 200)));
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

/// Assemble an [`OAuthConn`] from a completed Koma token exchange. Best-effort
/// decodes the access-token JWT payload for `email`, mirroring
/// `claude::to_conn`'s fallback (koma.run's response carries no separate
/// account object).
pub fn to_conn(tokens: super::codex::TokenResponse) -> OAuthConn {
    let expires_at = tokens
        .expires_in
        .map(|secs| now_secs() + secs)
        .unwrap_or_else(|| jwt::expiry(&tokens.access_token));

    let email = jwt::decode_payload(&tokens.access_token)
        .and_then(|v| v.get("email").and_then(|e| e.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();

    let label = if !email.is_empty() {
        email.clone()
    } else {
        "unknown".to_string()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("koma ({label})"),
        provider: OAuthProvider::KomaRun,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: String::new(),
        expires_at,
        last_refresh: now_secs(),
        account_id: String::new(),
        org_id: String::new(),
        email,
        plan: String::new(),
        // Native flow — never extension-backed (W11 fields stay None; omitted from JSON).
        ext_id: None,
        provider_id: None,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

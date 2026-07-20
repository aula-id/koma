//! ClinePass OAuth: WorkOS credential reuse from the Cline CLI's
//! `~/.cline/data/settings/providers.json` (and optionally
//! `~/.pi/agent/auth.json`), plus a custom refresh endpoint. Also supports
//! static API key paste (no refresh). Flow kind: "reuse" — no browser loopback,
//! no device code; the login either succeeds from cached creds or fails with a
//! message directing the user to paste.

use std::time::{SystemTime, UNIX_EPOCH};

use super::registry::{
    CLINE_API_BASE, CLINE_REFRESH_LEAD_SECS, CLINE_REFRESH_PATH, CLINE_WORKOS_TOKEN_LIFETIME_SECS,
    CLINE_WORKOS_TOKEN_PREFIX,
};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

/// Whether `s` starts with the WorkOS token prefix (`"workos:"`).
pub fn is_workos_token(s: &str) -> bool {
    s.starts_with(CLINE_WORKOS_TOKEN_PREFIX)
}

/// Build an [`OAuthConn`] from a WorkOS access+refresh token pair (from the
/// Cline CLI credential reuse). `expires_at` is a unix-seconds timestamp.
pub fn to_conn_workos(
    access: String,
    refresh: String,
    expires_at_secs: u64,
    email: String,
) -> OAuthConn {
    OAuthConn {
        uuid: new_uuid(),
        name: if !email.is_empty() {
            format!("clinepass ({email})")
        } else {
            "clinepass (workos)".to_string()
        },
        provider: OAuthProvider::ClinePass,
        access_token: access,
        refresh_token: refresh,
        id_token: String::new(),
        expires_at: expires_at_secs,
        last_refresh: now_secs(),
        account_id: String::new(),
        org_id: String::new(),
        email,
        plan: String::new(),
        ext_id: None,
        provider_id: None,
        chat_endpoint: None,
        api_type: None,
        refresh_token_url: None,
        refresh_client_id: None,
    }
}

/// Build an [`OAuthConn`] from a static API key (paste path). No refresh, no
/// expiry (`expires_at = 0`).
pub fn to_conn_api_key(key: &str) -> OAuthConn {
    OAuthConn {
        uuid: new_uuid(),
        name: "clinepass (api key)".to_string(),
        provider: OAuthProvider::ClinePass,
        access_token: key.to_string(),
        refresh_token: key.to_string(),
        id_token: String::new(),
        expires_at: 0,
        last_refresh: 0,
        account_id: String::new(),
        org_id: String::new(),
        email: String::new(),
        plan: String::new(),
        ext_id: None,
        provider_id: None,
        chat_endpoint: None,
        api_type: None,
        refresh_token_url: None,
        refresh_client_id: None,
    }
}

/// Refresh a WorkOS access token via ClinePass's custom refresh endpoint.
/// POST JSON `{ granttype: "refresh_token", refreshToken: "..." }`. Returns
/// the shared [`super::codex::TokenResponse`] shape so the manager's
/// persist/update path stays provider-agnostic.
pub async fn refresh(
    http: &reqwest::Client,
    refresh_token: &str,
) -> Result<super::codex::TokenResponse, String> {
    let url = format!("{CLINE_API_BASE}{CLINE_REFRESH_PATH}");
    let resp = http
        .post(&url)
        .json(&serde_json::json!({
            "granttype": "refresh_token",
            "refreshToken": refresh_token,
        }))
        .send()
        .await
        .map_err(|e| format!("clinepass refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if body.contains("invalid_grant") || body.contains("refresh_token_reused") {
            return Err("unrecoverable: re-login required".to_string());
        }
        return Err(format!(
            "clinepass refresh failed ({status}): {}",
            truncate(&body, 200)
        ));
    }

    // Response may be `{ data: { accessToken, refreshToken } }` or flat.
    let raw: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("clinepass refresh parse failed: {e}"))?;

    let data = raw.get("data").unwrap_or(&raw);

    let access_token = data
        .get("accessToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let refresh_token_new = data
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if access_token.is_empty() {
        return Err("clinepass refresh returned empty accessToken".to_string());
    }

    // Ensure the workos: prefix is present.
    let access_token = if is_workos_token(&access_token) {
        access_token
    } else {
        format!("{CLINE_WORKOS_TOKEN_PREFIX}{access_token}")
    };

    Ok(super::codex::TokenResponse {
        access_token,
        refresh_token: if refresh_token_new.is_empty() {
            refresh_token.to_string()
        } else {
            refresh_token_new.to_string()
        },
        id_token: String::new(),
        expires_in: Some(CLINE_WORKOS_TOKEN_LIFETIME_SECS - CLINE_REFRESH_LEAD_SECS),
    })
}

/// Try to resolve WorkOS credentials from disk (Cline CLI's credential files).
/// Returns `Some((access, refresh, expires_at_secs, email))` or `None` if no
/// valid credentials are found. Checks:
/// - `~/.cline/data/settings/providers.json` → `providers["cline-pass"|"cline"].settings.auth`
/// - `~/.pi/agent/auth.json` → `clinepass.{access,refresh,expires}`
///
/// `expiresAt`/`expires` from the Cline CLI are **milliseconds** since epoch
/// (JS `Date.now()`); this function converts them to unix seconds for koma's
/// `OAuthConn.expires_at` / manager staleness windows.
pub fn resolve_workos_from_disk() -> Option<(String, String, u64, String)> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let candidates = [
        std::path::PathBuf::from(&home).join(".cline/data/settings/providers.json"),
        std::path::PathBuf::from(&home).join(".pi/agent/auth.json"),
    ];

    let mut best: Option<(String, String, u64, String)> = None;

    for path in candidates {
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // providers.json format: providers["cline-pass"|"cline"].settings.auth
        for key in &["cline-pass", "cline"] {
            if let Some(auth) = parsed
                .get("providers")
                .and_then(|p| p.get(*key))
                .and_then(|p| p.get("settings"))
                .and_then(|s| s.get("auth"))
            {
                if let Some(entry) = try_extract_workos_entry(auth) {
                    if better_than(&entry, &best) {
                        best = Some(entry);
                    }
                }
            }
        }

        // pi auth.json format: { clinepass: { access, refresh, expires } }
        // `expires` is ms since epoch (JS Date.now()), same as Cline CLI.
        if let Some(obj) = parsed.get("clinepass").and_then(|v| v.as_object()) {
            if let Some(access) = obj.get("access").and_then(|v| v.as_str()) {
                if is_workos_token(access) {
                    let refresh = obj
                        .get("refresh")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let expires_ms = obj
                        .get("expires")
                        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
                        .unwrap_or(0);
                    let expires = ms_to_secs(expires_ms);
                    let entry = (access.to_string(), refresh, expires, String::new());
                    if better_than(&entry, &best) {
                        best = Some(entry);
                    }
                }
            }
        }
    }

    best
}

/// Try to extract a WorkOS entry (access, refresh, expires_at_secs, email) from a
/// JSON auth object. Returns `None` if the access token is missing or not a
/// WorkOS token. `expiresAt` is converted from ms → secs.
fn try_extract_workos_entry(auth: &serde_json::Value) -> Option<(String, String, u64, String)> {
    let access = auth.get("accessToken").and_then(|v| v.as_str())?;
    if !is_workos_token(access) {
        return None;
    }
    let refresh = auth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Cline CLI stores expiresAt as JS milliseconds.
    let expires_ms = auth
        .get("expiresAt")
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .unwrap_or(0);
    let expires = ms_to_secs(expires_ms);
    let email = auth
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((access.to_string(), refresh, expires, email))
}

/// Convert a millisecond timestamp to unix seconds. Values already in seconds
/// (< year ~2001 in ms terms, i.e. < 1e12) pass through unchanged so a
/// seconds-stamped source isn't divided again.
fn ms_to_secs(ts: u64) -> u64 {
    if ts >= 1_000_000_000_000 {
        ts / 1000
    } else {
        ts
    }
}

/// Check if `entry` is better than `current` (higher expiry, or current is None).
fn better_than(
    entry: &(String, String, u64, String),
    current: &Option<(String, String, u64, String)>,
) -> bool {
    match current {
        None => true,
        Some(c) => entry.2 > c.2, // higher expires_at wins
    }
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max).collect::<String>())
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_workos_token_detects_prefix() {
        assert!(is_workos_token("workos:eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(!is_workos_token("sk-abc123"));
        assert!(!is_workos_token(""));
    }

    #[test]
    fn ms_to_secs_converts_js_timestamps() {
        // JS Date.now()-style ms → secs.
        assert_eq!(ms_to_secs(1_750_000_000_000), 1_750_000_000);
        // Already-seconds values pass through.
        assert_eq!(ms_to_secs(1_750_000_000), 1_750_000_000);
        assert_eq!(ms_to_secs(0), 0);
    }
}

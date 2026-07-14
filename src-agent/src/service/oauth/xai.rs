//! The xAI (Grok) OAuth flow: an RFC 8628 OAuth 2.0 device-authorization grant
//! against `auth.x.ai`. Mirrors the Kilo Code device flow's *shape* (request a
//! device code, open the verification URL, poll for approval — no local redirect
//! listener), but speaks the standard device-grant dialect and differs from Kilo
//! in two ways that matter downstream:
//!
//! 1. The token endpoint is DISCOVERED from xAI's OIDC configuration document on
//!    every login AND every refresh (never hardcoded, never cached), then
//!    VALIDATED to an `https` URL on `x.ai` / `*.x.ai` before any token POST — so
//!    a tampered discovery document can't redirect a bearer-bearing request to an
//!    attacker-controlled host.
//! 2. xAI issues a refresh token + access-token expiry, so the resulting
//!    [`OAuthConn`] is rotated near expiry by `manager::fresh_key` (see
//!    [`refresh`]) — unlike Kilo, whose tokens carry no expiry.
//!
//! The token-endpoint response reuses the shared [`TokenResponse`] shape from the
//! codex module so the manager's persist/update path stays provider-agnostic.

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::time::{sleep, Duration};

use super::codex::TokenResponse;
use super::jwt;
use super::registry::{XAI_CLIENT_ID, XAI_DEVICE_URL, XAI_DISCOVERY_URL, XAI_SCOPE};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

/// Never poll the token endpoint faster than this, regardless of the server's
/// advertised `interval` (and the floor a `slow_down` backs off from).
const MIN_POLL_SECS: u64 = 5;

/// A freshly issued device authorization: the opaque `device_code` we poll with,
/// the short `user_code` the user reads, and the URL they approve it at.
pub struct DeviceCode {
    /// Opaque code POSTed to the token endpoint while polling for approval.
    pub device_code: String,
    /// Short human-readable code the user confirms in the browser.
    pub user_code: String,
    /// URL the user opens to approve — `verification_uri_complete` (which embeds
    /// the user code) when present, else the bare `verification_uri`.
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(serde::Deserialize)]
struct DeviceInitResponse {
    device_code: String,
    user_code: String,
    #[serde(default)]
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_expires_in() -> u64 {
    600
}

fn default_interval() -> u64 {
    5
}

/// Kick off a device-authorization login against xAI's (static) device endpoint:
/// form-encoded `client_id` + `scope`, yielding the device/user codes.
pub async fn device_init(http: &reqwest::Client) -> Result<DeviceCode, String> {
    let resp = http
        .post(XAI_DEVICE_URL)
        .form(&[("client_id", XAI_CLIENT_ID), ("scope", XAI_SCOPE)])
        .send()
        .await
        .map_err(|e| format!("device code request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "device code request failed ({status}): {}",
            truncate(&body, 200)
        ));
    }

    let body: DeviceInitResponse = resp
        .json()
        .await
        .map_err(|e| format!("device code response parse failed: {e}"))?;

    let verification_url = if !body.verification_uri_complete.is_empty() {
        body.verification_uri_complete
    } else {
        body.verification_uri
    };

    Ok(DeviceCode {
        device_code: body.device_code,
        user_code: body.user_code,
        verification_url,
        expires_in: body.expires_in,
        interval: body.interval,
    })
}

#[derive(serde::Deserialize, Default)]
struct Discovery {
    #[serde(default)]
    token_endpoint: String,
}

/// Fetch xAI's OIDC discovery document and return its `token_endpoint`, after
/// validating it is an `https` URL on `x.ai` (or a `*.x.ai` subdomain). Called
/// fresh on every login-poll and refresh — never cached — so the token POST's
/// destination is re-derived and re-checked each time.
async fn discover_token_endpoint(http: &reqwest::Client) -> Result<String, String> {
    let resp = http
        .get(XAI_DISCOVERY_URL)
        .send()
        .await
        .map_err(|e| format!("discovery request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("discovery request failed ({})", resp.status()));
    }
    let doc: Discovery = resp
        .json()
        .await
        .map_err(|e| format!("discovery response parse failed: {e}"))?;
    if doc.token_endpoint.is_empty() {
        return Err("discovery document has no token_endpoint".to_string());
    }
    if !is_valid_xai_endpoint(&doc.token_endpoint) {
        return Err("discovered token_endpoint failed host validation".to_string());
    }
    Ok(doc.token_endpoint)
}

/// True when `u` is an `https` URL whose host is exactly `x.ai` or a `*.x.ai`
/// subdomain, carrying no userinfo. Any other scheme, an embedded credential
/// (`user[:pass]@host`), or any other host is rejected — so a poisoned discovery
/// document can't point a token POST at an attacker-controlled endpoint.
///
/// This MUST derive the host from the same WHATWG URL parser the HTTP client
/// dials with (`url::Url`, reqwest's own), NOT by hand: for special schemes like
/// `https`, WHATWG treats `\` as a path separator, so a naive string-split host
/// extractor reads `https://evil.com\.x.ai/…` as host `evil.com\.x.ai` (which
/// "ends with .x.ai") while the client actually connects to `evil.com` — a
/// token-exfiltration bypass. Parsing with the real parser makes the validator
/// and the connection agree by construction.
fn is_valid_xai_endpoint(u: &str) -> bool {
    let parsed = match url::Url::parse(u) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if parsed.scheme() != "https" {
        return false;
    }
    // Reject any embedded credentials (`user@` / `user:pass@`).
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    match parsed.host_str() {
        Some(h) => {
            let h = h.to_ascii_lowercase();
            h == "x.ai" || h.ends_with(".x.ai")
        }
        None => false,
    }
}

/// Poll xAI's discovered token endpoint for device-code approval. Discovers +
/// validates the token endpoint once up front, then POSTs the device-code grant
/// every `interval` (>= [`MIN_POLL_SECS`]) seconds until approval, denial,
/// expiry, or `expires_in` seconds elapse. Honours `slow_down` (backs the
/// interval off by 5s) and keeps waiting on `authorization_pending`.
pub async fn poll(
    http: &reqwest::Client,
    device_code: &str,
    expires_in: u64,
    interval: u64,
) -> Result<TokenResponse, String> {
    let token_endpoint = discover_token_endpoint(http).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    let mut wait = interval.max(MIN_POLL_SECS);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("device login expired — restart the login flow".to_string());
        }
        sleep(Duration::from_secs(wait)).await;

        let resp = http
            .post(&token_endpoint)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", XAI_CLIENT_ID),
                ("device_code", device_code),
            ])
            .send()
            .await
            .map_err(|e| format!("device poll request failed: {e}"))?;

        if resp.status().is_success() {
            return resp
                .json::<TokenResponse>()
                .await
                .map_err(|e| format!("token response parse failed: {e}"));
        }

        // Non-success: read the OAuth `error` code to decide pending vs slow_down
        // vs fatal. An error body carries no token material, so it's safe to read.
        let body = resp.text().await.unwrap_or_default();
        match oauth_error_code(&body).as_str() {
            "authorization_pending" => {} // not yet approved — keep polling
            "slow_down" => wait += 5,     // RFC 8628: back off by 5s
            "expired_token" => {
                return Err("device login expired — restart the login flow".to_string())
            }
            "access_denied" => return Err("device login denied".to_string()),
            other => {
                if other.is_empty() {
                    return Err("device login failed".to_string());
                }
                return Err(format!("device login failed: {other}"));
            }
        }
    }
}

/// Pull the OAuth `error` code out of a token-endpoint error body, or `""` when
/// the body isn't JSON or carries no `error` field.
fn oauth_error_code(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// Refresh an xAI access token: re-discover + re-validate the token endpoint,
/// then POST the `refresh_token` grant. Carries the prior refresh token forward
/// when the response omits a rotated one, so a rotation-free refresh keeps a
/// usable refresh token for the next cycle. Returns the shared [`TokenResponse`]
/// so the manager's persist/update path stays provider-agnostic.
pub async fn refresh(http: &reqwest::Client, refresh_token: &str) -> Result<TokenResponse, String> {
    let token_endpoint = discover_token_endpoint(http).await?;
    let resp = http
        .post(&token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", XAI_CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
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

    let mut tokens = resp
        .json::<TokenResponse>()
        .await
        .map_err(|e| format!("refresh response parse failed: {e}"))?;
    // Carry the prior refresh token forward when the response didn't rotate it.
    if tokens.refresh_token.is_empty() {
        tokens.refresh_token = refresh_token.to_string();
    }
    Ok(tokens)
}

/// Assemble an [`OAuthConn`] from a completed xAI device login. xAI issues a
/// refresh token + expiry, so `manager::fresh_key` rotates the access token near
/// expiry. `expires_at` is stored as the raw `now + expires_in` (matching the
/// codex store convention, so a login and a later refresh stamp it identically);
/// the 5-minute refresh skew lives in `manager::is_stale` via
/// `XAI_REFRESH_LEAD_SECS`, not baked into the stamp. `account_id`/`org_id` stay
/// EMPTY — xAI has no org/account concept, and an empty `account_id` is what
/// keeps the OpenAI-compatible transport's `X-Kilocode-OrganizationID` header
/// from ever firing on an xAI request.
pub fn to_conn(tokens: TokenResponse) -> OAuthConn {
    let expires_at = tokens
        .expires_in
        .map(|secs| now_secs() + secs)
        .unwrap_or_else(|| jwt::expiry(&tokens.access_token));

    // Best-effort display identity: the access JWT's `email` claim, if present.
    let email = jwt::decode_payload(&tokens.access_token)
        .and_then(|v| {
            v.get("email")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    let label = if email.is_empty() {
        "personal".to_string()
    } else {
        email.clone()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("xai ({label})"),
        provider: OAuthProvider::Xai,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens.id_token,
        expires_at,
        last_refresh: now_secs(),
        account_id: String::new(),
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
    use super::is_valid_xai_endpoint;

    #[test]
    fn accepts_x_ai_hosts() {
        assert!(is_valid_xai_endpoint("https://auth.x.ai/oauth2/token"));
        assert!(is_valid_xai_endpoint("https://x.ai/token"));
        assert!(is_valid_xai_endpoint("https://api.x.ai/v1/models"));
        assert!(is_valid_xai_endpoint("https://api.x.ai:443/oauth2/token"));
        assert!(is_valid_xai_endpoint("https://AUTH.X.AI/oauth2/token"));
    }

    #[test]
    fn rejects_non_https_and_foreign_hosts() {
        assert!(!is_valid_xai_endpoint("http://auth.x.ai/token")); // not https
        assert!(!is_valid_xai_endpoint("https://evil.com/token"));
        assert!(!is_valid_xai_endpoint("https://x.ai.evil.com/token"));
        assert!(!is_valid_xai_endpoint("https://notx.ai/token")); // not a .x.ai suffix
        assert!(!is_valid_xai_endpoint("https://evilx.ai/token"));
        assert!(!is_valid_xai_endpoint("https://x.ai@evil.com/token")); // userinfo → host evil.com
        assert!(!is_valid_xai_endpoint("https://user:pass@x.ai/token")); // credentials
        assert!(!is_valid_xai_endpoint("//x.ai/token")); // no scheme
        assert!(!is_valid_xai_endpoint("not-a-url"));
    }

    /// Regression: WHATWG treats `\` as a path separator for special (https)
    /// schemes, so `https://evil.com\.x.ai/…` actually DIALS `evil.com`. The old
    /// string-split extractor read that as host `evil.com\.x.ai` and wrongly
    /// passed it (a token-exfiltration bypass). Every vector here dials a NON-x.ai
    /// host and MUST be rejected.
    #[test]
    fn rejects_backslash_authority_confusables() {
        for bad in [
            "https://evil.com\\.x.ai/token",
            "https://evil.com\\.x.ai:443/token",
            "https://evil.com\\.x.ai?a=1",
            "https://evil.com\\.x.ai#frag",
            "https://evil.com\\\\.x.ai/token", // multiple backslashes
        ] {
            assert!(!is_valid_xai_endpoint(bad), "must reject (dials non-x.ai): {bad}");
        }
    }

    /// The hard invariant, checked against the SAME parser the HTTP client dials
    /// with: whenever the validator accepts a URL, the host `url`/reqwest would
    /// actually connect to must be an x.ai host. This holds by construction (the
    /// validator parses via `url::Url`) and guards against a future regression to
    /// hand-rolled host extraction — which would accept a `\`-confusable the
    /// client dials elsewhere. Probes include `\`-forms that legitimately resolve
    /// to x.ai (e.g. `x.ai\.evil.com` → host x.ai, path `/.evil.com`).
    #[test]
    fn accept_implies_dialed_host_is_xai() {
        for probe in [
            "https://x.ai/token",
            "https://api.x.ai/v1/models",
            "https://auth.x.ai/oauth2/token",
            "https://evil.com\\.x.ai/token",
            "https://evil.com\\.x.ai:443/token",
            "https://x.ai\\.evil.com/token", // dials x.ai (path /.evil.com) — safe to accept
            "https://x.ai\\@evil.com/token", // dials x.ai (path /@evil.com) — safe to accept
            "https://x.ai@evil.com/token",   // dials evil.com — must be rejected
            "https://evil.com/token",
            "https://x.ai.evil.com/token",
            "http://x.ai/token",
            "//x.ai/token",
        ] {
            if is_valid_xai_endpoint(probe) {
                let host = url::Url::parse(probe)
                    .ok()
                    .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
                    .unwrap_or_default();
                assert!(
                    host == "x.ai" || host.ends_with(".x.ai"),
                    "validator accepted {probe:?} but the dialed host is {host:?}"
                );
            }
        }
    }
}

//! The Kilo Code OAuth flow: device-authorization grant (RFC 8628-flavoured,
//! Kilo's own dialect) — no local redirect listener, the user approves the
//! device code on Kilo's website and the client polls for the result.

use tokio::time::{sleep, Duration};

use super::registry::{KILO_DEVICE_URL, KILO_PROFILE_URL};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// A freshly issued device code plus the URL the user approves it at.
pub struct DeviceCode {
    pub code: String,
    pub verification_url: String,
    pub expires_in: u64,
}

#[derive(serde::Deserialize)]
struct DeviceInitResponse {
    code: String,
    #[serde(rename = "verificationUrl")]
    verification_url: String,
    #[serde(rename = "expiresIn", default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    300
}

/// Kick off a device-authorization login: request a device code the user
/// approves in their browser.
pub async fn device_init(http: &reqwest::Client) -> Result<DeviceCode, String> {
    let resp = http
        .post(KILO_DEVICE_URL)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("device code request failed: {e}"))?;

    if resp.status().as_u16() == 429 {
        return Err("too many pending device logins — wait a minute and retry".to_string());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("device code request failed ({status})"));
    }

    let body: DeviceInitResponse = resp
        .json()
        .await
        .map_err(|e| format!("device code response parse failed: {e}"))?;

    Ok(DeviceCode {
        code: body.code,
        verification_url: body.verification_url,
        expires_in: body.expires_in,
    })
}

#[derive(serde::Deserialize, Default)]
struct PollResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    token: String,
}

/// Poll for device-code approval every 3 seconds until approved, denied,
/// expired, or `expires_in` seconds have elapsed.
pub async fn poll(http: &reqwest::Client, code: &str, expires_in: u64) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    let url = format!("{KILO_DEVICE_URL}/{code}");

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("device login expired — restart the login flow".to_string());
        }

        let resp = http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("device poll request failed: {e}"))?;

        match resp.status().as_u16() {
            403 => return Err("device login denied".to_string()),
            410 => return Err("device login expired — restart the login flow".to_string()),
            200 => {
                let body: PollResponse = resp.json().await.unwrap_or_default();
                if body.status == "approved" && !body.token.is_empty() {
                    return Ok(body.token);
                }
                // status present but not yet approved ("pending", etc.) — keep polling.
            }
            202 => {} // pending
            _ => {}   // treat any other transient status as pending too
        }

        sleep(POLL_INTERVAL).await;
    }
}

#[derive(serde::Deserialize, Default)]
struct ProfileResponse {
    #[serde(default)]
    organizations: Vec<ProfileOrg>,
    #[serde(default)]
    email: String,
}

#[derive(serde::Deserialize, Default)]
struct ProfileOrg {
    #[serde(default)]
    id: String,
}

/// Fetch the org id (first organization, empty if none) and email for `token`.
pub async fn profile(http: &reqwest::Client, token: &str) -> (String, String) {
    let resp = match http.get(KILO_PROFILE_URL).bearer_auth(token).send().await {
        Ok(r) => r,
        Err(_) => return (String::new(), String::new()),
    };
    if !resp.status().is_success() {
        return (String::new(), String::new());
    }
    let body: ProfileResponse = resp.json().await.unwrap_or_default();
    let org_id = body
        .organizations
        .first()
        .map(|o| o.id.clone())
        .unwrap_or_default();
    (org_id, body.email)
}

/// Assemble an [`OAuthConn`] from a completed device login. Kilo tokens carry
/// no refresh token / expiry in this flow (`expires_at` stays `0`).
pub fn to_conn(token: String, org_id: String, email: String) -> OAuthConn {
    let label = if !org_id.is_empty() {
        org_id.clone()
    } else if !email.is_empty() {
        email.clone()
    } else {
        "personal".to_string()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("kilocode ({label})"),
        provider: OAuthProvider::Kilocode,
        access_token: token,
        refresh_token: String::new(),
        id_token: String::new(),
        expires_at: 0,
        last_refresh: 0,
        account_id: String::new(),
        org_id,
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

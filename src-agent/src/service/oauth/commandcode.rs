//! Command Code OAuth: browser-assisted API key retrieval via localhost POST
//! callback (NOT PKCE). The Studio website POSTs the API key back to a local
//! loopback server. Also supports static API key paste. Flow kind: "callback".
//! LOGIN/TOKEN ONLY — no NDJSON chat transport this PR.

use super::registry::{COMMANDCODE_AUTH_PATH, COMMANDCODE_STUDIO_BASE};
use crate::model::app_config::{new_uuid, OAuthConn, OAuthProvider};

/// Build the Command Code Studio authorization URL. The browser opens this;
/// after authentication, the Studio website POSTs the API key to
/// `http://localhost:{port}/callback` with a CSRF `state` token.
pub fn build_auth_url(port: u16, state: &str) -> String {
    format!(
        "{COMMANDCODE_STUDIO_BASE}{COMMANDCODE_AUTH_PATH}?callback=http://localhost:{port}/callback&state={state}"
    )
}

/// Generate a random 32-byte state token, base64url-encoded. Uses two UUIDs
/// concatenated (256 bits total) — good enough for CSRF.
pub fn generate_state() -> String {
    use base64::Engine;
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(a.as_bytes());
    bytes[16..].copy_from_slice(b.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Build an [`OAuthConn`] from a completed callback or pasted API key. Keys
/// never expire (`expires_at = 0`).
pub fn to_conn(api_key: &str, user_name: &str, user_id: &str) -> OAuthConn {
    let label = if !user_name.is_empty() {
        user_name.to_string()
    } else if !user_id.is_empty() {
        user_id.to_string()
    } else {
        "unknown".to_string()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("commandcode ({label})"),
        provider: OAuthProvider::CommandCode,
        access_token: api_key.to_string(),
        refresh_token: api_key.to_string(),
        id_token: String::new(),
        expires_at: 0,
        last_refresh: 0,
        account_id: user_id.to_string(),
        org_id: String::new(),
        email: user_name.to_string(),
        plan: String::new(),
        ext_id: None,
        provider_id: None,
        chat_endpoint: None,
        api_type: None,
        refresh_token_url: None,
        refresh_client_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_auth_url_contains_port_and_state() {
        let url = build_auth_url(5959, "mystate123");
        assert!(url.contains("localhost:5959/callback"));
        assert!(url.contains("state=mystate123"));
        assert!(url.starts_with("https://commandcode.ai/studio/auth/cli"));
    }

    #[test]
    fn generate_state_is_base64url() {
        let state = generate_state();
        assert!(!state.is_empty());
        // base64url chars only.
        assert!(state
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn to_conn_stamps_identity() {
        let conn = to_conn("key-abc", "Alice", "user_123");
        assert_eq!(conn.provider, OAuthProvider::CommandCode);
        assert_eq!(conn.access_token, "key-abc");
        assert_eq!(conn.refresh_token, "key-abc");
        assert_eq!(conn.expires_at, 0);
        assert_eq!(conn.account_id, "user_123");
        assert_eq!(conn.email, "Alice");
        assert!(conn.name.contains("Alice"));
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
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
    assert!(conn.commandcode_chat.is_none());
}

#[test]
fn detects_go_plan_403() {
    let body = r#"{"error":"403 Forbidden: Your Go plan doesn't include API access. Upgrade to Provider or higher at https://commandcode.ai/billing"}"#;
    assert!(is_provider_api_denied(reqwest::StatusCode::FORBIDDEN, body));
    assert!(!is_provider_api_denied(
        reqwest::StatusCode::UNAUTHORIZED,
        body
    ));
    assert!(!is_provider_api_denied(
        reqwest::StatusCode::FORBIDDEN,
        "rate limited"
    ));
}

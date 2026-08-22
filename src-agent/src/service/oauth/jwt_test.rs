#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn make_jwt(payload_json: &serde_json::Value) -> String {
    let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(payload_json).unwrap());
    format!("x.{payload_b64}.x")
}

#[test]
fn decode_payload_roundtrips() {
    let payload = serde_json::json!({"foo": "bar"});
    let token = make_jwt(&payload);
    assert_eq!(decode_payload(&token), Some(payload));
}

#[test]
fn decode_payload_rejects_malformed_token() {
    assert!(decode_payload("not-a-jwt").is_none());
    assert!(decode_payload("a.b").is_none());
}

#[test]
fn codex_identity_reads_auth_claim() {
    let id_token = make_jwt(&serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_123",
            "chatgpt_plan_type": "plus",
        },
        "email": "user@example.com",
    }));
    let access_token = make_jwt(&serde_json::json!({}));
    let identity = codex_identity(&id_token, &access_token);
    assert_eq!(identity.account_id, "acct_123");
    assert_eq!(identity.plan, "plus");
    assert_eq!(identity.email, "user@example.com");
}

#[test]
fn codex_identity_falls_back_to_top_level_and_access_token_email() {
    let id_token = make_jwt(&serde_json::json!({
        "account_id": "acct_456",
        "plan_type": "pro",
    }));
    let access_token = make_jwt(&serde_json::json!({"email": "fallback@example.com"}));
    let identity = codex_identity(&id_token, &access_token);
    assert_eq!(identity.account_id, "acct_456");
    assert_eq!(identity.plan, "pro");
    assert_eq!(identity.email, "fallback@example.com");
}

#[test]
fn expiry_reads_exp_claim() {
    let token = make_jwt(&serde_json::json!({"exp": 1_700_000_000u64}));
    assert_eq!(expiry(&token), 1_700_000_000);
}

#[test]
fn expiry_defaults_to_zero() {
    let token = make_jwt(&serde_json::json!({}));
    assert_eq!(expiry(&token), 0);
    assert_eq!(expiry("garbage"), 0);
}

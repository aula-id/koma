#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::{OAuthConn, OAuthProvider};

/// A NATIVE OAuthConn exactly as pre-W11 `config.json` wrote it — no
/// `ext_id`/`provider_id` keys, field order matching the struct declaration
/// (which is the order `serde_json` emits). The W11 change MUST NOT alter this.
const NATIVE_CONN_JSON: &str = concat!(
    r#"{"uuid":"11111111-1111-1111-1111-111111111111","name":"codex (me@example.com)","#,
    r#""provider":"codex","access_token":"at","refresh_token":"rt","id_token":"it","#,
    r#""expires_at":1750000000,"last_refresh":1749000000,"account_id":"acc","org_id":"","#,
    r#""email":"me@example.com","plan":"pro"}"#
);

/// SERDE-COMPAT PROOF: a pre-W11 native conn deserializes cleanly (the two new
/// `Option` fields default to `None`) AND re-serializes BYTE-IDENTICALLY. The new
/// fields carry `skip_serializing_if = "Option::is_none"`, so a native conn never
/// emits them — existing `config.json` files round-trip unchanged.
#[test]
fn native_conn_roundtrips_byte_stable() {
    let conn: OAuthConn = serde_json::from_str(NATIVE_CONN_JSON).expect("pre-W11 conn parses");
    assert_eq!(conn.provider, OAuthProvider::Codex);
    assert!(conn.ext_id.is_none());
    assert!(conn.provider_id.is_none());
    // W12: the four data-driven-resolution fields also default to None on a native
    // conn, so they too are omitted from the on-disk JSON.
    assert!(conn.chat_endpoint.is_none());
    assert!(conn.api_type.is_none());
    assert!(conn.refresh_token_url.is_none());
    assert!(conn.refresh_client_id.is_none());
    // A native conn is never a model provider (no chat_endpoint/api_type meta).
    assert!(conn.ext_model_route().is_none());
    let reser = serde_json::to_string(&conn).expect("serializes");
    assert_eq!(
        reser, NATIVE_CONN_JSON,
        "a native OAuthConn must round-trip byte-identically after W11/W12"
    );
}

/// An EXT-backed conn serializes with the `"extension"` provider tag plus the two
/// ext fields and the W12 model-provider meta, and round-trips back to an equal value.
#[test]
fn ext_conn_roundtrips() {
    let conn = OAuthConn {
        uuid: "22222222-2222-2222-2222-222222222222".to_string(),
        name: "Demo account".to_string(),
        provider: OAuthProvider::Extension,
        access_token: "demo-at".to_string(),
        email: "demo@example.com".to_string(),
        ext_id: Some("run.koma.example.oauth-demo-daemon".to_string()),
        provider_id: Some("demo".to_string()),
        // W12 model-provider meta.
        chat_endpoint: Some("https://api.demo.test/v1".to_string()),
        api_type: Some("openai".to_string()),
        refresh_token_url: Some("https://demo.test/token".to_string()),
        refresh_client_id: Some("cid".to_string()),
        ..Default::default()
    };
    let v = serde_json::to_value(&conn).expect("serializes");
    assert_eq!(v["provider"], "extension");
    assert_eq!(v["ext_id"], "run.koma.example.oauth-demo-daemon");
    assert_eq!(v["provider_id"], "demo");
    assert_eq!(v["chat_endpoint"], "https://api.demo.test/v1");
    assert_eq!(v["api_type"], "openai");
    assert_eq!(v["refresh_token_url"], "https://demo.test/token");
    assert_eq!(v["refresh_client_id"], "cid");

    let back: OAuthConn = serde_json::from_value(v).expect("ext conn roundtrips");
    assert_eq!(back.provider, OAuthProvider::Extension);
    assert_eq!(
        back.ext_id.as_deref(),
        Some("run.koma.example.oauth-demo-daemon")
    );
    assert_eq!(back.provider_id.as_deref(), Some("demo"));
    assert_eq!(back.access_token, "demo-at");
    assert_eq!(
        back.chat_endpoint.as_deref(),
        Some("https://api.demo.test/v1")
    );
    assert_eq!(back.api_type.as_deref(), Some("openai"));
    assert_eq!(
        back.refresh_token_url.as_deref(),
        Some("https://demo.test/token")
    );
}

/// [`OAuthConn::ext_model_route`] accepts a conn with both a chat endpoint and a
/// recognised api_type, mapping the wire string to the dispatch [`ApiType`]; it rejects
/// a conn missing either half (account-login-only) or carrying an unrecognised api_type.
#[test]
fn ext_model_route_gates_on_endpoint_and_api_type() {
    use super::ApiType;
    let with = |endpoint: Option<&str>, api_type: Option<&str>| OAuthConn {
        provider: OAuthProvider::Extension,
        chat_endpoint: endpoint.map(str::to_string),
        api_type: api_type.map(str::to_string),
        ..Default::default()
    };
    // openai → OpenAiCompatible.
    assert_eq!(
        with(Some("https://x.test/v1"), Some("openai")).ext_model_route(),
        Some(("https://x.test/v1", ApiType::OpenAiCompatible))
    );
    // anthropic → AnthropicCompatible.
    assert_eq!(
        with(Some("https://x.test"), Some("anthropic")).ext_model_route(),
        Some(("https://x.test", ApiType::AnthropicCompatible))
    );
    // Missing endpoint, missing api_type, or unrecognised api_type → account-login-only.
    assert!(with(None, Some("openai")).ext_model_route().is_none());
    assert!(with(Some("https://x.test"), None)
        .ext_model_route()
        .is_none());
    assert!(with(Some("   "), Some("openai"))
        .ext_model_route()
        .is_none());
    assert!(with(Some("https://x.test"), Some("openai_compatible"))
        .ext_model_route()
        .is_none());
}

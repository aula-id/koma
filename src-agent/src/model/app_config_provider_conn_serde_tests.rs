#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::{ApiType, ProviderConn};

/// A NATIVE ProviderConn exactly as pre-W12b `config.json` wrote it — no `ext_id` key,
/// field order matching the struct declaration (which is the order `serde_json` emits).
const NATIVE_PROVIDER_JSON: &str = concat!(
    r#"{"uuid":"33333333-3333-3333-3333-333333333333","name":"OpenRouter","#,
    r#""api_type":"open_ai_compatible","endpoint":"https://openrouter.ai/api/v1","#,
    r#""api_key":"sk-abc"}"#
);

/// SERDE-COMPAT PROOF: a pre-W12b native provider deserializes cleanly (the new `ext_id`
/// field defaults to `None`) AND re-serializes BYTE-IDENTICALLY — `skip_serializing_if =
/// "Option::is_none"` means a native provider never emits the key. Existing `config.json`
/// files round-trip unchanged (the W11/W12 `OAuthConn` discipline, applied to providers).
#[test]
fn native_provider_roundtrips_byte_stable() {
    let p: ProviderConn =
        serde_json::from_str(NATIVE_PROVIDER_JSON).expect("pre-W12b provider parses");
    assert_eq!(p.api_type, ApiType::OpenAiCompatible);
    assert!(p.ext_id.is_none(), "a native provider carries no ext_id");
    let reser = serde_json::to_string(&p).expect("serializes");
    assert_eq!(
        reser, NATIVE_PROVIDER_JSON,
        "a native ProviderConn must round-trip byte-identically after W12b"
    );
}

/// An EXT-owned (key-backed) provider serializes WITH the `ext_id` key and round-trips.
#[test]
fn ext_provider_roundtrips() {
    let p = ProviderConn {
        uuid: "44444444-4444-4444-4444-444444444444".to_string(),
        name: "Demo Gateway".to_string(),
        api_type: ApiType::AnthropicCompatible,
        endpoint: "https://api.demo.test/v1".to_string(),
        api_key: "demo-key".to_string(),
        ext_id: Some("run.koma.example.gateway".to_string()),
    };
    let v = serde_json::to_value(&p).expect("serializes");
    assert_eq!(v["ext_id"], "run.koma.example.gateway");
    let back: ProviderConn = serde_json::from_value(v).expect("ext provider roundtrips");
    assert_eq!(back.ext_id.as_deref(), Some("run.koma.example.gateway"));
    assert_eq!(back.api_type, ApiType::AnthropicCompatible);
    assert_eq!(back, p);
}

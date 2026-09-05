#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use reqwest::StatusCode;

#[test]
fn retryable_statuses() {
    // Retryable: server errors + rate limit
    for code in [500, 502, 503, 520, 529, 429] {
        let s = StatusCode::from_u16(code).unwrap();
        assert!(is_retryable_status(s), "expected {code} to be retryable");
    }
    // NOT retryable: success + permanent client errors
    for code in [200, 201, 204, 400, 401, 403, 404, 405, 422] {
        let s = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(s),
            "expected {code} to NOT be retryable"
        );
    }
}

#[test]
fn backoff_delay_is_monotonic_and_bounded() {
    let d1 = backoff_delay(1);
    let d2 = backoff_delay(2);
    let d3 = backoff_delay(3);
    // Each base is larger than the previous.
    assert!(d2 > d1, "d2={d2:?} should be > d1={d1:?}");
    assert!(d3 > d2, "d3={d3:?} should be > d2={d2:?}");
    // Upper bound: base + max jitter.
    assert!(d3 <= std::time::Duration::from_millis(4000 + JITTER_MS));
    // Lower bound: base + 0 jitter.
    assert!(d1 >= std::time::Duration::from_millis(1000));
}

#[test]
fn max_attempts_is_three() {
    assert_eq!(MAX_ATTEMPTS, 3);
}

fn sample_conn(endpoint: &str, api_type: crate::model::app_config::ApiType) -> Conn<'_> {
    Conn {
        endpoint,
        api_key: "k",
        api_type,
        account_id: "",
        oauth_uuid: "",
        install_id: "",
    }
}

#[test]
fn wants_openrouter_usage_or_and_koma_free_only() {
    use crate::model::app_config::ApiType;
    assert!(wants_openrouter_usage(&sample_conn(
        "https://openrouter.ai/api/v1",
        ApiType::OpenAiCompatible
    )));
    assert!(wants_openrouter_usage(&sample_conn(
        "https://koma.run/v1",
        ApiType::KomaFree
    )));
    // Direct OpenAI-compatible — no proprietary usage.include
    assert!(!wants_openrouter_usage(&sample_conn(
        "https://api.x.ai/v1",
        ApiType::OpenAiCompatible
    )));
    assert!(!wants_openrouter_usage(&sample_conn(
        "https://api.deepseek.com/v1",
        ApiType::OpenAiCompatible
    )));
    // koma-free only by api_type, not bare endpoint without type
    assert!(!wants_openrouter_usage(&sample_conn(
        "https://koma.run/v1",
        ApiType::OpenAiCompatible
    )));
}

#[test]
fn chat_request_usage_fields_serialize_by_dialect() {
    use crate::dto::openrouter::{ChatRequest, StreamOptions, UsageRequest};

    let or = ChatRequest {
        model: "m".into(),
        messages: vec![],
        stream: true,
        provider: None,
        usage: Some(UsageRequest { include: true }),
        stream_options: None,
        tools: None,
        reasoning: None,
        response_format: None,
        max_tokens: None,
    };
    let or_json = serde_json::to_value(&or).unwrap();
    assert_eq!(or_json["usage"]["include"], true);
    assert!(or_json.get("stream_options").is_none());

    let direct = ChatRequest {
        model: "m".into(),
        messages: vec![],
        stream: true,
        provider: None,
        usage: None,
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
        tools: None,
        reasoning: None,
        response_format: None,
        max_tokens: None,
    };
    let d_json = serde_json::to_value(&direct).unwrap();
    assert!(d_json.get("usage").is_none(), "direct must omit usage: {d_json}");
    assert_eq!(d_json["stream_options"]["include_usage"], true);

    let oneshot_direct = ChatRequest {
        model: "m".into(),
        messages: vec![],
        stream: false,
        provider: None,
        usage: None,
        stream_options: None,
        tools: None,
        reasoning: None,
        response_format: None,
        max_tokens: None,
    };
    let o_json = serde_json::to_value(&oneshot_direct).unwrap();
    assert!(o_json.get("usage").is_none());
    assert!(o_json.get("stream_options").is_none());
}

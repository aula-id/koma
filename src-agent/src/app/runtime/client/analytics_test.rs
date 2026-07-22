#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Host-side Analytics projection + push-envelope serialization tests.
//!
//! These cover the Phase 1 Analytics dashboard contract: correlation fields are
//! echoed, status is explicitly `"ok"`/`"empty"`, and the push serializes to the
//! camelCase wire shape the web GUI expects. They never touch a real ledger
//! (all empty-result cases use a non-existent session UUID so they do not depend
//! on the developer's global ledger contents).

use std::cell::RefCell;

use super::diff::compute_analytics;
use super::push_proto::{
    push_analytics, PushAnalyticsModel, PushAnalyticsSeriesPoint, PushEnvelope,
};

#[test]
fn compute_analytics_echoes_correlation_and_empty_status() {
    // Missing/empty ledger → zero calls → status "empty", never hang.
    let result = compute_analytics(
        42,
        "session".to_string(),
        Some("__analytics_test_missing_session__".to_string()),
        "7d".to_string(),
        "cost".to_string(),
    );
    assert_eq!(result.req_seq, 42);
    assert_eq!(result.scope, "session");
    assert_eq!(
        result.session_id.as_deref(),
        Some("__analytics_test_missing_session__")
    );
    assert_eq!(result.range, "7d");
    assert_eq!(result.metric, "cost");
    assert_eq!(result.status, "empty");
    assert!(result.error.is_none());
    assert_eq!(result.calls, 0);
    assert_eq!(result.cache_rate, 0.0);
    // 7d → 7 zero-filled daily buckets.
    assert_eq!(result.series.len(), 7);
    assert!(result.series.iter().all(|p| p.cost == 0.0 && p.tokens == 0));
}

#[test]
fn compute_analytics_today_has_24_hourly_buckets() {
    let result = compute_analytics(
        1,
        "session".to_string(),
        Some("sess-uuid".to_string()),
        "today".to_string(),
        "tokens".to_string(),
    );
    assert_eq!(result.session_id.as_deref(), Some("sess-uuid"));
    assert_eq!(result.range, "today");
    assert_eq!(result.metric, "tokens");
    assert_eq!(result.series.len(), 24);
    assert_eq!(result.status, "empty");
}

#[test]
fn push_analytics_serializes_camel_case_contract() {
    let result = compute_analytics(
        7,
        "session".to_string(),
        Some("__analytics_test_missing_session__".to_string()),
        "30d".to_string(),
        "cost".to_string(),
    );
    let pushed = RefCell::new(String::new());
    push_analytics(&|json| *pushed.borrow_mut() = json, result);
    let pushed = pushed.into_inner();
    assert!(!pushed.is_empty(), "push_analytics must emit JSON");

    let v: serde_json::Value = serde_json::from_str(&pushed).expect("valid JSON");
    assert_eq!(v["k"], "Analytics");
    assert_eq!(v["reqSeq"], 7);
    assert_eq!(v["scope"], "session");
    assert_eq!(v["sessionId"], "__analytics_test_missing_session__");
    assert_eq!(v["range"], "30d");
    assert_eq!(v["metric"], "cost");
    assert_eq!(v["status"], "empty");
    assert!(v["error"].is_null());
    assert!(v["cost"].is_number());
    assert!(v["tokensIn"].is_number());
    assert!(v["tokensCached"].is_number());
    assert!(v["tokensOut"].is_number());
    assert!(v["calls"].is_number());
    assert!(v["cacheRate"].is_number());
    assert!(v["series"].is_array());
    assert!(v["models"].is_array());
    assert!(v["mainCost"].is_number());
    assert!(v["mainCalls"].is_number());
    assert!(v["subCost"].is_number());
    assert!(v["subCalls"].is_number());
    // 30d → 30 zero-filled daily buckets.
    assert_eq!(v["series"].as_array().unwrap().len(), 30);
}

#[test]
fn analytics_envelope_round_trips_model_and_series_shapes() {
    // Directly build the envelope shape to lock the nested camelCase fields
    // without depending on a populated ledger.
    let env = PushEnvelope::Analytics {
        req_seq: 3,
        scope: "session".to_string(),
        session_id: Some("abc".to_string()),
        range: "year".to_string(),
        metric: "tokens".to_string(),
        status: "ok".to_string(),
        error: None,
        cost: 1.25,
        tokens_in: 100,
        tokens_cached: 50,
        tokens_out: 20,
        calls: 4,
        cache_rate: 50.0 / 150.0,
        series: vec![PushAnalyticsSeriesPoint {
            epoch: 1_700_000_000,
            cost: 0.5,
            tokens: 30,
        }],
        models: vec![PushAnalyticsModel {
            model_id: "gpt-test".to_string(),
            cost: 1.25,
            tokens_in: 100,
            tokens_cached: 50,
            tokens_out: 20,
            calls: 4,
        }],
        main_cost: 1.0,
        main_calls: 3,
        sub_cost: 0.25,
        sub_calls: 1,
    };
    let json = serde_json::to_string(&env).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(v["k"], "Analytics");
    assert_eq!(v["sessionId"], "abc");
    assert_eq!(v["series"][0]["epoch"], 1_700_000_000);
    assert_eq!(v["series"][0]["cost"], 0.5);
    assert_eq!(v["series"][0]["tokens"], 30);
    assert_eq!(v["models"][0]["modelId"], "gpt-test");
    assert_eq!(v["models"][0]["tokensIn"], 100);
    assert_eq!(v["models"][0]["tokensCached"], 50);
    assert_eq!(v["models"][0]["tokensOut"], 20);
    assert_eq!(v["models"][0]["calls"], 4);
    assert!((v["cacheRate"].as_f64().unwrap() - (50.0 / 150.0)).abs() < 1e-9);
}

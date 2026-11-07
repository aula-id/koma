use super::*;
use serde_json::json;

#[test]
fn a_scripted_reply_is_returned() {
    let f = shared(Script::new().then_result("sec_http", r#"{"verdict":"confirmed"}"#));
    assert!(call(&f, "sec_http", &json!({})).contains("confirmed"));
}

#[test]
fn an_unscripted_call_answers_the_same_way_every_time() {
    let f = shared(Script::new());
    for _ in 0..3 {
        assert_eq!(
            call(&f, "sec_http", &json!({})),
            call(&f, "sec_http", &json!({}))
        );
    }
}

#[test]
fn a_scripted_reply_is_consumed_once() {
    // What makes a duplicate call detectable: the second call must differ.
    let f = shared(Script::new().then_result("sec_http", "first"));
    let one = call(&f, "sec_http", &json!({}));
    let two = call(&f, "sec_http", &json!({}));
    assert_ne!(one, two);
}

#[test]
fn replies_are_matched_per_tool_not_in_order() {
    let f = shared(
        Script::new()
            .then_result("sec_ffuf", "for-ffuf")
            .then_result("sec_http", "for-http"),
    );
    assert_eq!(call(&f, "sec_http", &json!({})), "for-http");
    assert_eq!(call(&f, "sec_ffuf", &json!({})), "for-ffuf");
}

#[test]
fn calls_are_recorded_in_order() {
    let f = shared(Script::new());
    let _ = call(&f, "sec_http", &json!({"url": "a"}));
    let _ = call(&f, "sec_ffuf", &json!({"url": "b"}));
    assert_eq!(
        recorded(&f),
        "sec_http {\"url\":\"a\"}\nsec_ffuf {\"url\":\"b\"}"
    );
}

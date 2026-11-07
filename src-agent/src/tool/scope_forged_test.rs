use crate::app::sec::lane::Lane;
use crate::tool::ToolCtx;

fn ctx(lane: Option<Lane>) -> ToolCtx {
    // `lane_refusal` reads only `cyber_lane`, so a bare context is enough and
    // does not drag in the (undeclared) shared test helper.
    let mut c = crate::tool::test_ctx();
    c.cyber_lane = lane;
    c
}

fn refuse(lane: Option<Lane>, tool: &str, args: serde_json::Value) -> Option<String> {
    let mut args = args;
    super::lane_refusal(&ctx(lane), tool, &mut args)
}

#[test]
fn a_forged_name_still_meets_the_lane_gate() {
    // The model supplies the name; the lane is looked up from the session, so
    // claiming a tool the lane forbids is refused even though the call parses.
    let refusal = refuse(Some(Lane::Research), "sec_ffuf", serde_json::json!({}));
    assert!(refusal.expect("refused").contains("research"));
}

#[test]
fn a_tool_the_lane_allows_passes() {
    assert!(refuse(Some(Lane::Research), "sec_decode", serde_json::json!({})).is_none());
}

#[test]
fn with_no_lane_a_sec_call_is_refused_and_read_is_not() {
    let refusal = refuse(None, "sec_ffuf", serde_json::json!({}));
    assert!(refusal.expect("refused").contains("no cyber lane"));
    assert!(refuse(None, "sec_decode", serde_json::json!({})).is_some());
    assert!(refuse(None, "read", serde_json::json!({})).is_none());
}

#[test]
fn a_panel_disabled_tool_is_refused() {
    let mut c = ctx(Some(Lane::Red));
    c.sec_inactive.push("sec_http".into());
    let mut args = serde_json::json!({});
    let refusal = super::lane_refusal(&c, "sec_http", &mut args);
    assert!(refusal.expect("refused").contains("disabled"));
}

#[test]
fn an_inactive_reason_is_the_sec_refusal() {
    let mut c = ctx(None);
    c.sec_block = Some("security mode is off".into());
    let mut args = serde_json::json!({});
    let refusal = super::lane_refusal(&c, "sec_http", &mut args);
    assert_eq!(refusal.as_deref(), Some("security mode is off"));
}

#[test]
fn blue_may_get_a_bare_url_and_nothing_else() {
    let mut ok = serde_json::json!({"url": "https://a.example/path"});
    assert!(super::lane_refusal(&ctx(Some(Lane::Blue)), "sec_http", &mut ok).is_none());
    assert_eq!(ok["follow_redirects"], serde_json::json!(false));
    assert!(refuse(Some(Lane::Blue), "edit", serde_json::json!({})).is_none());
    for (tool, args) in [
        (
            "sec_http",
            serde_json::json!({"url": "https://a.example/x?q=1"}),
        ),
        (
            "sec_http",
            serde_json::json!({"url": "https://a.example/x", "method": "POST"}),
        ),
        (
            "sec_http",
            serde_json::json!({"url": "https://a.example/x", "body": "p"}),
        ),
        (
            "sec_http",
            serde_json::json!({"url": "https://a.example/x", "headers": {"X": "1"}}),
        ),
        (
            "sec_http",
            serde_json::json!({"url": "https://a.example/x", "follow_redirects": true}),
        ),
        ("sec_sqlmap", serde_json::json!({})),
        ("sec_browser", serde_json::json!({})),
        ("sec_xss_confirm", serde_json::json!({})),
        ("bash", serde_json::json!({})),
    ] {
        assert!(
            refuse(Some(Lane::Blue), tool, args).is_some(),
            "{tool} was admitted"
        );
    }
}

#[test]
fn research_is_refused_the_network_but_allowed_local_compute() {
    assert!(refuse(Some(Lane::Research), "sec_http", serde_json::json!({})).is_some());
    assert!(refuse(Some(Lane::Research), "sec_hashid", serde_json::json!({})).is_none());
}

#[test]
fn every_tool_name_is_gated_consistently_across_calls() {
    // Same input, same answer: the gate is a pure function of (lane, name).
    let c = ctx(Some(Lane::Red));
    for tool in ["sec_http", "sec_decode", "sec_ffuf"] {
        let mut a_args = serde_json::json!({});
        let mut b_args = serde_json::json!({});
        let a = super::lane_refusal(&c, tool, &mut a_args);
        let b = super::lane_refusal(&c, tool, &mut b_args);
        assert_eq!(a, b, "{tool}");
    }
}

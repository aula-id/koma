use super::*;

/// Full path a handshake entry takes: deserialize, then convert. Both stages
/// can refuse, and `serde` owns the unknown-`risk` case, so the helper folds
/// them into one error rather than only exercising the second stage.
fn desc(v: serde_json::Value) -> Result<SecToolDesc, String> {
    let tool: WireTool =
        serde_json::from_value(v).map_err(|e| format!("malformed descriptor: {e}"))?;
    tool.into_desc()
}

fn net_tool(compute: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "sec_http",
        "description": "d",
        "compute": compute,
        "risk": "remote",
        "parameters": {
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        },
    })
}

#[test]
fn unknown_risk_label_is_refused_not_defaulted() {
    let mut v = net_tool("network");
    v["risk"] = serde_json::json!("remotee");
    assert!(desc(v).unwrap_err().contains("remotee"));
}

#[test]
fn missing_risk_is_refused_rather_than_assumed_harmless() {
    // The tier decides containment. Defaulting it to `Static` advertised a
    // network tool as pure compute, so it would be offered with no egress
    // parameter and only fail later, at dispatch.
    let mut v = net_tool("network");
    v.as_object_mut().expect("object").remove("risk");
    let reason = desc(v).expect_err("missing risk is refused");
    assert!(reason.contains("risk"), "{reason}");
}

#[test]
fn unknown_compute_class_is_refused() {
    assert!(desc(net_tool("netwrok")).unwrap_err().contains("netwrok"));
}

#[test]
fn missing_compute_class_is_refused() {
    let mut v = net_tool("network");
    v.as_object_mut().expect("object").remove("compute");
    assert!(desc(v).unwrap_err().contains("compute class"));
}

#[test]
fn remote_tool_records_its_network_parameter() {
    let got = desc(net_tool("network")).expect("network argument is declared");
    assert_eq!(got.network_param.as_deref(), Some("url"));
}

#[test]
fn remote_tool_without_a_network_argument_is_refused() {
    let mut v = net_tool("network");
    v["parameters"] = serde_json::json!({
        "type": "object",
        "properties": {"size": {"type": "integer"}},
    });
    assert!(desc(v).unwrap_err().contains("no network argument"));
}

#[test]
fn static_tool_needs_no_network_argument() {
    let v = serde_json::json!({
        "name": "sec_decode", "description": "d",
        "compute": "instant-cpu", "risk": "static",
        "parameters": {"type": "object", "properties": {"data": {"type": "string"}}},
    });
    assert_eq!(desc(v).expect("static needs no destination").risk, super::super::SecRisk::Static);
}

#[test]
fn one_bad_descriptor_does_not_cost_the_others() {
    let tools = serde_json::json!([
        net_tool("network"),
        {"name": "sec_broken", "compute": "nope", "risk": "static"},
        {"name": "sec_nameless_check", "compute": "light"},
    ]);
    let (ok, refused) = parse_tools(Some(&tools));
    // One good descriptor survives; both bad ones are refused individually.
    assert_eq!(ok.len(), 1);
    assert_eq!(refused.len(), 2);
    assert_eq!(refused[0].0, "sec_broken");
    // The nameless one has no risk tier either, so it is refused on that too —
    // the point is the refusal is per-descriptor, not that it is one field.
    assert_eq!(refused[1].0, "sec_nameless_check");
}

#[test]
fn non_array_tools_yield_nothing() {
    assert!(parse_tools(Some(&serde_json::json!("nope"))).0.is_empty());
    assert!(parse_tools(None).0.is_empty());
}

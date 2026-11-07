use super::*;

/// Full path a handshake entry takes: deserialize, then convert. Both stages
/// can refuse, and `serde` owns the unknown-label case, so the helper folds
/// them into one error rather than only exercising the second stage.
fn desc(v: serde_json::Value) -> Result<SecToolDesc, String> {
    let tool: WireTool =
        serde_json::from_value(v).map_err(|e| format!("malformed descriptor: {e}"))?;
    tool.into_desc()
}

fn net_tool_v1(compute: &str) -> serde_json::Value {
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

fn net_tool_v2(exec: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "sec_http",
        "description": "d",
        "exec": exec,
        "effect": "remote",
        "domain": "web",
        "parameters": {
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        },
    })
}

#[test]
fn unknown_risk_label_is_refused_not_defaulted() {
    let mut v = net_tool_v1("network");
    v["risk"] = serde_json::json!("remotee");
    assert!(desc(v).unwrap_err().contains("remotee"));
}

#[test]
fn missing_reach_tier_is_refused_rather_than_assumed_harmless() {
    let mut v = net_tool_v1("network");
    v.as_object_mut().expect("object").remove("risk");
    let reason = desc(v).expect_err("missing risk is refused");
    assert!(
        reason.contains("effect") || reason.contains("risk"),
        "{reason}"
    );
}

#[test]
fn unknown_compute_class_is_refused() {
    assert!(desc(net_tool_v1("netwrok")).unwrap_err().contains("netwrok"));
}

#[test]
fn missing_runtime_class_is_refused() {
    let mut v = net_tool_v1("network");
    v.as_object_mut().expect("object").remove("compute");
    let reason = desc(v).unwrap_err();
    assert!(
        reason.contains("exec") || reason.contains("compute"),
        "{reason}"
    );
}

#[test]
fn remote_tool_records_its_network_parameter() {
    let got = desc(net_tool_v1("network")).expect("network argument is declared");
    assert_eq!(got.network_param.as_deref(), Some("url"));
}

#[test]
fn remote_tool_without_a_network_argument_is_refused() {
    let mut v = net_tool_v1("network");
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
    assert_eq!(
        desc(v).expect("static needs no destination").risk,
        super::super::SecRisk::Static
    );
}

#[test]
fn one_bad_descriptor_does_not_cost_the_others() {
    let tools = serde_json::json!([
        net_tool_v1("network"),
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

#[test]
fn phase2_effect_exec_is_accepted() {
    let got = desc(net_tool_v2("net")).expect("v2 wire");
    assert_eq!(got.risk, super::super::SecRisk::Remote);
    assert_eq!(got.compute, "net");
    assert_eq!(got.network_param.as_deref(), Some("url"));
    assert_eq!(got.domain, "web");
}

#[test]
fn phase2_effect_preferred_over_legacy_risk() {
    let mut v = net_tool_v2("net");
    v["risk"] = serde_json::json!(false); // would map to Local if preferred
    let got = desc(v).expect("effect wins");
    assert_eq!(got.risk, super::super::SecRisk::Remote);
}

#[test]
fn legacy_bool_risk_false_maps_to_local() {
    let v = serde_json::json!({
        "name": "sec_decode",
        "description": "d",
        "compute": "instant-cpu",
        "risk": false,
        "parameters": {"type": "object", "properties": {"data": {"type": "string"}}},
    });
    let got = desc(v).expect("legacy bool risk");
    assert_eq!(got.risk, super::super::SecRisk::Local);
    assert_eq!(got.compute, "cpu");
}

#[test]
fn legacy_bool_risk_true_maps_to_remote_and_needs_destination() {
    let v = serde_json::json!({
        "name": "sec_http",
        "description": "d",
        "compute": "network",
        "risk": true,
        "parameters": {
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        },
    });
    let got = desc(v).expect("legacy bool remote");
    assert_eq!(got.risk, super::super::SecRisk::Remote);
    assert_eq!(got.compute, "net");
    assert_eq!(got.network_param.as_deref(), Some("url"));
}

#[test]
fn legacy_compute_labels_normalize_to_exec() {
    assert_eq!(desc(net_tool_v1("network")).unwrap().compute, "net");
    assert_eq!(desc(net_tool_v1("instant-cpu")).unwrap().compute, "cpu");
    let binary = serde_json::json!({
        "name": "sec_remote",
        "risk": "remote",
        "compute": "executes-target",
        "parameters": {
            "type": "object",
            "properties": {"host": {"type": "string"}},
            "required": ["host"],
        },
    });
    assert_eq!(desc(binary).unwrap().compute, "binary");
}

#[test]
fn fixed_host_remote_tool_needs_no_destination_argument() {
    let v = serde_json::json!({
        "name": "sec_factor",
        "effect": "remote",
        "exec": "net",
        "parameters": {
            "type": "object",
            "properties": {"n": {"type": "string"}},
            "required": ["n"],
        },
    });
    let got = desc(v).expect("fixed-host remote is shippable");
    assert_eq!(got.risk, super::super::SecRisk::Remote);
    assert!(got.network_param.is_none());
}

//! Capability metadata as it arrives on the wire, and its conversion into the
//! gated [`SecToolDesc`].
//!
//! Two stages, deliberately separate:
//!
//! 1. [`WireTool`] deserializes the daemon's handshake entry. `risk` is a
//!    three-value [`RiskWire`] with no catch-all, so an unrecognized label is a
//!    parse error rather than a silent `false`.
//! 2. [`WireTool::into_desc`] applies the fail-closed rules with no daemon
//!    involved: the [allow-list](ALLOWED_COMPUTE) on `compute`, and the
//!    downgrade for a `remote`/`exploit` tool whose schema declares no network
//!    argument. Both are testable in isolation, which is the point — the rules
//!    that matter cannot be exercised without a live child otherwise.
//!
//! Absent-vs-unknown is a real distinction here. `risk` falls back to
//! [`RiskWire::Static`] (the weakest tier, so a daemon that never learned the
//! field is not trusted), while `compute` fails the tool outright (the tier
//! decides routing and containment, so a guess is worse than a gap).

use serde::Deserialize;

use super::{RiskWire, SecToolDesc};

/// `compute` values this parent understands.
///
/// A daemon-side typo (`"netwrok"`) must not quietly classify a network tool as
/// pure compute, so anything outside this list is refused. Adding a value is a
/// two-line change here plus its Python counterpart.
const ALLOWED_COMPUTE: &[&str] = &["instant-cpu", "light", "network", "browser", "heavy"];

/// One advertised tool, straight off the handshake.
#[derive(Debug, Clone, Deserialize)]
struct WireTool {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "empty_object")]
    parameters: serde_json::Value,
    #[serde(default)]
    risk: RiskWire,
    #[serde(default)]
    compute: String,
    #[serde(default)]
    domain: String,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

/// Parse the handshake's `tools` array.
///
/// Per-tool refusals are returned alongside the tools instead of failing the
/// whole handshake: one malformed descriptor must not cost the user every other
/// tool. Refusals are `(tool name, reason)` and are surfaced by the caller so the
/// gap is visible rather than silent.
pub(super) fn parse_tools(
    value: Option<&serde_json::Value>,
) -> (Vec<SecToolDesc>, Vec<(String, String)>) {
    let Some(arr) = value.and_then(serde_json::Value::as_array) else {
        return (Vec::new(), Vec::new());
    };
    let mut descs = Vec::with_capacity(arr.len());
    let mut refused = Vec::new();
    for entry in arr {
        // The name is needed to report a refusal, so it is read out first and its
        // absence is its own case.
        let Some(name) = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .filter(|n| !n.is_empty())
        else {
            refused.push(("<unnamed>".to_owned(), "descriptor carried no name".to_owned()));
            continue;
        };
        let tool: WireTool = match serde_json::from_value(entry.clone()) {
            Ok(t) => t,
            Err(e) => {
                refused.push((name, format!("malformed descriptor: {e}")));
                continue;
            }
        };
        match tool.into_desc() {
            Ok(d) => descs.push(d),
            Err(reason) => refused.push((name, reason)),
        }
    }
    (descs, refused)
}

impl WireTool {
    /// Apply the fail-closed rules and yield a gated descriptor.
    fn into_desc(self) -> Result<SecToolDesc, String> {
        let compute = self.compute.trim().to_string();
        if compute.is_empty() {
            return Err("descriptor carried no compute class".to_owned());
        }
        if !ALLOWED_COMPUTE.contains(&compute.as_str()) {
            return Err(format!("unknown compute class '{compute}'"));
        }
        let risk = self.risk.into();
        // A network tool whose schema offers no way to name a destination is
        // unreachable through the enforced egress path, so it is not safe to
        // advertise: `sec_nuclei` takes `url`, `sec_http` takes `url`, but a
        // hypothetical `sec_portscan` taking one is fine too — the parameter name
        // is recorded rather than assumed, because the gate has to name it.
        let network_param = if matches!(risk, super::SecRisk::Static | super::SecRisk::Local) {
            None
        } else {
            match network_parameter(&self.parameters) {
                Some(p) => Some(p),
                None => {
                    return Err(format!(
                        "risk '{risk:?}' but the schema declares no network argument"
                    ))
                }
            }
        };
        Ok(SecToolDesc {
            name: self.name,
            description: self.description,
            parameters: self.parameters,
            risk,
            network_param,
            compute,
            domain: self.domain,
        })
    }
}

/// The first `string` parameter whose name marks it as a destination.
const DESTINATION_HINTS: &[&str] = &["url", "host", "target", "hostname", "endpoint"];

fn network_parameter(parameters: &serde_json::Value) -> Option<String> {
    let props = parameters.get("properties")?.as_object()?;
    // Prefer a declared `required` destination over property order.
    let required = parameters
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    for name in &required {
        if is_string_prop(props.get(*name)) && DESTINATION_HINTS.iter().any(|h| name.to_lowercase().contains(h)) {
            return Some((*name).to_string());
        }
    }
    for hint in DESTINATION_HINTS {
        for (name, spec) in props {
            if name.to_lowercase() == *hint && is_string_prop(Some(spec)) {
                return Some(name.clone());
            }
        }
    }
    None
}

fn is_string_prop(spec: Option<&serde_json::Value>) -> bool {
    matches!(
        spec.and_then(|s| s.get("type")).and_then(serde_json::Value::as_str),
        Some("string")
    )
}

#[cfg(test)]
mod tests {
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
    fn missing_risk_falls_back_to_the_weakest_tier() {
        let mut v = net_tool("network");
        v.as_object_mut()
            .expect("object")
            .remove("risk");
        let got = desc(v).expect("missing risk is tolerated");
        assert_eq!(got.risk, super::super::SecRisk::Static);
        // Static, so it does not need a network parameter even though it has one.
        assert!(got.network_param.is_none());
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
        assert_eq!(ok.len(), 2);
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].0, "sec_broken");
    }

    #[test]
    fn non_array_tools_yield_nothing() {
        assert!(parse_tools(Some(&serde_json::json!("nope"))).0.is_empty());
        assert!(parse_tools(None).0.is_empty());
    }
}

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
//! Absent-vs-unknown is a real distinction here, and both now fail CLOSED. `risk`
//! is REQUIRED: an absent tier used to fall back to [`RiskWire::Static`], the
//! weakest value, so a descriptor that never learned the field was advertised as
//! harmless — a network tool would have been offered without an egress parameter
//! and would have failed at dispatch instead of at handshake. `compute` and
//! `domain` fail the tool outright for the same reason: the tier decides routing
//! and containment, so a guess is worse than a visible gap.

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
    /// No default: see the module note. `Option` so the refusal can say which
    /// field was missing rather than reporting a generic malformed descriptor.
    #[serde(default)]
    risk: Option<RiskWire>,
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
        let Some(risk) = self.risk else {
            return Err("descriptor carried no risk tier".to_owned());
        };
        let risk = risk.into();
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
#[path = "wire_tool_test.rs"]
mod tests;


//! Capability metadata as it arrives on the wire, and its conversion into the
//! gated [`SecToolDesc`].
//!
//! Two stages, deliberately separate:
//!
//! 1. [`WireTool`] deserializes the daemon's handshake entry. Reach and runtime
//!    axes accept the phase-2 vocabulary (`effect` / `exec`) preferred by the
//!    Python daemon, plus the phase-1 spellings (`risk` / `compute`) so a
//!    half-migrated pair still fails closed with a clear reason rather than a
//!    serde type error.
//! 2. [`WireTool::into_desc`] applies the fail-closed rules with no daemon
//!    involved: the allow-list on the runtime class, and the downgrade for a
//!    `remote`/`exploit` tool whose schema declares no network argument (unless
//!    it is a fixed-host tool such as `sec_factor`).
//!
//! Absent-vs-unknown is a real distinction here, and both fail CLOSED. The reach
//! tier is REQUIRED: an absent tier must not fall back to the weakest value, so
//! a descriptor that never learned the field is refused at handshake instead of
//! being advertised as harmless.

use serde::Deserialize;

use super::{RiskWire, SecToolDesc};

/// Runtime classes this parent understands on the wire.
///
/// Phase-2 `exec` values (`cpu`/`binary`/`net`/`browser`) are canonical. The
/// older `compute` labels stay accepted so a daemon that still emits them is not
/// wiped out by a vocabulary rename alone — they normalize to the same internal
/// string the cockpit already shows.
const ALLOWED_EXEC: &[&str] = &["cpu", "binary", "net", "browser"];

/// One advertised tool, straight off the handshake.
///
/// Fields are optional at deserialize time so we can name *which* axis is missing
/// in the refusal rather than collapsing every gap into a serde type error.
#[derive(Debug, Clone, Deserialize)]
struct WireTool {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "empty_object")]
    parameters: serde_json::Value,
    /// Phase-2 reach axis (preferred). Same labels as [`RiskWire`].
    #[serde(default)]
    effect: Option<RiskWire>,
    /// Phase-1 reach axis. Accepts the string enum *or* a legacy bool via
    /// [`FlexibleRisk`].
    #[serde(default)]
    risk: Option<FlexibleRisk>,
    /// Phase-2 runtime axis (preferred): `cpu` / `binary` / `net` / `browser`.
    #[serde(default)]
    #[serde(rename = "exec")]
    exec_class: Option<String>,
    /// Phase-1 runtime axis (`instant-cpu`, `network`, …). Normalized into the
    /// phase-2 vocabulary when `exec` is absent.
    #[serde(default)]
    compute: Option<String>,
    #[serde(default)]
    domain: String,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

/// `risk` on the wire: string enum (current) or legacy bool (`true`→remote,
/// `false`→local). Bool mapping is intentionally coarse — stale installs should
/// be refreshed via `koma --security-install`; this only keeps the handshake
/// from refusing every tool with a type error while that happens.
#[derive(Debug, Clone, Copy)]
enum FlexibleRisk {
    Labeled(RiskWire),
    LegacyBool(bool),
}

impl<'de> Deserialize<'de> for FlexibleRisk {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = FlexibleRisk;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a risk/effect label string or a legacy boolean")
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(FlexibleRisk::LegacyBool(v))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let labeled = match v.trim().to_ascii_lowercase().as_str() {
                    "static" => RiskWire::Static,
                    "local" => RiskWire::Local,
                    "remote" => RiskWire::Remote,
                    "exploit" => RiskWire::Exploit,
                    other => {
                        return Err(E::unknown_variant(
                            other,
                            &["static", "local", "remote", "exploit"],
                        ))
                    }
                };
                Ok(FlexibleRisk::Labeled(labeled))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

impl FlexibleRisk {
    fn into_wire(self) -> RiskWire {
        match self {
            FlexibleRisk::Labeled(w) => w,
            // Legacy bool: true meant "touches a target", false meant local/static.
            // Map true→Remote and false→Local so egress still applies to old network
            // tools; static pure-compute tools that used false lose a bit of
            // precision (Local vs Static) which is safer than under-classifying.
            FlexibleRisk::LegacyBool(true) => RiskWire::Remote,
            FlexibleRisk::LegacyBool(false) => RiskWire::Local,
        }
    }
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
        let risk_wire = self
            .effect
            .or_else(|| self.risk.map(FlexibleRisk::into_wire))
            .ok_or_else(|| "descriptor carried no effect/risk tier".to_owned())?;
        let risk = risk_wire.into();

        let compute = resolve_runtime(self.exec_class.as_deref(), self.compute.as_deref())?;

        // A network tool whose schema offers no way to name a destination is
        // unreachable through the enforced egress path, so it is not safe to
        // advertise — unless it is a fixed-host tool (destination is constant
        // in the handler, not an argument).
        let network_param = if matches!(risk, super::SecRisk::Static | super::SecRisk::Local) {
            None
        } else if crate::security::fixed_host(&self.name) {
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

/// Pick a runtime class from `exec` (preferred) or legacy `compute`.
fn resolve_runtime(exec: Option<&str>, compute: Option<&str>) -> Result<String, String> {
    if let Some(raw) = exec.map(str::trim).filter(|s| !s.is_empty()) {
        let key = raw.to_ascii_lowercase();
        if ALLOWED_EXEC.contains(&key.as_str()) {
            return Ok(key);
        }
        return Err(format!("unknown exec class '{raw}'"));
    }
    if let Some(raw) = compute.map(str::trim).filter(|s| !s.is_empty()) {
        return normalize_legacy_compute(raw);
    }
    Err("descriptor carried no exec/compute class".to_owned())
}

/// Map phase-1 `compute` labels onto the phase-2 `exec` vocabulary.
fn normalize_legacy_compute(raw: &str) -> Result<String, String> {
    let key = raw.to_ascii_lowercase();
    let mapped = match key.as_str() {
        "cpu" | "instant-cpu" | "light" | "long-cpu" | "gpu" | "heavy" => "cpu",
        "binary" | "executes-target" => "binary",
        "net" | "network" => "net",
        "browser" => "browser",
        other => return Err(format!("unknown compute class '{other}'")),
    };
    Ok(mapped.to_owned())
}

/// The first `string` parameter whose name marks it as a destination.
const DESTINATION_HINTS: &[&str] = &["url", "host", "target", "hostname", "endpoint"];

fn network_parameter(parameters: &serde_json::Value) -> Option<String> {
    let props = parameters.get("properties")?.as_object()?;
    // Prefer a declared `required` destination over property order.
    let required = parameters
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for name in &required {
        if is_string_prop(props.get(*name))
            && DESTINATION_HINTS
                .iter()
                .any(|h| name.to_lowercase().contains(h))
        {
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
        spec.and_then(|s| s.get("type"))
            .and_then(serde_json::Value::as_str),
        Some("string")
    )
}

#[cfg(test)]
#[path = "wire_tool_test.rs"]
mod tests;

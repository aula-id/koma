//! The tool schemas of the typed puts (`put_claim`, `put_trajectory`, ...).
//!
//! A model fills a tool from its schema, so the schema has to name every field
//! the record builder (`record.rs`) demands. The tests at the bottom hold the
//! two together: a record built from exactly the required fields is accepted,
//! and one without any of them is refused.

use serde_json::{json, Map, Value};

use super::record::{typed_prefix, SEVERITIES, VERDICTS};

fn text(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn list(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

fn choice(values: &[&str], description: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn number(description: &str) -> Value {
    json!({ "type": "number", "description": description })
}

fn whole(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 0, "description": description })
}

struct Fields {
    items: Vec<(&'static str, Value)>,
    required: Vec<&'static str>,
}

impl Fields {
    fn add(&mut self, name: &'static str, schema: Value, need: bool) {
        self.items.push((name, schema));
        if need {
            self.required.push(name);
        }
    }

    fn cvss(&mut self) {
        self.add("cve_ids", list("CVE ids"), false);
        self.add("cwe_ids", list("CWE ids"), false);
        self.add("cvss_score", number("CVSS score, 0 to 10"), false);
        self.add("cvss_vector", text("CVSS vector string"), false);
    }
}

const REVISION: &str = "0 to create the item, or the revision the last get returned";

/// The description and parameter schema of `put_<kind>`, or `None` for an unknown kind.
pub fn typed_tool(kind: &str) -> Option<(String, Value)> {
    let prefix = typed_prefix(kind)?;
    let mut f = Fields { items: vec![("key", text(&format!("Shelf key; must start with {prefix}")))], required: vec!["key"] };
    // Written by a plain compare-and-swap: these carry `expected_revision`. Assets and trajectories merge instead.
    let versioned = !matches!(kind, "asset" | "trajectory");
    let description = match kind {
        "scope" => {
            f.add("host", text("The host or target this scope covers"), true);
            f.add("allow", list("Hosts, paths or actions that are in scope"), false);
            f.add("deny", list("Hosts, paths or actions that are out of scope"), false);
            f.add("goal_key", text("The goal this scope belongs to"), false);
            f.add("notes", text("Rules of engagement and caveats"), false);
            "Record the authorized scope."
        }
        "claim" => {
            f.add("vulnclass", text("Vulnerability class, such as ssrf or idor"), true);
            f.add("name", text("Short name of the claim"), true);
            f.add("severity", choice(SEVERITIES, "How serious it is"), true);
            f.add("summary", text("What is wrong and why it matters"), true);
            f.add("host", text("The host or component affected"), true);
            f.add("poc", text("Steps or a command that reproduce it"), true);
            f.add("evidence_keys", list("Shelf keys under evidence/"), false);
            f.add("asset_keys", list("Shelf keys under assets/"), false);
            f.cvss();
            "Publish a vulnerability claim for review."
        }
        "evidence" => {
            f.add("claim_key", text("The claim this supports; starts with claims/"), true);
            f.add("evidence_kind", text("What kind of evidence: request, response, output, screenshot, log"), true);
            f.add("body", text("The evidence itself"), true);
            f.add("note", text("What to notice in it"), false);
            "Attach evidence to a claim."
        }
        "verdict" => {
            f.add("claim_key", text("The claim being judged; starts with claims/"), true);
            f.add("claim_revision", json!({ "type": "integer", "minimum": 1, "description": "The revision of the claim you checked" }), true);
            f.add("verdict", choice(VERDICTS, "Your decision"), true);
            f.add("note", text("Why, with what you re-ran"), true);
            f.add("recheck_evidence_keys", list("Evidence keys you produced while re-checking"), false);
            "Record the independent verdict on a claim."
        }
        "finding" => {
            f.add("claim_key", text("The confirmed claim; starts with claims/"), true);
            f.add("verdict_key", text("The verdict that confirmed it; starts with verdicts/"), true);
            f.add("vulnclass", text("Vulnerability class"), true);
            f.add("name", text("Short name"), true);
            f.add("severity", choice(SEVERITIES, "How serious it is"), true);
            f.add("summary", text("What is wrong and why it matters"), true);
            f.add("host", text("The host or component affected"), true);
            f.add("poc", text("Steps or a command that reproduce it"), false);
            f.cvss();
            "Publish a confirmed finding."
        }
        "report" => {
            f.add("markdown", text("The report body in markdown"), true);
            f.add("finding_key", text("The main finding; starts with findings/"), false);
            f.add("finding_keys", list("All findings the report covers. Give this or finding_key"), false);
            f.add("title", text("Report title"), false);
            f.add("audience", choice(&["manager", "technical"], "Who reads it (default manager)"), false);
            f.add("severity_summary", text("One line on the severities"), false);
            f.add("go_no_go", text("Release recommendation"), false);
            f.cvss();
            "Publish a report over findings. Give finding_key or finding_keys."
        }
        "asset" => {
            f.add("host", text("The host the asset lives on"), true);
            f.add("asset_type", text("host, service, endpoint, credential, repo, ..."), true);
            f.add("title", text("Short name"), true);
            f.add("detail", text("What you learned"), false);
            f.add("tags", list("Free tags"), false);
            f.add("status", text("State of the asset"), false);
            f.add("tech", list("Technology seen"), false);
            f.add("paths", list("Paths seen"), false);
            f.add("ports", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Ports seen" }), false);
            f.add("parent_key", text("Parent asset key"), false);
            f.add("evidence_keys", list("Related evidence keys"), false);
            f.add("claim_keys", list("Related claim keys"), false);
            f.add("finding_keys", list("Related finding keys"), false);
            f.add("trajectory_goal", text("Goal whose map this asset appears on"), false);
            f.add("last_seen", whole("Unix time last seen"), false);
            "Record or update an asset. Writes merge with what is there."
        }
        "trajectory" => {
            f.add("goal_key", text("The goal this map belongs to"), true);
            f.add(
                "nodes",
                json!({ "type": "array", "description": "Steps and facts on the map", "items": { "type": "object", "properties": {
                    "id": { "type": "string", "description": "Unique id on this map" },
                    "label": { "type": "string" },
                    "kind": { "type": "string", "description": "fact, hypothesis, action, claim, ... (default fact)" },
                    "status": { "type": "string" },
                    "severity": { "type": "string" },
                    "resource_key": { "type": "string", "description": "A shelf key this node stands for" },
                    "note": { "type": "string" }
                }, "required": ["id"] } }),
                true,
            );
            f.add(
                "edges",
                json!({ "type": "array", "description": "Links between node ids. Send [] when there are none", "items": { "type": "object", "properties": {
                    "from": { "type": "string", "description": "Node id the link starts at" },
                    "to": { "type": "string", "description": "Node id the link ends at" },
                    "rel": { "type": "string", "description": "leads_to, confirms, blocks, ..." }
                }, "required": ["from", "to"] } }),
                true,
            );
            "Add nodes and edges to a goal's map. Writes merge by id with what is there."
        }
        "result" => {
            f.add("title", text("What this result is"), true);
            f.add("body", text("The result"), true);
            "Publish a result."
        }
        _ => return None,
    };
    if versioned {
        f.add("expected_revision", whole(REVISION), true);
    }
    let mut properties = Map::new();
    for (name, schema) in f.items {
        properties.insert(name.to_string(), schema);
    }
    let schema = json!({ "type": "object", "properties": properties, "required": f.required });
    Some((description.to_string(), schema))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::shelf::record::build_record;

    const KINDS: &[&str] = &["scope", "claim", "evidence", "verdict", "finding", "report", "asset", "trajectory", "result"];

    /// A value the builder accepts for a field, by name.
    fn sample(kind: &str, name: &str, schema: &Value) -> Value {
        match name {
            "claim_key" => json!("claims/a"),
            "verdict_key" => json!("verdicts/a"),
            "severity" => json!("high"),
            "verdict" => json!("confirmed"),
            "claim_revision" => json!(1),
            "nodes" => json!([{ "id": "n1" }]),
            "edges" => json!([]),
            _ => match schema.get("type").and_then(Value::as_str) {
                Some("array") => json!([]),
                Some("integer") | Some("number") => json!(0),
                _ => json!(format!("{kind}-{name}")),
            },
        }
    }

    fn args(kind: &str, skip: Option<&str>) -> Map<String, Value> {
        let (_, schema) = typed_tool(kind).expect("schema");
        let properties = schema["properties"].as_object().expect("properties");
        let mut out = Map::new();
        for name in schema["required"].as_array().expect("required").iter().filter_map(Value::as_str) {
            if Some(name) != skip {
                out.insert(name.to_string(), sample(kind, name, &properties[name]));
            }
        }
        if kind == "report" {
            out.insert("finding_key".into(), json!("findings/a"));
        }
        out
    }

    #[test]
    fn a_record_built_from_the_required_fields_alone_is_accepted() {
        for kind in KINDS {
            let built = build_record(kind, &args(kind, None));
            assert!(built.is_ok(), "{kind}: {:?}", built.err());
        }
    }

    #[test]
    fn leaving_out_any_required_field_is_refused() {
        for kind in KINDS {
            let (_, schema) = typed_tool(kind).expect("schema");
            for name in schema["required"].as_array().expect("required").iter().filter_map(Value::as_str) {
                if matches!(name, "key" | "expected_revision") {
                    continue; // checked by the store, not the record builder
                }
                assert!(build_record(kind, &args(kind, Some(name))).is_err(), "{kind} accepted a record without {name}");
            }
        }
    }

    #[test]
    fn every_typed_tool_has_a_schema_and_the_trajectory_one_names_its_lists() {
        for kind in KINDS {
            let (description, schema) = typed_tool(kind).expect("schema");
            assert!(!description.is_empty());
            assert_eq!(schema["type"], "object");
            assert!(schema["properties"]["key"]["description"].as_str().unwrap().contains(typed_prefix(kind).unwrap()));
        }
        let (_, trajectory) = typed_tool("trajectory").unwrap();
        let required: Vec<&str> = trajectory["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(required, ["key", "goal_key", "nodes", "edges"]);
        assert!(typed_tool("nonsense").is_none());
    }
}

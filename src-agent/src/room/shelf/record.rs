//! Typed shelf records (`put_scope`, `put_claim`, ...). Each builder validates
//! its fields and returns the JSON object that is stored.

use serde_json::{json, Map, Value};

use super::args::{
    fail, mixed_list, now, optional_cvss, optional_unix, require_positive_int, string_list, text,
    text_req, Args, MAX_CONTENT,
};
use super::trajectory;
use super::ShelfError;

pub const SEVERITIES: &[&str] = &["critical", "high", "medium", "low", "info"];
pub const VERDICTS: &[&str] = &["confirmed", "returned", "dismissed", "discarded"];

/// Key prefix each typed tool writes under.
pub fn typed_prefix(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "scope" => "scope/",
        "claim" => "claims/",
        "evidence" => "evidence/",
        "verdict" => "verdicts/",
        "finding" => "findings/",
        "report" => "reports/",
        "asset" => "assets/",
        "trajectory" => "trajectory/",
        "result" => "results/",
        _ => return None,
    })
}

fn severity(args: &Args) -> Result<String, ShelfError> {
    let value = text_req(args, "severity", 16)?.to_lowercase();
    if !SEVERITIES.contains(&value.as_str()) {
        return fail("severity must be critical|high|medium|low|info");
    }
    Ok(value)
}

fn keys_with_prefix(args: &Args, name: &str, prefix: &str) -> Result<Vec<String>, ShelfError> {
    let keys = string_list(args, name, 32)?;
    if keys.iter().any(|k| !k.starts_with(prefix)) {
        return fail(format!("{name} entries must start with {prefix}"));
    }
    Ok(keys)
}

pub fn build_record(kind: &str, args: &Args) -> Result<Map<String, Value>, ShelfError> {
    let updated = now();
    let value = match kind {
        "scope" => json!({
            "kind": kind,
            "host": text_req(args, "host", 512)?,
            "allow": string_list(args, "allow", 32)?,
            "deny": string_list(args, "deny", 32)?,
            "goal_key": text(args, "goal_key", false, 128)?,
            "notes": text(args, "notes", false, 20000)?,
            "updated": updated,
        }),
        "claim" => {
            let severity = severity(args)?;
            let evidence_keys = keys_with_prefix(args, "evidence_keys", "evidence/")?;
            let asset_keys = keys_with_prefix(args, "asset_keys", "assets/")?;
            json!({
                "kind": kind,
                "status": "open",
                "vulnclass": text_req(args, "vulnclass", 128)?,
                "name": text_req(args, "name", 256)?,
                "severity": severity,
                "summary": text_req(args, "summary", 20000)?,
                "host": text_req(args, "host", 512)?,
                "poc": text_req(args, "poc", 20000)?,
                "evidence_keys": evidence_keys,
                "cve_ids": string_list(args, "cve_ids", 32)?,
                "cwe_ids": string_list(args, "cwe_ids", 32)?,
                "cvss_score": optional_cvss(args.get("cvss_score"))?,
                "cvss_vector": text(args, "cvss_vector", false, 128)?,
                "asset_keys": asset_keys,
                "updated": updated,
            })
        }
        "evidence" => {
            let claim_key = text_req(args, "claim_key", 128)?;
            if !claim_key.starts_with("claims/") && !claim_key.starts_with("claim/") {
                return fail("claim_key must start with claims/");
            }
            json!({
                "kind": kind,
                "claim_key": claim_key,
                "evidence_kind": text_req(args, "evidence_kind", 32)?,
                "body": text_req(args, "body", MAX_CONTENT / 2)?,
                "note": text(args, "note", false, 20000)?,
                "updated": updated,
            })
        }
        "verdict" => {
            let verdict = text_req(args, "verdict", 16)?.to_lowercase();
            if !VERDICTS.contains(&verdict.as_str()) {
                return fail("verdict must be confirmed|returned|dismissed|discarded");
            }
            json!({
                "kind": kind,
                "claim_key": text_req(args, "claim_key", 128)?,
                "claim_revision": require_positive_int(args.get("claim_revision"), "claim_revision")?,
                "verdict": verdict,
                "note": text_req(args, "note", 20000)?,
                "recheck_evidence_keys": string_list(args, "recheck_evidence_keys", 32)?,
                "updated": updated,
            })
        }
        "finding" => {
            let severity = severity(args)?;
            json!({
                "kind": kind,
                "status": "confirmed",
                "claim_key": text_req(args, "claim_key", 128)?,
                "verdict_key": text_req(args, "verdict_key", 128)?,
                "vulnclass": text_req(args, "vulnclass", 128)?,
                "name": text_req(args, "name", 256)?,
                "severity": severity,
                "summary": text_req(args, "summary", 20000)?,
                "host": text_req(args, "host", 512)?,
                "poc": text(args, "poc", false, 20000)?,
                "cve_ids": string_list(args, "cve_ids", 32)?,
                "cwe_ids": string_list(args, "cwe_ids", 32)?,
                "cvss_score": optional_cvss(args.get("cvss_score"))?,
                "cvss_vector": text(args, "cvss_vector", false, 128)?,
                "updated": updated,
            })
        }
        "report" => {
            let audience = match text(args, "audience", false, 16)? {
                Some(a) => {
                    let a = a.to_lowercase();
                    if a != "manager" && a != "technical" {
                        return fail("audience must be manager|technical");
                    }
                    a
                }
                None => "manager".to_string(),
            };
            let mut finding_keys = string_list(args, "finding_keys", 32)?;
            let finding_key = match text(args, "finding_key", false, 128)? {
                Some(k) => k,
                None => match finding_keys.first() {
                    Some(k) => k.clone(),
                    None => return fail("finding_key or finding_keys is required"),
                },
            };
            if !finding_keys.contains(&finding_key) {
                finding_keys.insert(0, finding_key.clone());
            }
            json!({
                "kind": kind,
                "title": text(args, "title", false, 256)?,
                "audience": audience,
                "finding_key": finding_key,
                "finding_keys": finding_keys,
                "severity_summary": text(args, "severity_summary", false, 512)?,
                "go_no_go": text(args, "go_no_go", false, 64)?,
                "cve_ids": string_list(args, "cve_ids", 32)?,
                "cwe_ids": string_list(args, "cwe_ids", 32)?,
                "cvss_score": optional_cvss(args.get("cvss_score"))?,
                "cvss_vector": text(args, "cvss_vector", false, 128)?,
                "markdown": text_req(args, "markdown", MAX_CONTENT / 2)?,
                "updated": updated,
            })
        }
        "asset" => json!({
            "kind": kind,
            "host": text_req(args, "host", 512)?,
            "asset_type": text_req(args, "asset_type", 64)?,
            "title": text_req(args, "title", 256)?,
            "detail": text(args, "detail", false, 20000)?,
            "tags": string_list(args, "tags", 32)?,
            "status": text(args, "status", false, 64)?,
            "tech": string_list(args, "tech", 32)?,
            "paths": string_list(args, "paths", 64)?,
            "ports": mixed_list(args, "ports", 32)?,
            "parent_key": text(args, "parent_key", false, 128)?,
            "evidence_keys": string_list(args, "evidence_keys", 32)?,
            "claim_keys": string_list(args, "claim_keys", 32)?,
            "finding_keys": string_list(args, "finding_keys", 32)?,
            "trajectory_goal": text(args, "trajectory_goal", false, 128)?,
            "last_seen": optional_unix(args.get("last_seen"))?.unwrap_or(updated),
            "updated": updated,
        }),
        "trajectory" => {
            let (nodes, edges) = trajectory::lists(args)?;
            json!({
                "kind": kind,
                "goal_key": text_req(args, "goal_key", 128)?,
                "nodes": nodes.iter().filter_map(trajectory::normalize_node).collect::<Vec<_>>(),
                "edges": edges.iter().filter_map(trajectory::normalize_edge).collect::<Vec<_>>(),
                "updated": updated,
            })
        }
        "result" => json!({
            "kind": kind,
            "title": text_req(args, "title", 256)?,
            "body": text(args, "body", true, 20000)?,
            "updated": updated,
        }),
        _ => return fail("Unknown kind"),
    };
    match value {
        Value::Object(map) => Ok(map),
        _ => fail("record is not an object"),
    }
}

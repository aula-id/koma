//! Gate truth, read from the shelf: what is open, verified or returned.
//!
//! Ported from Orpheus `shelf-status.ts`. The rule that matters: a claim is
//! **verified only when a finding exists for it**. A reviewer's "confirmed"
//! verdict alone is not enough, and a bot stamping its own trajectory node
//! "confirmed" is ignored until the shelf proves it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::graph::Node;
use crate::room::shelf::ShelfEntry;

const RETURNED: &[&str] = &["returned", "dismissed", "discarded"];

pub fn parse_object(content: Option<&str>) -> Option<serde_json::Map<String, Value>> {
    match serde_json::from_str::<Value>(content?).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn str_field(map: &Option<serde_json::Map<String, Value>>, name: &str) -> Option<String> {
    map.as_ref()?.get(name)?.as_str().map(str::to_string)
}

pub fn is_claim_key(key: &str) -> bool {
    key.starts_with("claims/") || key.starts_with("claim/")
}

pub fn is_verdict_key(key: &str) -> bool {
    key.starts_with("verdicts/") || key.starts_with("verdict/")
}

pub fn is_finding_key(key: &str) -> bool {
    key.starts_with("findings/") || key.starts_with("finding/")
}

pub fn is_report_key(key: &str) -> bool {
    key.starts_with("reports/") || key.starts_with("report/")
}

/// Which claims have a verdict and which have a finding.
#[derive(Debug, Default)]
pub struct GateIndex {
    /// claim key -> latest non-empty verdict, lowercased
    pub verdict_by_claim: BTreeMap<String, String>,
    /// claims that have a finding
    pub verified_claims: BTreeSet<String>,
}

pub fn index(items: &[ShelfEntry]) -> GateIndex {
    let mut idx = GateIndex::default();
    for item in items {
        let key = item.key.trim();
        let record = parse_object(item.content.as_deref());
        if is_verdict_key(key) {
            let claim = str_field(&record, "claim_key").map(|s| s.trim().to_string()).unwrap_or_default();
            let verdict = str_field(&record, "verdict").map(|s| s.trim().to_lowercase()).unwrap_or_default();
            if !claim.is_empty() && !verdict.is_empty() {
                idx.verdict_by_claim.insert(claim, verdict);
            }
        } else if is_finding_key(key) {
            // A finding with no claim_key still counts as verified work product.
            let claim = str_field(&record, "claim_key").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            idx.verified_claims.insert(claim.unwrap_or_else(|| key.to_string()));
        }
    }
    idx
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Open,
    Verified,
    Returned,
}

impl Gate {
    pub fn as_str(self) -> &'static str {
        match self {
            Gate::Open => "open",
            Gate::Verified => "verified",
            Gate::Returned => "returned",
        }
    }
}

pub fn gate_status_for_claim(claim_key: &str, idx: &GateIndex, record_status: Option<&str>) -> Gate {
    let from_claim = record_status.unwrap_or("").trim().to_lowercase();
    let verdict = idx.verdict_by_claim.get(claim_key).map(|v| v.trim().to_lowercase()).unwrap_or_default();
    if RETURNED.contains(&from_claim.as_str()) || RETURNED.contains(&verdict.as_str()) {
        Gate::Returned
    } else if idx.verified_claims.contains(claim_key) {
        Gate::Verified
    } else {
        Gate::Open
    }
}

/// A verdict node's label: keep the reviewer's decision, but "confirmed" reads
/// `review-ok` until the finding lands, then `verified`.
pub fn display_status_for_verdict(verdict: &str, claim_key: Option<&str>, idx: &GateIndex) -> String {
    let v = verdict.trim().to_lowercase();
    match (v.as_str(), claim_key) {
        ("confirmed", Some(c)) if idx.verified_claims.contains(c) => "verified".to_string(),
        ("confirmed", Some(_)) => "review-ok".to_string(),
        ("", _) => "verdict".to_string(),
        _ => v,
    }
}

/// Counts of what the shelf holds, for a one-line summary.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Counts {
    pub open: usize,
    pub verified: usize,
    pub returned: usize,
    pub reports: usize,
}

pub fn count(items: &[ShelfEntry]) -> Counts {
    let idx = index(items);
    let mut counts = Counts::default();
    let mut seen_claims = BTreeSet::new();
    for item in items {
        let key = item.key.trim();
        let record = parse_object(item.content.as_deref());
        if is_claim_key(key) {
            seen_claims.insert(key.to_string());
            match gate_status_for_claim(key, &idx, str_field(&record, "status").as_deref()) {
                Gate::Open => counts.open += 1,
                Gate::Returned => counts.returned += 1,
                Gate::Verified => {} // counted through its finding
            }
        } else if is_finding_key(key) {
            counts.verified += 1;
        } else if is_report_key(key) {
            counts.reports += 1;
        }
    }
    // A returned verdict whose claim is gone, or that names none, still counts once.
    for item in items.iter().filter(|i| is_verdict_key(i.key.trim())) {
        let record = parse_object(item.content.as_deref());
        let verdict = str_field(&record, "verdict").unwrap_or_default().to_lowercase();
        let claim = str_field(&record, "claim_key").map(|s| s.trim().to_string()).unwrap_or_default();
        let accounted = !claim.is_empty() && (seen_claims.contains(&claim) || idx.verified_claims.contains(&claim));
        if RETURNED.contains(&verdict.as_str()) && !accounted {
            counts.returned += 1;
        }
    }
    counts
}

fn last_segment(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// Find the claim key a node stands for: by resource key, by its own id, or by
/// slug against the claims the shelf knows about.
fn resolve_claim_key(node: &Node, idx: &GateIndex, items: &[ShelfEntry]) -> Option<String> {
    if let Some(rk) = node.resource_key.as_deref().filter(|k| is_claim_key(k)) {
        return Some(rk.to_string());
    }
    if is_claim_key(&node.id) {
        return Some(node.id.clone());
    }
    let slug = if node.id.contains('/') { last_segment(&node.id) } else { node.id.as_str() };
    let hits = |key: &str| key == node.id || key.ends_with(&format!("/{slug}")) || last_segment(key) == slug;
    idx.verdict_by_claim
        .keys()
        .find(|k| hits(k))
        .or_else(|| idx.verified_claims.iter().find(|k| hits(k)))
        .cloned()
        .or_else(|| {
            items
                .iter()
                .find(|i| is_claim_key(&i.key) && (i.key == node.id || i.key.ends_with(&format!("/{slug}"))))
                .map(|i| i.key.clone())
        })
}

/// A display name for a shelf row: its name, title, summary, or last key segment.
fn title_of(key: &str, record: &Option<serde_json::Map<String, Value>>) -> String {
    for field in ["name", "title", "summary"] {
        if let Some(value) = str_field(record, field) {
            if !value.is_empty() {
                return value;
            }
        }
    }
    last_segment(key).to_string()
}

/// One row for a status menu: a gate record classified for display.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StatusItem {
    pub key: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// open, verified, returned or report.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
}

/// Classify the shelf's gate rows for a status menu. A verified claim is shown
/// through its finding, not twice; a returned verdict surfaces only when its
/// claim is missing or not already shown. Other keys are ignored.
pub fn classify_shelf_items(items: &[ShelfEntry]) -> Vec<StatusItem> {
    let idx = index(items);
    let mut out = Vec::new();
    let mut seen_claims = BTreeSet::new();

    for item in items {
        let key = item.key.trim();
        let record = parse_object(item.content.as_deref());
        if is_claim_key(key) {
            seen_claims.insert(key.to_string());
            let status = gate_status_for_claim(key, &idx, str_field(&record, "status").as_deref());
            if status == Gate::Verified {
                continue; // shown through its finding
            }
            out.push(StatusItem {
                key: key.to_string(),
                title: title_of(key, &record),
                severity: str_field(&record, "severity"),
                status: status.as_str().to_string(),
                host: str_field(&record, "host"),
                audience: None,
            });
        } else if is_finding_key(key) {
            out.push(StatusItem {
                key: key.to_string(),
                title: title_of(key, &record),
                severity: str_field(&record, "severity"),
                status: "verified".to_string(),
                host: str_field(&record, "host"),
                audience: None,
            });
        } else if is_report_key(key) {
            out.push(StatusItem {
                key: key.to_string(),
                title: title_of(key, &record),
                severity: str_field(&record, "severity_summary").or_else(|| str_field(&record, "go_no_go")),
                status: "report".to_string(),
                host: None,
                audience: str_field(&record, "audience"),
            });
        } else if is_verdict_key(key) {
            let verdict = str_field(&record, "verdict").unwrap_or_default().to_lowercase();
            let claim = str_field(&record, "claim_key").map(|s| s.trim().to_string()).unwrap_or_default();
            if !RETURNED.contains(&verdict.as_str()) {
                continue;
            }
            if !claim.is_empty() && (seen_claims.contains(&claim) || idx.verified_claims.contains(&claim)) {
                continue;
            }
            let shown_key = if claim.is_empty() { key.to_string() } else { claim.clone() };
            let title = if claim.is_empty() { title_of(key, &record) } else { last_segment(&claim).to_string() };
            out.push(StatusItem { key: shown_key, title, severity: None, status: "returned".to_string(), host: None, audience: None });
        }
    }
    out
}

/// Rewrite trajectory node statuses so the graph matches the gate: only a
/// finding makes a claim verified.
pub fn apply_gate_statuses(nodes: Vec<Node>, items: &[ShelfEntry]) -> Vec<Node> {
    let idx = index(items);
    nodes
        .into_iter()
        .map(|mut node| {
            let kind = node.kind.to_lowercase();
            match kind.as_str() {
                "claim" => match resolve_claim_key(&node, &idx, items) {
                    None => {
                        // Bots often stamp "confirmed" on a draft; strip it until the shelf proves it.
                        let raw = node.status.as_deref().unwrap_or("").to_lowercase();
                        if raw == "confirmed" || raw == "verified" {
                            node.status = Some("open".to_string());
                        }
                    }
                    Some(claim_key) => {
                        let record = items.iter().find(|i| i.key == claim_key).and_then(|i| parse_object(i.content.as_deref()));
                        let status = str_field(&record, "status").or_else(|| node.status.clone());
                        node.status = Some(gate_status_for_claim(&claim_key, &idx, status.as_deref()).as_str().to_string());
                    }
                },
                "finding" | "vuln" => node.status = Some("verified".to_string()),
                "verdict" => {
                    let synthetic = match node.resource_key.as_deref() {
                        Some(rk) if rk.starts_with("claims/") => rk.to_string(),
                        _ => node.id.replacen("verdicts/", "claims/", 1).replacen("verdict/", "claim/", 1),
                    };
                    let probe = Node { kind: "claim".to_string(), id: synthetic, ..node.clone() };
                    let mut linked = resolve_claim_key(&probe, &idx, items);
                    let record = items
                        .iter()
                        .find(|i| i.key == node.id || Some(i.key.as_str()) == node.resource_key.as_deref())
                        .and_then(|i| parse_object(i.content.as_deref()));
                    if let Some(claim) = str_field(&record, "claim_key") {
                        linked = Some(claim);
                    }
                    let label_is_returned = RETURNED.contains(&node.label.to_lowercase().as_str());
                    let raw = node
                        .status
                        .clone()
                        .filter(|s| !s.is_empty())
                        .or_else(|| label_is_returned.then(|| node.label.clone()))
                        .or_else(|| str_field(&record, "verdict"))
                        .unwrap_or_else(|| if node.label.is_empty() { "verdict".to_string() } else { node.label.clone() });
                    let status = display_status_for_verdict(&raw, linked.as_deref(), &idx);
                    if node.label == "confirmed" || node.label == "verified" {
                        node.label = status.clone();
                    }
                    node.status = Some(status);
                }
                _ => {}
            }
            node
        })
        .collect()
}

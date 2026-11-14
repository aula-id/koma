//! The reporter's dual-pack rule: every finding needs a detailed technical
//! report, plus one manager report that covers all findings.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::args::parse_object;

/// A technical report body shorter than this is "too thin".
pub const TECH_MARKDOWN_MIN: usize = 400;

#[derive(Debug, Default, PartialEq)]
pub struct PackGaps {
    pub missing_technical: Vec<String>,
    pub short_technical: Vec<String>,
    pub manager_missing_findings: Vec<String>,
    pub has_manager: bool,
}

fn is_finding(key: &str) -> bool {
    key.starts_with("findings/") || key.starts_with("finding/")
}

fn is_report(key: &str) -> bool {
    key.starts_with("reports/") || key.starts_with("report/")
}

pub fn is_relevant(key: &str) -> bool {
    is_finding(key) || is_report(key)
}

fn report_finding_keys(body: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !keys.iter().any(|k| k == s) {
            keys.push(s.to_string());
        }
    };
    if let Some(k) = body.get("finding_key").and_then(Value::as_str) {
        push(k);
    }
    if let Some(list) = body.get("finding_keys").and_then(Value::as_array) {
        for k in list.iter().filter_map(Value::as_str) {
            push(k);
        }
    }
    keys
}

/// `items` is `(key, content)` for the findings and reports on the shelf.
pub fn pack_gaps(items: &[(String, String)]) -> PackGaps {
    let findings: Vec<&str> = items.iter().map(|(k, _)| k.as_str()).filter(|k| is_finding(k)).collect();

    let mut technical: HashMap<String, Vec<usize>> = HashMap::new();
    let mut manager_covered: HashSet<String> = HashSet::new();
    let mut has_manager = false;

    for (_, content) in items.iter().filter(|(k, _)| is_report(k)) {
        let Some(body) = parse_object(content) else {
            continue;
        };
        if body.get("kind").and_then(Value::as_str) != Some("report") {
            continue;
        }
        let audience = body.get("audience").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let markdown_len = body.get("markdown").and_then(Value::as_str).map_or(0, |m| m.trim().chars().count());
        let linked = report_finding_keys(&body);
        match audience.as_str() {
            "technical" => {
                for finding in linked {
                    technical.entry(finding).or_default().push(markdown_len);
                }
            }
            "manager" => {
                has_manager = true;
                manager_covered.extend(linked);
            }
            _ => {}
        }
    }

    let mut gaps = PackGaps { has_manager, ..PackGaps::default() };
    for finding in &findings {
        match technical.get(*finding) {
            None => gaps.missing_technical.push((*finding).to_string()),
            Some(lens) if !lens.iter().any(|l| *l >= TECH_MARKDOWN_MIN) => gaps.short_technical.push(format!(
                "{finding} (technical report markdown < {TECH_MARKDOWN_MIN} chars — need DETAILED body)"
            )),
            Some(_) => {}
        }
    }
    gaps.manager_missing_findings = if has_manager {
        findings
            .iter()
            .filter(|f| !manager_covered.contains(**f))
            .map(|f| (*f).to_string())
            .collect()
    } else {
        findings.iter().map(|f| (*f).to_string()).collect()
    };
    gaps
}

/// `None` when the pack is complete.
pub fn format_gaps(gaps: &PackGaps) -> Option<String> {
    if gaps.missing_technical.is_empty()
        && gaps.short_technical.is_empty()
        && gaps.has_manager
        && gaps.manager_missing_findings.is_empty()
    {
        return None;
    }
    let mut parts = Vec::new();
    if !gaps.missing_technical.is_empty() {
        parts.push(format!(
            "missing audience=technical report for: {} (prefer reports/<same path as findings/>)",
            gaps.missing_technical.join(", ")
        ));
    }
    if !gaps.short_technical.is_empty() {
        parts.push(format!("technical reports too thin: {}", gaps.short_technical.join("; ")));
    }
    if !gaps.has_manager {
        parts.push("missing audience=manager executive pack under reports/ (prefer reports/<goal>/executive)".to_string());
    } else if !gaps.manager_missing_findings.is_empty() {
        parts.push(format!(
            "manager pack missing finding_keys coverage for: {}",
            gaps.manager_missing_findings.join(", ")
        ));
    }
    Some(format!(
        "Reporter dual pack incomplete. {}. Write DETAILED technical per finding, then manager executive.",
        parts.join(" ")
    ))
}

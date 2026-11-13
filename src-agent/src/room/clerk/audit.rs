//! The clerk's deterministic checks.
//!
//! Anything a program can establish, the program establishes: a hallucinating
//! model must not be the thing that says a host is out of scope. These checks
//! run on every clerk tick, cost nothing, and hand their findings to the
//! clerk's model pass, which judges them, adds what only judgement can see, and
//! decides what is worth raising with the Chief.
//!
//! Unusual tool use is left to the model pass. A claim with no evidence, and
//! two claims for the same host and class that disagree on severity, are
//! checked here. The shelf already refuses a citation of a key that is not
//! there.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::scope::{in_scope, normalize_host, parse_scopes};
use crate::room::lane::{Digest, Phase};
use crate::room::shelf::Shelf;
use crate::room::types::{AssignmentKind, AssignmentStatus};
use crate::room::work::Assignment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    /// The Chief should halt the lane.
    Stop,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Stop => "stop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Severity::Info),
            "warn" => Some(Severity::Warn),
            "stop" => Some(Severity::Stop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    OutOfScope,
    NoScopeRecord,
    Stalled,
    Unpublished,
    RepeatedWork,
    /// A claim names no evidence.
    MissingEvidence,
    /// Two claims for the same host and class disagree on severity.
    Contradiction,
    /// Verification keeps being superseded and never lands a verdict.
    ReviewLoop,
    /// Findings are confirmed and nobody is writing the report.
    ReportDue,
}

/// One thing the audit noticed.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Identifies the problem, so it is reported once until it clears.
    pub key: String,
    pub kind: FindingKind,
    /// The lane involved, when one can be named.
    pub avatar: Option<String>,
    pub severity: Severity,
    pub note: String,
    /// Shelf keys that show it.
    pub refs: Vec<String>,
}

/// Limits for the time- and count-based checks.
#[derive(Debug, Clone, Copy)]
pub struct AuditConfig {
    /// A running lane silent for this long is stalled.
    pub stall_ms: u64,
    /// A lane this many steps in with nothing published is not sharing its work.
    pub unpublished_steps: usize,
    /// The same task given this many times is a loop.
    pub repeat_limit: usize,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self { stall_ms: 5 * 60 * 1000, unpublished_steps: 6, repeat_limit: 3 }
    }
}

/// What the audit looks at.
pub struct AuditInput<'a> {
    pub shelf: &'a Shelf,
    pub digest: &'a Digest,
    /// The active work's assignments.
    pub assignments: &'a [Assignment],
    /// Lanes the user paused; a paused lane is quiet on purpose.
    pub paused: &'a BTreeSet<String>,
    pub now_ms: u64,
    pub config: AuditConfig,
}

fn host_of(entry_content: &str) -> Option<String> {
    let body: Value = serde_json::from_str(entry_content).ok()?;
    body.get("host").and_then(Value::as_str).map(str::to_string)
}

fn short_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes())).chars().take(8).collect()
}

/// Run every deterministic check. The result is ordered, most severe first.
pub fn audit(input: &AuditInput<'_>) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    let scopes_present = check_scope(input, &mut findings)?;
    check_claims(input, scopes_present, &mut findings)?;
    check_lanes(input, &mut findings);
    check_repeats(input, &mut findings);
    check_review_loop(input, &mut findings);
    check_report_due(input, &mut findings)?;
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.key.cmp(&b.key)));
    Ok(findings)
}

fn check_scope(input: &AuditInput<'_>, out: &mut Vec<Finding>) -> Result<bool, String> {
    let shelf_err = |e: crate::room::shelf::ShelfError| e.0;
    let scopes = parse_scopes(&input.shelf.scan("scope/").map_err(shelf_err)?);
    let mut targets = input.shelf.scan("claims/").map_err(shelf_err)?;
    targets.extend(input.shelf.scan("assets/").map_err(shelf_err)?);
    if targets.is_empty() {
        return Ok(false);
    }
    if scopes.is_empty() {
        out.push(Finding {
            key: "no-scope-record".to_string(),
            kind: FindingKind::NoScopeRecord,
            avatar: None,
            severity: Severity::Warn,
            note: "Claims or assets are being published but no scope record exists, so nothing can be checked against the authorised targets.".to_string(),
            refs: Vec::new(),
        });
        return Ok(false);
    }
    for entry in targets {
        let Some(host) = host_of(entry.content.as_deref().unwrap_or("")) else {
            continue;
        };
        if !in_scope(&host, &scopes) {
            out.push(Finding {
                key: format!("out-of-scope:{}", entry.key),
                kind: FindingKind::OutOfScope,
                avatar: input.digest.publisher_of(&entry.key).map(str::to_string),
                severity: Severity::Stop,
                note: format!("{} targets {host}, which no scope record covers.", entry.key),
                refs: vec![entry.key.clone()],
            });
        }
    }
    Ok(true)
}

fn check_claims(input: &AuditInput<'_>, scopes_present: bool, out: &mut Vec<Finding>) -> Result<(), String> {
    let shelf_err = |e: crate::room::shelf::ShelfError| e.0;
    let claims = input.shelf.scan("claims/").map_err(shelf_err)?;
    if claims.is_empty() {
        return Ok(());
    }
    let mut by_subject: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for entry in &claims {
        let Some(body) = entry.content.as_deref().and_then(|raw| serde_json::from_str::<Value>(raw).ok()) else {
            continue;
        };
        if scopes_present && string_list(&body, "evidence_keys").is_empty() {
            out.push(Finding {
                key: format!("no-evidence:{}", entry.key),
                kind: FindingKind::MissingEvidence,
                avatar: input.digest.publisher_of(&entry.key).map(str::to_string),
                severity: Severity::Warn,
                note: format!("{} names no evidence.", entry.key),
                refs: vec![entry.key.clone()],
            });
        }
        let host = body.get("host").and_then(Value::as_str).map(normalize_host).unwrap_or_default();
        let class = body.get("vulnclass").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let severity = body.get("severity").and_then(Value::as_str).unwrap_or("").to_lowercase();
        if !host.is_empty() && !class.is_empty() && !severity.is_empty() {
            by_subject.entry((host, class)).or_default().insert(severity);
        }
    }
    for ((host, class), severities) in by_subject {
        if severities.len() < 2 {
            continue;
        }
        let list = severities.into_iter().collect::<Vec<_>>().join(", ");
        out.push(Finding {
            key: format!("contradict:{host}:{class}"),
            kind: FindingKind::Contradiction,
            avatar: None,
            severity: Severity::Warn,
            note: format!("Claims for {host} ({class}) disagree on severity: {list}."),
            refs: Vec::new(),
        });
    }
    Ok(())
}

fn string_list(body: &Value, field: &str) -> Vec<String> {
    body.get(field)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn check_lanes(input: &AuditInput<'_>, out: &mut Vec<Finding>) {
    let pending_kind: BTreeMap<&str, AssignmentKind> = input
        .assignments
        .iter()
        .filter(|a| a.status.is_pending())
        .map(|a| (a.avatar.as_str(), a.kind))
        .collect();
    for avatar in input.digest.avatars() {
        let Some(lane) = input.digest.status(avatar) else { continue };
        let Some(kind) = pending_kind.get(avatar) else { continue };
        if lane.phase != Phase::Running || input.paused.contains(avatar) {
            continue;
        }
        if let Some(last) = lane.last_event_ms {
            let quiet = input.now_ms.saturating_sub(last);
            if quiet >= input.config.stall_ms {
                out.push(Finding {
                    key: format!("stalled:{avatar}"),
                    kind: FindingKind::Stalled,
                    avatar: Some(avatar.to_string()),
                    severity: Severity::Warn,
                    note: format!("{avatar} has been silent for {} minutes while its {} assignment is running.", quiet / 60_000, kind_name(*kind)),
                    refs: Vec::new(),
                });
            }
        }
        if lane.steps >= input.config.unpublished_steps && lane.published.is_empty() {
            out.push(Finding {
                key: format!("unpublished:{avatar}"),
                kind: FindingKind::Unpublished,
                avatar: Some(avatar.to_string()),
                severity: Severity::Warn,
                note: format!("{avatar} is {} steps into its {} assignment and has published nothing to the shelf; work that is not on the shelf is lost when its turn ends.", lane.steps, kind_name(*kind)),
                refs: Vec::new(),
            });
        }
    }
}

fn kind_name(kind: AssignmentKind) -> &'static str {
    kind.as_str()
}

fn check_repeats(input: &AuditInput<'_>, out: &mut Vec<Finding>) {
    let mut counts: BTreeMap<(String, &'static str, String), usize> = BTreeMap::new();
    for a in input.assignments.iter().filter(|a| a.status != AssignmentStatus::Superseded) {
        *counts.entry((a.avatar.clone(), a.kind.as_str(), a.prompt.clone())).or_default() += 1;
    }
    for ((avatar, kind, prompt), n) in counts {
        if n >= input.config.repeat_limit {
            out.push(Finding {
                key: format!("repeated:{avatar}:{kind}:{}", short_hash(&prompt)),
                kind: FindingKind::RepeatedWork,
                avatar: Some(avatar.clone()),
                severity: Severity::Warn,
                note: format!("{avatar} has been given the same {kind} assignment {n} times; the Chief may be looping."),
                refs: Vec::new(),
            });
        }
    }
}

/// A review that is superseded again and again is not progress: the evidence under it keeps moving.
fn check_review_loop(input: &AuditInput<'_>, out: &mut Vec<Finding>) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for a in input.assignments.iter().filter(|a| a.kind == AssignmentKind::Verification && a.status == AssignmentStatus::Superseded) {
        *counts.entry(a.avatar.as_str()).or_default() += 1;
    }
    for (avatar, n) in counts {
        if n >= input.config.repeat_limit {
            out.push(Finding {
                key: format!("review-loop:{avatar}"),
                kind: FindingKind::ReviewLoop,
                avatar: Some(avatar.to_string()),
                severity: Severity::Warn,
                note: format!("{avatar}'s verification has been superseded {n} times without a settled verdict: something it reviews keeps changing (a claim or its evidence is being rewritten). Stop re-queuing the same review; find what moves, or settle the verdicts that are final."),
                refs: Vec::new(),
            });
        }
    }
}

/// Confirmed findings with no report under way. The goal cannot finish without `reports/`, and the
/// Reviewer is done once its verdicts and findings are in: the hand-over is to the Reporter.
fn check_report_due(input: &AuditInput<'_>, out: &mut Vec<Finding>) -> Result<(), String> {
    let shelf_err = |e: crate::room::shelf::ShelfError| e.0;
    let findings = input.shelf.scan("findings/").map_err(shelf_err)?;
    if findings.is_empty() || !input.shelf.scan("reports/").map_err(shelf_err)?.is_empty() {
        return Ok(());
    }
    let reviewing = input.assignments.iter().any(|a| a.kind == AssignmentKind::Verification && a.status.is_pending());
    let reporting = input.assignments.iter().any(|a| a.kind == AssignmentKind::Report && (a.status.is_pending() || a.status == AssignmentStatus::Completed));
    if reviewing || reporting {
        return Ok(());
    }
    out.push(Finding {
        key: "report-due".into(),
        kind: FindingKind::ReportDue,
        avatar: Some("reporter".into()),
        severity: Severity::Warn,
        note: format!("{} confirmed finding(s) are on the shelf, no review is running and no report is being written. Hand them to the reporter (a report assignment that follows the verification); the goal cannot finish without reports/.", findings.len()),
        refs: findings.iter().take(8).map(|e| e.key.clone()).collect(),
    });
    Ok(())
}

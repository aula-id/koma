//! The Clerk: an automatic sanity check over the work lanes.
//!
//! It speaks only to the Chief. It never talks to the user or to another lane,
//! has no power to pause or stop anything, and never edits anyone's work. When
//! something needs attention it raises a flag; the Chief decides what to do.
//!
//! Two parts:
//!
//! - [`audit`]: deterministic checks (scope, stalls, unshared work, loops)
//!   that run on every tick at no cost.
//! - [`Clerk`]: decides when a model pass is worth running, and turns the
//!   clerk lane's `clerk_flag` call into a Chief inbox item.
//!
//! Orpheus's clerk is the model for the rhythm: it runs only while work is
//! active, never twice at once, and a quiet room costs nothing because a pass
//! starts only when something changed.

pub mod audit;
pub mod scope;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::room::lane::InboxItem;
use crate::room::registry::Registry;
use crate::room::shelf::Shelf;

#[allow(unused_imports)]
pub use audit::{audit, AuditConfig, AuditInput, Finding, FindingKind, Severity};

/// How often the actor should call [`Clerk::tick`].
pub const CLERK_TICK_MS: u64 = 4_000;
/// Least time between model passes when only the shelf changed.
pub const MIN_PASS_GAP_MS: u64 = 30_000;

/// What the actor should do after a tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Tick {
    /// No active work: nothing to watch.
    Idle,
    /// A pass is already running.
    Busy,
    /// Nothing new since the last pass.
    Quiet,
    /// Start a pass over these findings.
    Run(Pass),
}

/// One model pass the actor should start on the clerk's work lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    /// Findings the audit has not reported before, most severe first.
    pub new_findings: Vec<Finding>,
    /// Every finding currently standing, for context.
    pub all_findings: Vec<Finding>,
}

/// The clerk's memory between ticks.
#[derive(Default)]
pub struct Clerk {
    reported: BTreeSet<String>,
    last_fingerprint: Option<String>,
    last_pass_ms: Option<u64>,
    in_flight: bool,
    flag_seq: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FlagInput {
    avatar: String,
    severity: String,
    note: String,
    #[serde(default)]
    refs: Vec<String>,
}

fn fingerprint(input: &AuditInput<'_>) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let shelf = input.shelf.fingerprint().map_err(|e| e.0)?;
    let mut hasher = Sha256::new();
    hasher.update(shelf.as_bytes());
    for a in input.assignments {
        hasher.update(a.id.as_bytes());
        hasher.update(format!("{:?}", a.status).as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

impl Clerk {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide whether to start a model pass.
    ///
    /// A pass runs when the audit has a finding it has not reported, or when
    /// the shelf or the assignments changed and the last pass was not too
    /// recent. A finding that clears is forgotten, so it is reported again if
    /// it comes back. Marks the pass in flight; call [`Clerk::finish_pass`]
    /// when it ends.
    pub fn tick(&mut self, input: &AuditInput<'_>) -> Result<Tick, String> {
        if input.assignments.is_empty() {
            return Ok(Tick::Idle);
        }
        if self.in_flight {
            return Ok(Tick::Busy);
        }
        let all = audit(input)?;
        let standing: BTreeSet<&str> = all.iter().map(|f| f.key.as_str()).collect();
        self.reported.retain(|key| standing.contains(key.as_str()));
        let new_findings: Vec<Finding> = all.iter().filter(|f| !self.reported.contains(&f.key)).cloned().collect();

        let print = fingerprint(input)?;
        let changed = self.last_fingerprint.as_deref() != Some(print.as_str());
        let spaced = self.last_pass_ms.is_none_or(|t| input.now_ms.saturating_sub(t) >= MIN_PASS_GAP_MS);
        if new_findings.is_empty() && !(changed && spaced) {
            return Ok(Tick::Quiet);
        }
        self.reported.extend(new_findings.iter().map(|f| f.key.clone()));
        self.last_fingerprint = Some(print);
        self.last_pass_ms = Some(input.now_ms);
        self.in_flight = true;
        Ok(Tick::Run(Pass { new_findings, all_findings: all }))
    }

    /// The pass ended (finished, failed or was stopped).
    pub fn finish_pass(&mut self) {
        self.in_flight = false;
    }

    pub fn is_busy(&self) -> bool {
        self.in_flight
    }

    /// The text that starts the clerk's work lane for this pass.
    pub fn brief(registry: &Registry, pass: &Pass, digest: &str) -> Result<String, String> {
        let findings = if pass.all_findings.is_empty() {
            "- nothing".to_string()
        } else {
            pass.all_findings
                .iter()
                .map(|f| {
                    let new = if pass.new_findings.iter().any(|n| n.key == f.key) { " [new]" } else { "" };
                    format!("- [{}]{new} {}", f.severity.as_str(), f.note)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let digest = if digest.trim().is_empty() { "(no lanes yet)\n" } else { digest };
        registry
            .render("clerk_pass", &[("findings", &findings), ("digest", digest)])
            .map_err(|e| e.to_string())
    }

    /// Handle the clerk lane's `clerk_flag` call: record it on the shelf as an
    /// audit note and produce the Chief inbox item. The clerk can flag a
    /// specialist lane only; there is no other recipient.
    pub fn flag(&mut self, registry: &Registry, shelf: &Shelf, args: &Value, now_ms: u64) -> Result<(Value, InboxItem), String> {
        let input: FlagInput = serde_json::from_value(args.clone()).map_err(|e| format!("invalid arguments: {e}"))?;
        let Some(severity) = Severity::parse(&input.severity) else {
            return Err("severity must be info, warn or stop".to_string());
        };
        let note = input.note.trim();
        if note.is_empty() || note.chars().count() > 2000 {
            return Err("note must be 1..2000 chars".to_string());
        }
        if input.refs.len() > 16 || input.refs.iter().any(|r| r.is_empty() || r.chars().count() > 128) {
            return Err("refs must be at most 16 keys of 1..128 chars".to_string());
        }
        let role = registry.avatar(&input.avatar).map(|a| a.role.as_str());
        if !role.is_some_and(|r| matches!(r, "red" | "reviewer" | "reporter")) {
            return Err("The Clerk flags specialist lanes only".to_string());
        }

        self.flag_seq += 1;
        let key = format!("notes/clerk/flag-{now_ms}-{}", self.flag_seq);
        let record = json!({
            "kind": "clerk_flag", "avatar": input.avatar, "severity": severity.as_str(),
            "note": note, "refs": input.refs, "at_ms": now_ms,
        });
        shelf
            .call("put", &json!({ "key": key, "content": record.to_string(), "expected_revision": 0 }))
            .map_err(|e| e.0)?;

        let text = if input.refs.is_empty() { note.to_string() } else { format!("{note} (see {})", input.refs.join(", ")) };
        Ok((
            json!({ "ok": true, "note": "Flag sent to the Chief.", "recorded": key }),
            InboxItem::ClerkFlag { avatar: input.avatar, severity: severity.as_str().to_string(), note: text },
        ))
    }
}

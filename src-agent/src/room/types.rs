//! Vocabulary shared by the room: assignment kinds and statuses, outcomes, and
//! the `{key, revision}` references that results cite.
//!
//! Ported from Orpheus `group-work.ts`, restricted to the Lab team (no
//! builder / `execution` kind).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// What a specialist is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentKind {
    /// Red probes the target and publishes assets, claims and evidence.
    Probe,
    /// Reviewer looks into something without producing a verdict.
    Investigation,
    /// Reviewer independently re-checks cited claim/evidence revisions.
    Verification,
    /// Reporter writes the report pack from verified findings.
    Report,
}

impl AssignmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AssignmentKind::Probe => "probe",
            AssignmentKind::Investigation => "investigation",
            AssignmentKind::Verification => "verification",
            AssignmentKind::Report => "report",
        }
    }

    /// The only role allowed to take this kind of assignment.
    pub fn required_role(self) -> &'static str {
        match self {
            AssignmentKind::Probe => "red",
            AssignmentKind::Investigation | AssignmentKind::Verification => "reviewer",
            AssignmentKind::Report => "reporter",
        }
    }

    /// Production work is what verification has to cover before finishing.
    pub fn is_production(self) -> bool {
        matches!(self, AssignmentKind::Probe)
    }

    /// Only these kinds may carry references to earlier results.
    pub fn may_reference(self) -> bool {
        matches!(self, AssignmentKind::Verification | AssignmentKind::Report)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Queued,
    Running,
    /// The specialist submitted a result; the lane has not ended yet.
    Submitted,
    WaitingInput,
    WaitingTakeover,
    Completed,
    Failed,
    Cancelled,
    /// Could not be proven finished or stopped; blocks finish and retry.
    Uncertain,
    /// Replaced by a retry or a newer verification.
    Superseded,
}

impl AssignmentStatus {
    /// Still occupying its specialist.
    pub fn is_pending(self) -> bool {
        matches!(
            self,
            AssignmentStatus::Queued
                | AssignmentStatus::Running
                | AssignmentStatus::Submitted
                | AssignmentStatus::WaitingInput
                | AssignmentStatus::WaitingTakeover
        )
    }

    /// Needs the Chief's attention rather than just waiting.
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            AssignmentStatus::Failed
                | AssignmentStatus::Uncertain
                | AssignmentStatus::WaitingInput
                | AssignmentStatus::WaitingTakeover
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Active,
    Completed,
    Cancelled,
}

/// A pointer to an exact shelf revision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reference {
    pub key: String,
    pub revision: u64,
}

pub const MAX_REFERENCES: usize = 32;

/// References must be at most 32, have a key of 1..128 chars, a positive
/// revision, and unique keys.
pub fn validate_references(refs: &[Reference]) -> Result<(), String> {
    if refs.len() > MAX_REFERENCES {
        return Err(format!("at most {MAX_REFERENCES} references"));
    }
    let mut seen = BTreeSet::new();
    for r in refs {
        if r.key.is_empty() || r.key.chars().count() > 128 {
            return Err("reference key must be 1..128 chars".to_string());
        }
        if r.revision < 1 {
            return Err("reference revision must be positive".to_string());
        }
        if !seen.insert(r.key.as_str()) {
            return Err("Shared result keys must be unique".to_string());
        }
    }
    Ok(())
}

/// Every reference in `required` is present in `provided` at the same or a
/// newer revision.
pub fn covers_references(required: &[Reference], provided: &[Reference]) -> bool {
    required
        .iter()
        .all(|r| provided.iter().any(|p| p.key == r.key && p.revision >= r.revision))
}

/// Same set of keys at exactly the same revisions.
pub fn same_references(a: &[Reference], b: &[Reference]) -> bool {
    a.len() == b.len() && a.iter().all(|r| b.iter().any(|p| p.key == r.key && p.revision == r.revision))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(key: &str, revision: u64) -> Reference {
        Reference { key: key.into(), revision }
    }

    #[test]
    fn kinds_map_to_the_only_role_that_may_take_them() {
        assert_eq!(AssignmentKind::Probe.required_role(), "red");
        assert_eq!(AssignmentKind::Investigation.required_role(), "reviewer");
        assert_eq!(AssignmentKind::Verification.required_role(), "reviewer");
        assert_eq!(AssignmentKind::Report.required_role(), "reporter");
        assert!(AssignmentKind::Probe.is_production());
        assert!(!AssignmentKind::Verification.is_production());
        assert!(AssignmentKind::Verification.may_reference() && AssignmentKind::Report.may_reference());
        assert!(!AssignmentKind::Probe.may_reference());
    }

    #[test]
    fn pending_and_attention_statuses() {
        use AssignmentStatus::*;
        for s in [Queued, Running, Submitted, WaitingInput, WaitingTakeover] {
            assert!(s.is_pending(), "{s:?}");
        }
        for s in [Completed, Failed, Cancelled, Uncertain, Superseded] {
            assert!(!s.is_pending(), "{s:?}");
        }
        for s in [Failed, Uncertain, WaitingInput, WaitingTakeover] {
            assert!(s.needs_attention(), "{s:?}");
        }
        assert!(!Running.needs_attention() && !Completed.needs_attention());
    }

    #[test]
    fn references_must_be_unique_positive_and_bounded() {
        assert!(validate_references(&[r("a", 1), r("b", 2)]).is_ok());
        assert!(validate_references(&[r("a", 1), r("a", 2)]).is_err());
        assert!(validate_references(&[r("a", 0)]).is_err());
        assert!(validate_references(&[r("", 1)]).is_err());
        let many: Vec<_> = (0..33).map(|i| r(&format!("k{i}"), 1)).collect();
        assert!(validate_references(&many).is_err());
    }

    #[test]
    fn coverage_accepts_newer_revisions_but_never_older_ones() {
        assert!(covers_references(&[r("a", 2)], &[r("a", 2), r("b", 1)]));
        assert!(covers_references(&[r("a", 2)], &[r("a", 3)]), "a teammate's newer revision still covers");
        assert!(!covers_references(&[r("a", 2)], &[r("a", 1)]));
        assert!(!covers_references(&[r("a", 1)], &[r("b", 1)]));
        assert!(covers_references(&[], &[]));
        assert!(same_references(&[r("a", 1), r("b", 2)], &[r("b", 2), r("a", 1)]));
        assert!(!same_references(&[r("a", 1)], &[r("a", 2)]));
    }
}

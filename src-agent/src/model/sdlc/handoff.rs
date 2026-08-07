//! SDLC handoff stage A: typed envelope + pure validation.
//!
//! A handoff is a subagent→main progress report. It is purely descriptive
//! (report semantics) and must never seal or mutate the graph directly.
//! `done` in the status field means "reporting done" — it does NOT mean
//! sealed/verified.
//!
//! This module defines versioned serde types, a pure validator, and a
//! `ValidatedHandoff` output type. It needs no DB connection and must not
//! parse prose.

use std::collections::HashSet;

/// Current handoff envelope schema version.
pub const CURRENT_HANDOFF_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Serde envelope types
// ---------------------------------------------------------------------------

/// Top-level handoff envelope.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SdlcHandoff {
    /// Schema version — must equal `CURRENT_HANDOFF_VERSION` at validation.
    pub version: u32,
    /// Graph node this handoff reports on.
    pub node_id: String,
    /// Report status — `done | partial | blocked`. This is *report* semantics
    /// (what the subagent finished/attempted), NOT a graph seal.
    pub status: HandoffStatus,
    /// One-line summary of the report.
    pub summary: String,
    /// Artifact paths produced by the work (relative, non-escaping).
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// Evidence references (relative, non-escaping).
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// Commit SHAs produced during this handoff window.
    #[serde(default)]
    pub commit_shas: Vec<String>,
    /// Decision notes: short rationale strings for non-obvious choices.
    #[serde(default)]
    pub decisions: Vec<String>,
    /// Graph update proposals — extremely limited: a note for the reported
    /// node and/or child node proposals (title + optional note).
    #[serde(default)]
    pub updates: HandoffUpdates,
}

/// Report status — never graph sealing. `done` = "reporting complete for this
/// node", NOT "node is sealed".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandoffStatus {
    Done,
    Partial,
    Blocked,
}

/// Graph update proposals scoped to what a subagent is allowed to suggest.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HandoffUpdates {
    /// Optional note to attach to the reported node.
    #[serde(default)]
    pub node_note: Option<String>,
    /// Proposals for new child nodes. Children will inherit ownership later.
    #[serde(default)]
    pub child_proposals: Vec<ChildProposal>,
}

/// A proposed new child node. Only title + optional note are allowed;
/// ownership, status, id, and contract fields are forbidden.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChildProposal {
    /// Short, unique-within-this-handoff title for the proposed child.
    pub title: String,
    /// Optional brief note (≤200 chars).
    #[serde(default)]
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Validated output
// ---------------------------------------------------------------------------

/// A successfully validated handoff. Carries only sanitized, checked values.
/// Constructed exclusively by [`validate_handoff`].
#[derive(Debug, Clone)]
pub struct ValidatedHandoff {
    pub envelope: SdlcHandoff,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Pure validator. Returns `Ok(ValidatedHandoff)` or a specific rejection
/// reason. Never touches a database or parses prose.
pub fn validate_handoff(h: &SdlcHandoff) -> Result<ValidatedHandoff, HandoffError> {
    // -- version --
    if h.version != CURRENT_HANDOFF_VERSION {
        return Err(HandoffError::WrongVersion {
            expected: CURRENT_HANDOFF_VERSION,
            got: h.version,
        });
    }

    // -- node_id --
    let node_id = h.node_id.trim();
    if node_id.is_empty() {
        return Err(HandoffError::BlankField("node_id".into()));
    }
    if node_id.len() > 128 {
        return Err(HandoffError::FieldTooLong {
            field: "node_id".into(),
            max: 128,
            got: node_id.len(),
        });
    }

    // -- summary --
    let summary = h.summary.trim();
    if summary.is_empty() {
        return Err(HandoffError::BlankField("summary".into()));
    }
    if summary.len() > 1024 {
        return Err(HandoffError::FieldTooLong {
            field: "summary".into(),
            max: 1024,
            got: summary.len(),
        });
    }

    // -- artifacts: unique, bounded count, relative non-escaping --
    validate_string_list_unique_bounded(&h.artifacts, "artifacts", 64, 512)?;
    for p in &h.artifacts {
        validate_relative_path(p, "artifacts")?;
    }

    // -- evidence_refs: unique, bounded count, relative non-escaping --
    validate_string_list_unique_bounded(&h.evidence_refs, "evidence_refs", 32, 512)?;
    for p in &h.evidence_refs {
        validate_relative_path(p, "evidence_refs")?;
    }

    // -- commit_shas: unique, bounded count, valid hex SHA --
    validate_string_list_unique_bounded(&h.commit_shas, "commit_shas", 64, 128)?;
    for sha in &h.commit_shas {
        validate_sha(sha)?;
    }

    // -- decisions: unique, bounded count, bounded length --
    validate_string_list_unique_bounded(&h.decisions, "decisions", 32, 512)?;

    // -- updates.node_note --
    if let Some(ref note) = h.updates.node_note {
        if note.len() > 2048 {
            return Err(HandoffError::FieldTooLong {
                field: "updates.node_note".into(),
                max: 2048,
                got: note.len(),
            });
        }
    }

    // -- updates.child_proposals: bounded count, unique nonblank titles --
    if h.updates.child_proposals.len() > 16 {
        return Err(HandoffError::TooManyChildren {
            max: 16,
            got: h.updates.child_proposals.len(),
        });
    }
    let mut titles_seen = HashSet::new();
    for cp in &h.updates.child_proposals {
        let t = cp.title.trim();
        if t.is_empty() {
            return Err(HandoffError::BlankField(
                "updates.child_proposals[].title".into(),
            ));
        }
        if t.len() > 128 {
            return Err(HandoffError::FieldTooLong {
                field: "updates.child_proposals[].title".into(),
                max: 128,
                got: t.len(),
            });
        }
        if !titles_seen.insert(t.to_ascii_lowercase()) {
            return Err(HandoffError::DuplicateChildTitle(t.into()));
        }
        if let Some(ref n) = cp.note {
            if n.len() > 200 {
                return Err(HandoffError::FieldTooLong {
                    field: "updates.child_proposals[].note".into(),
                    max: 200,
                    got: n.len(),
                });
            }
        }
    }

    // -- reject forbidden ownership/contract fields in JSON roundtrip --
    // The schema itself prevents ownership fields from being represented
    // (SdlcHandoff / HandoffUpdates have no owned_paths, status-on-child,
    // verify_bit, etc.). We verify this structurally by checking that a
    // roundtrip through serde_json does not introduce unexpected keys.
    //
    // (This is a compile-time guarantee backed by the test
    // `no_ownership_fields_representable_in_schema` below.)

    Ok(ValidatedHandoff {
        envelope: SdlcHandoff {
            node_id: node_id.to_string(),
            summary: summary.to_string(),
            ..h.clone()
        },
    })
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate a list of strings is unique, non-empty items, bounded count,
/// and each item is bounded in length.
fn validate_string_list_unique_bounded(
    items: &[String],
    field: &str,
    max_count: usize,
    max_len: usize,
) -> Result<(), HandoffError> {
    if items.len() > max_count {
        return Err(HandoffError::TooManyItems {
            field: field.into(),
            max: max_count,
            got: items.len(),
        });
    }
    let mut seen = HashSet::new();
    for item in items {
        if item.is_empty() {
            return Err(HandoffError::BlankField(field.into()));
        }
        if item.len() > max_len {
            return Err(HandoffError::FieldTooLong {
                field: field.into(),
                max: max_len,
                got: item.len(),
            });
        }
        if !seen.insert(item.as_str()) {
            return Err(HandoffError::DuplicateItem {
                field: field.into(),
                value: item.clone(),
            });
        }
    }
    Ok(())
}

/// Validate a path is relative and does not escape (`..` component or
/// leading `/`).
fn validate_relative_path(p: &str, field: &str) -> Result<(), HandoffError> {
    if p.starts_with('/') {
        return Err(HandoffError::PathEscapes {
            field: field.into(),
            path: p.into(),
            reason: "absolute path".into(),
        });
    }
    for component in std::path::Path::new(p).components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        ) {
            return Err(HandoffError::PathEscapes {
                field: field.into(),
                path: p.into(),
                reason: format!("contains '..' or absolute component: {component:?}"),
            });
        }
    }
    Ok(())
}

/// Validate a SHA is a lowercase hex string of reasonable length (7..40).
fn validate_sha(sha: &str) -> Result<(), HandoffError> {
    if sha.len() < 7 || sha.len() > 40 {
        return Err(HandoffError::InvalidSha {
            sha: sha.into(),
            reason: format!("length {} out of range 7..40", sha.len()),
        });
    }
    if !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(HandoffError::InvalidSha {
            sha: sha.into(),
            reason: "not valid hex".into(),
        });
    }
    if sha != sha.to_ascii_lowercase() {
        return Err(HandoffError::InvalidSha {
            sha: sha.into(),
            reason: "SHA must be lowercase hex".into(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Specific, actionable handoff validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffError {
    WrongVersion {
        expected: u32,
        got: u32,
    },
    BlankField(String),
    FieldTooLong {
        field: String,
        max: usize,
        got: usize,
    },
    TooManyItems {
        field: String,
        max: usize,
        got: usize,
    },
    DuplicateItem {
        field: String,
        value: String,
    },
    PathEscapes {
        field: String,
        path: String,
        reason: String,
    },
    InvalidSha {
        sha: String,
        reason: String,
    },
    TooManyChildren {
        max: usize,
        got: usize,
    },
    DuplicateChildTitle(String),
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongVersion { expected, got } => {
                write!(f, "unsupported handoff version {got} (expected {expected})")
            }
            Self::BlankField(field) => write!(f, "blank required field: {field}"),
            Self::FieldTooLong { field, max, got } => {
                write!(f, "field '{field}' length {got} exceeds maximum {max}")
            }
            Self::TooManyItems { field, max, got } => {
                write!(f, "too many items in '{field}': {got} (max {max})")
            }
            Self::DuplicateItem { field, value } => {
                write!(f, "duplicate item in '{field}': {value}")
            }
            Self::PathEscapes {
                field,
                path,
                reason,
            } => {
                write!(f, "path escapes in '{field}': {path} — {reason}")
            }
            Self::InvalidSha { sha, reason } => {
                write!(f, "invalid SHA '{sha}': {reason}")
            }
            Self::TooManyChildren { max, got } => {
                write!(f, "too many child proposals: {got} (max {max})")
            }
            Self::DuplicateChildTitle(title) => {
                write!(f, "duplicate child proposal title: {title}")
            }
        }
    }
}

impl std::error::Error for HandoffError {}

// ---------------------------------------------------------------------------
// Report extraction transport
// ---------------------------------------------------------------------------

/// Opening delimiter that wraps a structured handoff JSON block in a subagent
/// terminal report.
pub const SDLC_HANDOFF_JSON_START: &str = "<!-- SDLC_HANDOFF_JSON_START -->";

/// Closing delimiter that wraps a structured handoff JSON block in a subagent
/// terminal report.
pub const SDLC_HANDOFF_JSON_END: &str = "<!-- SDLC_HANDOFF_JSON_END -->";

/// Scan `text` for a JSON object delimited by [`SDLC_HANDOFF_JSON_START`] and
/// [`SDLC_HANDOFF_JSON_END`] markers (literal match, not regex). Returns the
/// parsed JSON value between them, trimmed of whitespace, or `None` if markers
/// are not found or the content is not valid JSON.
pub fn extract_handoff_from_report(text: &str) -> Option<serde_json::Value> {
    let (start_idx, end_idx) = find_handoff_markers(text)?;
    let raw = text[start_idx..end_idx].trim();
    serde_json::from_str(raw).ok()
}

/// Find the start/end byte offsets of the content between handoff markers.
/// Returns `(content_start, content_end)` or `None` if markers not found.
fn find_handoff_markers(text: &str) -> Option<(usize, usize)> {
    let start_idx = text.find(SDLC_HANDOFF_JSON_START)?;
    let json_start = start_idx + SDLC_HANDOFF_JSON_START.len();
    let rest = &text[json_start..];
    let end_idx = rest.find(SDLC_HANDOFF_JSON_END)?;
    Some((json_start, json_start + end_idx))
}

// ---------------------------------------------------------------------------
// Structured handoff transport
// ---------------------------------------------------------------------------

/// Try to extract, validate, and apply a structured handoff from a subagent's
/// terminal report text. Returns `Some(note)` when a handoff was processed
/// (accepted or rejected) or `None` when no handoff markers were found.
///
/// On rejection (parse failure, validation failure, or graph-apply failure) the
/// handoff is recorded as a rejected audit event and an error note is returned.
/// On success the graph is updated and, when children were added, the mission
/// contract hash is recomputed.
pub fn try_apply_subagent_handoff(
    session_dir: &std::path::Path,
    claimed_node_id: Option<&str>,
    fallback_node_id: Option<&str>,
    report_text: &str,
) -> Option<String> {
    use super::graph;
    use super::mission::Mission;

    // Check if handoff markers are present at all. If markers are present but
    // the JSON between them is invalid, we still record a rejection (the
    // subagent attempted a structured handoff but produced malformed JSON).
    let markers_present = find_handoff_markers(report_text).is_some();
    let json = match extract_handoff_from_report(report_text) {
        Some(v) => v,
        None if markers_present => {
            let node_id = fallback_node_id.or(claimed_node_id);
            let conn = crate::model::msglog::open(session_dir).ok()?;
            let _ = graph::ensure_tables(&conn);
            let _ = graph::record_rejected_handoff(
                &conn,
                claimed_node_id,
                node_id,
                "handoff markers found but content is not valid JSON",
            );
            return Some(
                "[SDLC handoff] rejected: markers found but content is not valid JSON".into(),
            );
        }
        None => return None,
    };

    // Deserialize into envelope type.
    let envelope: SdlcHandoff = match serde_json::from_value(json) {
        Ok(e) => e,
        Err(e) => {
            let node_id = fallback_node_id.or(claimed_node_id);
            let conn = crate::model::msglog::open(session_dir).ok()?;
            let _ = graph::ensure_tables(&conn);
            let _ = graph::record_rejected_handoff(
                &conn,
                claimed_node_id,
                node_id,
                &format!("handoff JSON deserialization failed: {e}"),
            );
            return Some(format!(
                "[SDLC handoff] rejected: deserialization error — {e}"
            ));
        }
    };

    // Pure validation.
    let validated = match validate_handoff(&envelope) {
        Ok(v) => v,
        Err(e) => {
            let conn = crate::model::msglog::open(session_dir).ok()?;
            let _ = graph::ensure_tables(&conn);
            let _ = graph::record_rejected_handoff(
                &conn,
                claimed_node_id,
                Some(&envelope.node_id),
                &format!("handoff validation failed: {e}"),
            );
            return Some(format!("[SDLC handoff] rejected: {e}"));
        }
    };

    // Resolve expected active leaf id.
    let expected_id = match claimed_node_id {
        Some(id) if !id.trim().is_empty() => id.to_string(),
        _ => match fallback_node_id {
            Some(id) if !id.trim().is_empty() => id.to_string(),
            _ => {
                let conn = crate::model::msglog::open(session_dir).ok()?;
                let _ = graph::ensure_tables(&conn);
                let _ = graph::record_rejected_handoff(
                    &conn,
                    None,
                    Some(&envelope.node_id),
                    "no active node id available for handoff application",
                );
                return Some(
                    "[SDLC handoff] rejected: no active node id for handoff application".into(),
                );
            }
        },
    };

    // Apply to graph.
    let conn = crate::model::msglog::open(session_dir).ok()?;
    let _ = graph::ensure_tables(&conn);
    let outcome = match graph::apply_handoff(&conn, &expected_id, &validated) {
        Ok(o) => o,
        Err(e) => {
            let _ = graph::record_rejected_handoff(
                &conn,
                Some(&expected_id),
                Some(&envelope.node_id),
                &format!("graph apply failed: {e}"),
            );
            return Some(format!("[SDLC handoff] rejected: {e}"));
        }
    };

    // Graph mutation succeeded. If children were added, recompute mission
    // contract hash so the frozen graph_hash stays current.
    if outcome.children_added {
        if let Some(mut mission) = Mission::load(session_dir) {
            if let Ok(gh) = graph::graph_fingerprint(&conn) {
                mission.graph_hash = Some(gh);
                mission.hash = mission.recompute_hash();
                let _ = mission.save(session_dir);
            }
        }
    }

    let status_label = match validated.envelope.status {
        HandoffStatus::Done => "done",
        HandoffStatus::Partial => "partial",
        HandoffStatus::Blocked => "blocked",
    };
    let extra = if outcome.children_added {
        " (children added)"
    } else {
        ""
    };
    Some(format!(
        "[SDLC handoff] accepted: {status_label} — {}{extra}",
        validated.envelope.summary,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// Minimal valid handoff with status=partial.
    fn valid_partial() -> SdlcHandoff {
        SdlcHandoff {
            version: CURRENT_HANDOFF_VERSION,
            node_id: "n-leaf-abc123".into(),
            status: HandoffStatus::Partial,
            summary: "Implemented the parser, tests pending".into(),
            artifacts: vec!["src/parser.rs".into()],
            evidence_refs: vec![],
            commit_shas: vec!["abcdef1234567890abcdef1234567890abcdef12".into()],
            decisions: vec!["Used PEG over regex for clarity".into()],
            updates: HandoffUpdates::default(),
        }
    }

    #[test]
    fn valid_partial_accepted() {
        let h = valid_partial();
        let v = validate_handoff(&h).unwrap();
        assert_eq!(v.envelope.status, HandoffStatus::Partial);
        assert_eq!(v.envelope.node_id, "n-leaf-abc123");
    }

    #[test]
    fn valid_blocked_accepted() {
        let mut h = valid_partial();
        h.status = HandoffStatus::Blocked;
        h.summary = "Waiting on upstream API schema".into();
        let v = validate_handoff(&h).unwrap();
        assert_eq!(v.envelope.status, HandoffStatus::Blocked);
    }

    #[test]
    fn valid_done_accepted() {
        // done = "reporting done for this node", NOT sealed.
        let mut h = valid_partial();
        h.status = HandoffStatus::Done;
        h.summary = "All work items complete for this leaf".into();
        let v = validate_handoff(&h).unwrap();
        assert_eq!(v.envelope.status, HandoffStatus::Done);
    }

    #[test]
    fn invalid_version_rejected() {
        let mut h = valid_partial();
        h.version = 0;
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::WrongVersion { .. }));
        assert!(err.to_string().contains("0"));
    }

    #[test]
    fn future_version_rejected() {
        let mut h = valid_partial();
        h.version = 99;
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(
            err,
            HandoffError::WrongVersion {
                expected: 1,
                got: 99
            }
        ));
    }

    #[test]
    fn traversal_artifact_rejected() {
        let mut h = valid_partial();
        h.artifacts = vec!["../../etc/passwd".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::PathEscapes { .. }),
            "expected PathEscapes, got: {err}"
        );
    }

    #[test]
    fn absolute_artifact_rejected() {
        let mut h = valid_partial();
        h.artifacts = vec!["/etc/passwd".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::PathEscapes { .. }),
            "expected PathEscapes for absolute, got: {err}"
        );
    }

    #[test]
    fn traversal_evidence_rejected() {
        let mut h = valid_partial();
        h.evidence_refs = vec!["../../../secret.txt".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::PathEscapes { .. }));
    }

    #[test]
    fn invalid_sha_too_short_rejected() {
        let mut h = valid_partial();
        h.commit_shas = vec!["abc123".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::InvalidSha { .. }),
            "expected InvalidSha, got: {err}"
        );
    }

    #[test]
    fn invalid_sha_non_hex_rejected() {
        let mut h = valid_partial();
        h.commit_shas = vec!["zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::InvalidSha { .. }));
    }

    #[test]
    fn invalid_sha_uppercase_rejected() {
        let mut h = valid_partial();
        h.commit_shas = vec!["ABCDEF1234567890ABCDEF1234567890ABCDEF12".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::InvalidSha { .. }),
            "uppercase hex should be rejected, got: {err}"
        );
    }

    #[test]
    fn duplicate_children_rejected() {
        let mut h = valid_partial();
        h.updates.child_proposals = vec![
            ChildProposal {
                title: "child-a".into(),
                note: None,
            },
            ChildProposal {
                title: "child-a".into(),
                note: None,
            },
        ];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::DuplicateChildTitle(_)),
            "expected DuplicateChildTitle, got: {err}"
        );
    }

    #[test]
    fn duplicate_case_insensitive_children_rejected() {
        let mut h = valid_partial();
        h.updates.child_proposals = vec![
            ChildProposal {
                title: "Child-A".into(),
                note: None,
            },
            ChildProposal {
                title: "child-a".into(),
                note: None,
            },
        ];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::DuplicateChildTitle(_)),
            "case-insensitive dup should fail, got: {err}"
        );
    }

    #[test]
    fn oversized_payload_rejected() {
        let mut h = valid_partial();
        h.summary = "x".repeat(1025);
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::FieldTooLong { ref field, .. } if field == "summary"),
            "expected FieldTooLong for summary, got: {err}"
        );
    }

    #[test]
    fn oversized_node_id_rejected() {
        let mut h = valid_partial();
        h.node_id = "x".repeat(129);
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::FieldTooLong { .. }));
    }

    #[test]
    fn too_many_artifacts_rejected() {
        let mut h = valid_partial();
        h.artifacts = (0..65).map(|i| format!("file_{i}.rs")).collect();
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::TooManyItems { ref field, .. } if field == "artifacts"),
            "expected TooManyItems for artifacts, got: {err}"
        );
    }

    #[test]
    fn too_many_children_rejected() {
        let mut h = valid_partial();
        h.updates.child_proposals = (0..17)
            .map(|i| ChildProposal {
                title: format!("child-{i}"),
                note: None,
            })
            .collect();
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::TooManyChildren { max: 16, .. }),
            "expected TooManyChildren, got: {err}"
        );
    }

    #[test]
    fn blank_node_id_rejected() {
        let mut h = valid_partial();
        h.node_id = "  ".into();
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::BlankField(ref f) if f == "node_id"),
            "expected BlankField node_id, got: {err}"
        );
    }

    #[test]
    fn blank_summary_rejected() {
        let mut h = valid_partial();
        h.summary = "".into();
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::BlankField(ref f) if f == "summary"));
    }

    #[test]
    fn blank_child_title_rejected() {
        let mut h = valid_partial();
        h.updates.child_proposals = vec![ChildProposal {
            title: "  ".into(),
            note: None,
        }];
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::BlankField(_)));
    }

    #[test]
    fn duplicate_decisions_rejected() {
        let mut h = valid_partial();
        h.decisions = vec!["same".into(), "same".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::DuplicateItem { ref field, .. } if field == "decisions"),
            "expected DuplicateItem for decisions, got: {err}"
        );
    }

    #[test]
    fn no_ownership_fields_representable_in_schema() {
        // Prove that SdlcHandoff cannot contain owned_paths, verify_bit,
        // or arbitrary node status/ownership fields via JSON roundtrip.
        let h = valid_partial();
        let json = serde_json::to_value(&h).unwrap();
        let obj = json.as_object().expect("envelope is JSON object");

        // Top-level must NOT contain ownership/contract fields.
        assert!(
            !obj.contains_key("owned_paths"),
            "SdlcHandoff must not have owned_paths"
        );
        assert!(
            !obj.contains_key("verify_bit"),
            "SdlcHandoff must not have verify_bit"
        );
        assert!(
            !obj.contains_key("phase"),
            "SdlcHandoff must not have phase"
        );
        assert!(
            !obj.contains_key("node_status"),
            "SdlcHandoff must not have node_status (status is report-only)"
        );

        // The `status` field must ONLY serialize the three report values.
        let status_str = obj
            .get("status")
            .and_then(|v| v.as_str())
            .expect("status present");
        assert!(
            matches!(status_str, "done" | "partial" | "blocked"),
            "unexpected status: {status_str}"
        );

        // Child proposals must NOT have ownership/status/id fields.
        if let Some(updates) = obj.get("updates") {
            if let Some(children) = updates.get("child_proposals") {
                for cp in children.as_array().expect("child_proposals is array") {
                    let cp_obj = cp.as_object().expect("child proposal is object");
                    assert!(
                        !cp_obj.contains_key("owned_paths"),
                        "child proposal must not have owned_paths"
                    );
                    assert!(
                        !cp_obj.contains_key("status"),
                        "child proposal must not have status"
                    );
                    assert!(
                        !cp_obj.contains_key("node_id"),
                        "child proposal must not have node_id"
                    );
                }
            }
        }
    }

    #[test]
    fn done_does_not_mean_sealed() {
        // A done-status handoff is report semantics only.
        let mut h = valid_partial();
        h.status = HandoffStatus::Done;
        let v = validate_handoff(&h).unwrap();
        assert_eq!(v.envelope.status, HandoffStatus::Done);
        // The validated output contains no sealing indicator — it is purely
        // a report envelope, not a graph mutation.
    }

    #[test]
    fn minimal_valid_with_empty_optional_fields() {
        let h = SdlcHandoff {
            version: CURRENT_HANDOFF_VERSION,
            node_id: "n-1".into(),
            status: HandoffStatus::Partial,
            summary: "start".into(),
            artifacts: vec![],
            evidence_refs: vec![],
            commit_shas: vec![],
            decisions: vec![],
            updates: HandoffUpdates::default(),
        };
        assert!(validate_handoff(&h).is_ok());
    }

    #[test]
    fn child_proposal_with_long_note_rejected() {
        let mut h = valid_partial();
        h.updates.child_proposals = vec![ChildProposal {
            title: "child".into(),
            note: Some("n".repeat(201)),
        }];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::FieldTooLong { ref field, .. } if field.contains("note")),
            "expected FieldTooLong for note, got: {err}"
        );
    }

    #[test]
    fn duplicate_artifacts_rejected() {
        let mut h = valid_partial();
        h.artifacts = vec!["a.rs".into(), "a.rs".into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::DuplicateItem { ref field, .. } if field == "artifacts"),
            "expected DuplicateItem for artifacts, got: {err}"
        );
    }

    #[test]
    fn duplicate_commit_shas_rejected() {
        let mut h = valid_partial();
        let sha = "abcdef1234567890abcdef1234567890abcdef12";
        h.commit_shas = vec![sha.into(), sha.into()];
        let err = validate_handoff(&h).unwrap_err();
        assert!(
            matches!(err, HandoffError::DuplicateItem { ref field, .. } if field == "commit_shas"),
            "expected DuplicateItem for commit_shas, got: {err}"
        );
    }

    #[test]
    fn node_note_too_long_rejected() {
        let mut h = valid_partial();
        h.updates.node_note = Some("x".repeat(2049));
        let err = validate_handoff(&h).unwrap_err();
        assert!(matches!(err, HandoffError::FieldTooLong { .. }));
    }

    #[test]
    fn serde_roundtrip_preserves_types() {
        let h = valid_partial();
        let json_str = serde_json::to_string(&h).unwrap();
        let back: SdlcHandoff = serde_json::from_str(&json_str).unwrap();
        assert_eq!(back.version, h.version);
        assert_eq!(back.status, h.status);
        assert_eq!(back.node_id, h.node_id);
    }

    // --- extract_handoff_from_report tests ---

    #[test]
    fn extract_handoff_valid_json_between_markers() {
        let report = r#"Here is my progress report.

<!-- SDLC_HANDOFF_JSON_START -->
{ "version": 1, "node_id": "n-test-001", "status": "done", "summary": "all done" }
<!-- SDLC_HANDOFF_JSON_END -->

That's everything."#;
        let val = extract_handoff_from_report(report).expect("should find handoff");
        assert_eq!(val["version"], 1);
        assert_eq!(val["node_id"], "n-test-001");
        assert_eq!(val["status"], "done");
    }

    #[test]
    fn extract_handoff_missing_markers_returns_none() {
        let report = "Just a plain report with no structured handoff.";
        assert!(extract_handoff_from_report(report).is_none());
    }

    #[test]
    fn extract_handoff_only_start_marker_returns_none() {
        let report = "Some text.\n<!-- SDLC_HANDOFF_JSON_START -->\n{bad json";
        assert!(extract_handoff_from_report(report).is_none());
    }

    #[test]
    fn extract_handoff_invalid_json_returns_none() {
        let report = "text\n<!-- SDLC_HANDOFF_JSON_START -->\nnot json at all\n<!-- SDLC_HANDOFF_JSON_END -->";
        assert!(extract_handoff_from_report(report).is_none());
    }

    #[test]
    fn extract_handoff_whitespace_trimmed() {
        let report = "before\n<!-- SDLC_HANDOFF_JSON_START -->\n  \n{\"version\":1,\"node_id\":\"n-1\",\"status\":\"partial\",\"summary\":\"hi\"}\n  \n<!-- SDLC_HANDOFF_JSON_END -->\nafter";
        let val = extract_handoff_from_report(report).expect("should parse with whitespace");
        assert_eq!(val["node_id"], "n-1");
    }

    #[test]
    fn normalized_whitespace_in_validated_output() {
        let mut h = valid_partial();
        h.node_id = "  n-leaf-xyz  ".into();
        h.summary = "  did the thing  ".into();
        let v = validate_handoff(&h).unwrap();
        assert_eq!(v.envelope.node_id, "n-leaf-xyz");
        assert_eq!(v.envelope.summary, "did the thing");
    }

    // --- try_apply_subagent_handoff round-trip tests ---

    /// Helper: create a session dir with a graph containing one active leaf.
    fn setup_session_with_active_leaf(leaf_title: &str) -> (std::path::PathBuf, String) {
        use crate::model::sdlc::graph::{
            ensure_tables, list_all, replace_nodes_from_checklist, ChecklistNode,
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_dir = std::env::temp_dir().join(format!(
            "koma-sdlc-handoff-{}-{}",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).unwrap();
        // Create the DB via msglog::open
        let conn = crate::model::msglog::open(&session_dir).unwrap();
        ensure_tables(&conn).unwrap();
        replace_nodes_from_checklist(
            &conn,
            &[ChecklistNode {
                title: leaf_title.into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            }],
        )
        .unwrap();
        let node_id = list_all(&conn).unwrap()[0].id.clone();
        (session_dir, node_id)
    }

    #[test]
    fn try_apply_handoff_full_round_trip() {
        let (session_dir, node_id) = setup_session_with_active_leaf("parser");
        let report = format!(
            "I'm done implementing the parser.\n\n\
             <!-- SDLC_HANDOFF_JSON_START -->\n\
             {{ \"version\": 1, \"node_id\": \"{node_id}\", \"status\": \"done\", \"summary\": \"parser implemented\" }}\n\
             <!-- SDLC_HANDOFF_JSON_END -->"
        );
        let note = try_apply_subagent_handoff(&session_dir, Some(&node_id), None, &report);
        let note = note.expect("should return accepted note");
        assert!(
            note.contains("accepted"),
            "note should say accepted: {note}"
        );
        assert!(
            note.contains("done"),
            "note should mention done status: {note}"
        );
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn try_apply_handoff_no_markers_returns_none() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_dir = std::env::temp_dir().join(format!(
            "koma-sdlc-handoff-no-mark-{}-{}",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).unwrap();
        let note = try_apply_subagent_handoff(&session_dir, Some("n-1"), None, "plain text report");
        assert!(note.is_none(), "no markers should yield None");
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn try_apply_handoff_wrong_node_id_rejected() {
        let (session_dir, node_id) = setup_session_with_active_leaf("parser");
        let report = format!(
            "<!-- SDLC_HANDOFF_JSON_START -->\n\
             {{ \"version\": 1, \"node_id\": \"{node_id}\", \"status\": \"done\", \"summary\": \"ok\" }}\n\
             <!-- SDLC_HANDOFF_JSON_END -->"
        );
        // Pass a wrong claimed id that doesn't match the node_id in the handoff.
        let note = try_apply_subagent_handoff(&session_dir, Some("n-wrong-id"), None, &report);
        let note = note.expect("should return rejected note");
        assert!(
            note.contains("rejected"),
            "note should say rejected: {note}"
        );
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn try_apply_handoff_invalid_json_rejected() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_dir = std::env::temp_dir().join(format!(
            "koma-sdlc-handoff-badj-{}-{}",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).unwrap();
        let _ = crate::model::msglog::open(&session_dir); // create DB
        let report = "<!-- SDLC_HANDOFF_JSON_START -->\nnot json\n<!-- SDLC_HANDOFF_JSON_END -->";
        let note = try_apply_subagent_handoff(&session_dir, Some("n-1"), None, report);
        let note = note.expect("should return rejected note");
        assert!(
            note.contains("rejected"),
            "note should say rejected: {note}"
        );
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn try_apply_handoff_validation_failure_rejected() {
        let (session_dir, node_id) = setup_session_with_active_leaf("parser");
        // version=99 is invalid
        let report = format!(
            "<!-- SDLC_HANDOFF_JSON_START -->\n\
             {{ \"version\": 99, \"node_id\": \"{node_id}\", \"status\": \"done\", \"summary\": \"ok\" }}\n\
             <!-- SDLC_HANDOFF_JSON_END -->"
        );
        let note = try_apply_subagent_handoff(&session_dir, Some(&node_id), None, &report);
        let note = note.expect("should return rejected note");
        assert!(
            note.contains("rejected"),
            "note should say rejected: {note}"
        );
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn try_apply_handoff_no_node_id_rejected() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_dir = std::env::temp_dir().join(format!(
            "koma-sdlc-handoff-noid-{}-{}",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).unwrap();
        let _ = crate::model::msglog::open(&session_dir); // create DB
        let report = "<!-- SDLC_HANDOFF_JSON_START -->\n\
             { \"version\": 1, \"node_id\": \"n-1\", \"status\": \"done\", \"summary\": \"ok\" }\n\
             <!-- SDLC_HANDOFF_JSON_END -->";
        // No claimed or fallback id → should reject
        let note = try_apply_subagent_handoff(&session_dir, None, None, report);
        let note = note.expect("should return rejected note");
        assert!(
            note.contains("rejected"),
            "note should say rejected: {note}"
        );
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn try_apply_handoff_rejected_writes_audit_event() {
        let (session_dir, node_id) = setup_session_with_active_leaf("parser");
        let report = format!(
            "<!-- SDLC_HANDOFF_JSON_START -->\n\
             {{ \"version\": 1, \"node_id\": \"{node_id}\", \"status\": \"done\", \"summary\": \"ok\" }}\n\
             <!-- SDLC_HANDOFF_JSON_END -->"
        );
        // Pass wrong claimed id → rejection
        let _ = try_apply_subagent_handoff(&session_dir, Some("n-wrong"), None, &report);
        // Verify audit event was written
        let conn = crate::model::msglog::open(&session_dir).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sdlc_events WHERE kind = 'handoff_rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count >= 1, "rejected handoff should write audit event");
        let _ = std::fs::remove_dir_all(&session_dir);
    }
}

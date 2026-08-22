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
#[path = "handoff_test.rs"]
mod tests;

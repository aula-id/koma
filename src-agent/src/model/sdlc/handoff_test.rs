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

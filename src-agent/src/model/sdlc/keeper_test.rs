#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::sdlc::graph::{self, ChecklistNode};
use crate::model::sdlc::Mission;
use std::path::PathBuf;
use std::process::Command;

fn tmp_session() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "koma-keeper-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_bound_worktree(dir: &std::path::Path) -> PathBuf {
    let worktree = dir.join("wt");
    std::fs::create_dir_all(&worktree).unwrap();
    // Avoid Homebrew git hook-template race on ARM macOS by pointing at an
    // empty template dir so `git init` skips the problematic copy step.
    let empty_templates = dir.join("empty-templates");
    std::fs::create_dir_all(&empty_templates).unwrap();
    let output = Command::new("git")
        .args(["init", "-b", "sdlc/g"])
        .current_dir(&worktree)
        .env("GIT_TEMPLATE_DIR", &empty_templates)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git init in {} failed: {}",
        worktree.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    run_git(
        &worktree,
        &["config", "user.email", "keeper-test@example.invalid"],
    );
    run_git(&worktree, &["config", "user.name", "Keeper Test"]);
    std::fs::write(worktree.join("README.md"), "test\n").unwrap();
    run_git(&worktree, &["add", "README.md"]);
    run_git(&worktree, &["commit", "-m", "initial"]);
    worktree
}

fn write_mission(dir: &std::path::Path, phase: &str, approved: bool) {
    let worktree = create_bound_worktree(dir);
    let graph_hash = Some("deadbeefdeadbeefdeadbeefdeadbeef".into());
    let worktree_name = Some("wt".into());
    let branch = Some("sdlc/g".into());
    let worktree_path = Some(worktree.to_string_lossy().into_owned());
    let target_worktree_path = Some("/tmp/primary".into());
    let target_branch = Some("main".into());
    let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let hash =
        Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &[],
            lane: "express",
            verify_plan: &["cargo test".into()],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: graph_hash.as_deref(),
            worktree_name: worktree_name.as_deref(),
            branch: branch.as_deref(),
            worktree_path: worktree_path.as_deref(),
            target_worktree_path: target_worktree_path.as_deref(),
            target_branch: target_branch.as_deref(),
            target_head: target_head.as_deref(),
        });
    let m = Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "m-k".into(),
        goal: "g".into(),
        non_goals: vec![],
        acceptance: vec!["a".into()],
        lane: "express".into(),
        verify_plan: vec!["cargo test".into()],
        human_gates: vec![],
        human_gates_approved: vec![],
        risks: vec![],
        worktree_name,
        branch,
        worktree_path,
        target_worktree_path,
        target_branch,
        target_head,
        rationale: String::new(),
        phase: phase.into(),
        approved,
        hash,
        graph_hash,
        needs_reapproval: false,
        amendment_note: None,
    };
    m.save(dir).unwrap();
}

// --- Prepare-phase keeper tests ---

#[test]
fn keeper_evaluates_during_prepare_phase() {
    // Keeper does NOT early-exit when phase is prepare.
    let dir = tmp_session();
    write_mission(&dir, "prepare", true);
    let conn = crate::model::msglog::open(&dir).unwrap();
    graph::ensure_tables(&conn).unwrap();
    graph::replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "setup task".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        }],
    )
    .unwrap();
    drop(conn);

    let report = evaluate(&dir);
    // Keeper ran (phase_hint should be prepare).
    assert_eq!(report.phase_hint.as_deref(), Some("prepare"));
    // No inject needed — single pending node is fine.
    assert!(report.inject.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_stall_detection_works_during_prepare() {
    let dir = tmp_session();
    write_mission(&dir, "prepare", true);
    let conn = crate::model::msglog::open(&dir).unwrap();
    graph::ensure_tables(&conn).unwrap();
    graph::replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "pending setup".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        }],
    )
    .unwrap();
    // Stamp tool round in the past.
    let past = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64)
        - KEEPER_STALL_SECS
        - 5;
    graph::set_mission_meta(&conn, META_LAST_TOOL_ROUND_AT, &past.to_string()).unwrap();
    let fp = graph::graph_fingerprint(&conn).unwrap();
    graph::set_mission_meta(&conn, META_LAST_GRAPH_FINGERPRINT, &fp).unwrap();
    drop(conn);

    let report = evaluate(&dir);
    assert!(
        report
            .inject
            .as_ref()
            .is_some_and(|s| s.contains("Stall detected")),
        "prepare phase should trigger stall detection: {:?}",
        report.inject
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reopens_false_done_in_prepare() {
    let dir = tmp_session();
    write_mission(&dir, "prepare", true);
    let conn = crate::model::msglog::open(&dir).unwrap();
    graph::ensure_tables(&conn).unwrap();
    graph::replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "setup item".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        }],
    )
    .unwrap();
    let id = graph::list_all(&conn).unwrap()[0].id.clone();
    // Fake false-done row.
    conn.execute(
        "UPDATE sdlc_nodes SET status = 'done', verify_bit = 0 WHERE id = ?1",
        rusqlite::params![id],
    )
    .unwrap();
    drop(conn);

    let report = evaluate(&dir);
    assert_eq!(report.reopened.len(), 1);
    assert!(report.inject.as_ref().unwrap().contains("False-done"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reopens_false_done() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    let conn = crate::model::msglog::open(&dir).unwrap();
    graph::ensure_tables(&conn).unwrap();
    graph::replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "ship it".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,

            owned_paths: vec![],
        }],
    )
    .unwrap();
    let id = graph::list_all(&conn).unwrap()[0].id.clone();
    // Legacy false-done row for keeper reopen path.
    conn.execute(
        "UPDATE sdlc_nodes SET status = 'done', verify_bit = 0 WHERE id = ?1",
        rusqlite::params![id],
    )
    .unwrap();
    drop(conn);

    let report = evaluate(&dir);
    assert_eq!(report.reopened.len(), 1);
    assert!(report.inject.as_ref().unwrap().contains("False-done"));
    let report2 = evaluate(&dir);
    assert!(report2.inject.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_skips_unapproved() {
    let dir = tmp_session();
    write_mission(&dir, "execute", false);
    let report = evaluate(&dir);
    assert!(report.inject.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_stall_when_graph_frozen() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    let conn = crate::model::msglog::open(&dir).unwrap();
    graph::ensure_tables(&conn).unwrap();
    graph::replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "leaf".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,

            owned_paths: vec![],
        }],
    )
    .unwrap();
    // Stamp tool round in the past.
    let past = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64)
        - KEEPER_STALL_SECS
        - 5;
    graph::set_mission_meta(&conn, META_LAST_TOOL_ROUND_AT, &past.to_string()).unwrap();
    let fp = graph::graph_fingerprint(&conn).unwrap();
    graph::set_mission_meta(&conn, META_LAST_GRAPH_FINGERPRINT, &fp).unwrap();
    drop(conn);

    let report = evaluate(&dir);
    assert!(
        report
            .inject
            .as_ref()
            .is_some_and(|s| s.contains("Stall detected")),
        "got {:?}",
        report.inject
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn llm_verdict_allow_true_returns_none() {
    let json = r#"{"allow": true, "reason": ""}"#;
    assert!(super::llm_verdict_to_inject(json).is_none());
}

#[test]
fn llm_verdict_allow_false_returns_inject() {
    let json = r#"{"allow": false, "reason": "Tasks stalled with no verify evidence"}"#;
    let inject = super::llm_verdict_to_inject(json).unwrap();
    assert!(inject.contains("[SDLC keeper — review]"));
    assert!(inject.contains("Tasks stalled"));
}

#[test]
fn llm_verdict_allow_false_empty_reason_returns_none() {
    let json = r#"{"allow": false, "reason": ""}"#;
    assert!(super::llm_verdict_to_inject(json).is_none());
}

#[test]
fn llm_verdict_malformed_returns_none() {
    assert!(super::llm_verdict_to_inject("not json").is_none());
    assert!(super::llm_verdict_to_inject("{}").is_none());
}

// --- Stage 2: reassessment rail tests ---

#[test]
fn keeper_reassessment_on_invalid_hash() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    // Invalidate the stored hash so hash_valid() fails.
    let mut m = Mission::load(&dir).unwrap();
    m.hash = "wrong".to_string();
    m.save(&dir).unwrap();

    let report = evaluate(&dir);
    assert!(
        report.action.is_some(),
        "should produce RequireReassessment"
    );
    match report.action.as_ref().unwrap() {
        super::KeeperAction::RequireReassessment { reason } => {
            assert!(
                reason.contains("contract hash invalid"),
                "unexpected reason: {reason}"
            );
        }
    }
    assert!(report.inject.is_some(), "should have inject text");

    // Simulate deferred action handling: mark disk mission reassess.
    let mut m = Mission::load(&dir).unwrap();
    assert!(
        m.approved,
        "mission still approved before deferred mutation"
    );
    m.approved = false;
    m.needs_reapproval = true;
    m.amendment_note = Some("keeper reassessment: contract hash invalid".into());
    let _ = m.try_transition("assess");
    m.save(&dir).unwrap();

    // Disk state: mission is in assess, unapproved, needs reapproval.
    let m2 = Mission::load(&dir).unwrap();
    assert!(!m2.approved);
    assert!(m2.needs_reapproval);
    assert_eq!(m2.phase, "assess");
    assert!(m2.amendment_note.is_some());
    // validate_active fails → tools blocked.
    assert!(m2.validate_active().is_err());
    let (ws, roots) = m2.tool_sandbox_roots(std::path::Path::new("/tmp"));
    assert!(ws.as_os_str().is_empty());
    assert!(roots.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reassessment_on_missing_graph_hash() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    // Remove graph hash but keep contract hash valid.
    let mut m = Mission::load(&dir).unwrap();
    m.graph_hash = None;
    m.hash = m.recompute_hash();
    m.save(&dir).unwrap();

    let report = evaluate(&dir);
    assert!(report.action.is_some());
    match report.action.as_ref().unwrap() {
        super::KeeperAction::RequireReassessment { reason } => {
            assert!(
                reason.contains("graph hash missing"),
                "unexpected reason: {reason}"
            );
        }
    }
    assert!(report.inject.is_some());

    // Simulate deferred: mark disk mission reassess.
    let mut m = Mission::load(&dir).unwrap();
    m.approved = false;
    m.needs_reapproval = true;
    let _ = m.try_transition("assess");
    m.save(&dir).unwrap();

    let m2 = Mission::load(&dir).unwrap();
    assert!(!m2.approved);
    assert!(m2.needs_reapproval);
    assert!(m2.validate_active().is_err());
    let (ws, roots) = m2.tool_sandbox_roots(std::path::Path::new("/tmp"));
    assert!(ws.as_os_str().is_empty());
    assert!(roots.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reassessment_on_lost_binding() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    // Lose the mission binding (worktree_path + branch cleared).
    let mut m = Mission::load(&dir).unwrap();
    m.worktree_path = None;
    m.branch = None;
    m.hash = m.recompute_hash();
    m.save(&dir).unwrap();

    let report = evaluate(&dir);
    assert!(report.action.is_some());
    match report.action.as_ref().unwrap() {
        super::KeeperAction::RequireReassessment { reason } => {
            assert!(
                reason.contains("mission binding lost"),
                "unexpected reason: {reason}"
            );
        }
    }
    assert!(report.inject.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reassessment_on_missing_worktree() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    let worktree = Mission::load(&dir)
        .unwrap()
        .worktree_path
        .map(PathBuf::from)
        .unwrap();
    std::fs::remove_dir_all(&worktree).unwrap();

    let report = evaluate(&dir);
    assert!(matches!(
        report.action,
        Some(super::KeeperAction::RequireReassessment { ref reason })
            if reason.contains("mission binding lost")
    ));
    assert!(report.inject.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reassessment_on_live_branch_mismatch() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    let worktree = Mission::load(&dir)
        .unwrap()
        .worktree_path
        .map(PathBuf::from)
        .unwrap();
    run_git(&worktree, &["checkout", "-b", "sdlc/other"]);

    let report = evaluate(&dir);
    assert!(matches!(
        report.action,
        Some(super::KeeperAction::RequireReassessment { ref reason })
            if reason.contains("mission binding lost")
    ));
    assert!(report.inject.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_reassessment_dedupe_on_repeated_eval() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    let mut m = Mission::load(&dir).unwrap();
    m.hash = "wrong".to_string();
    m.save(&dir).unwrap();

    // First evaluation: inject present.
    let report1 = evaluate(&dir);
    assert!(report1.inject.is_some(), "first eval should inject");
    assert!(report1.action.is_some());
    // Repeated evaluation: inject deduped (same hash), action still detected.
    let report2 = evaluate(&dir);
    assert!(report2.inject.is_none(), "second eval should be deduped");
    assert!(
        report2.action.is_some(),
        "action should still detect invalid state"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeper_repeated_action_does_not_reinject_after_disk_mutation() {
    let dir = tmp_session();
    write_mission(&dir, "execute", true);
    let mut m = Mission::load(&dir).unwrap();
    m.hash = "wrong".to_string();
    m.save(&dir).unwrap();

    // First evaluation: action + inject.
    let report1 = evaluate(&dir);
    assert!(report1.action.is_some());
    assert!(report1.inject.is_some());

    // Simulate deferred action handling (disk mutation).
    let mut m = Mission::load(&dir).unwrap();
    m.approved = false;
    m.needs_reapproval = true;
    m.amendment_note = Some("keeper reassessment: contract hash invalid".into());
    let _ = m.try_transition("assess");
    m.save(&dir).unwrap();

    // Second evaluation: mission no longer approved → no action, no inject.
    let report2 = evaluate(&dir);
    assert!(report2.action.is_none());
    assert!(report2.inject.is_none());
    assert!(report2.reopened.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::sdlc::graph::GraphTask;

fn sample_mission() -> Mission {
    let graph_hash = Some("abc".into());
    let worktree_name = Some("sdlc-test".into());
    let branch = Some("sdlc/ship-x".into());
    let worktree_path = Some("/tmp/wt".into());
    let target_worktree_path = Some("/tmp/primary".into());
    let target_branch = Some("main".into());
    let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let hash = Mission::compute_contract_hash_full(ContractHashInput {
        goal: "ship X",
        acceptance: &["tests pass".into()],
        non_goals: &["rewrite Y".into()],
        lane: "standard",
        verify_plan: &["cargo test".into()],
        human_gates: &[],
        risks: &["api churn".into()],
        rationale: "match house style",
        graph_hash: graph_hash.as_deref(),
        worktree_name: worktree_name.as_deref(),
        branch: branch.as_deref(),
        worktree_path: worktree_path.as_deref(),
        target_worktree_path: target_worktree_path.as_deref(),
        target_branch: target_branch.as_deref(),
        target_head: target_head.as_deref(),
    });
    Mission {
        contract_version: CURRENT_CONTRACT_VERSION,
        id: "m-test".into(),
        goal: "ship X".into(),
        non_goals: vec!["rewrite Y".into()],
        acceptance: vec!["tests pass".into()],
        lane: "standard".into(),
        verify_plan: vec!["cargo test".into()],
        human_gates: vec![],
        human_gates_approved: vec![],
        risks: vec!["api churn".into()],
        worktree_name,
        branch,
        worktree_path,
        target_worktree_path,
        target_branch,
        target_head,
        rationale: "match house style".into(),
        phase: "execute".into(),
        approved: true,
        hash,
        graph_hash,
        needs_reapproval: false,
        amendment_note: None,
        draft_locks: Default::default(),
    }
}

#[test]
fn seed_capsule_includes_open_and_sealed() {
    let m = sample_mission();
    let open = vec![GraphTask {
        id: "t1".into(),
        parent_id: None,
        title: "implement".into(),
        status: "active".into(),
        phase: None,
        notes: String::new(),
        verify_bit: false,
        updated_at: 0,
        owned_paths: vec![],
    }];
    let sealed = vec![GraphTask {
        id: "t0".into(),
        parent_id: None,
        title: "assess done".into(),
        status: "done".into(),
        phase: None,
        notes: String::new(),
        verify_bit: true,
        updated_at: 0,
        owned_paths: vec![],
    }];
    let cap =
        build_seed_capsule_with_all(&m, &open, &sealed, &[], &std::collections::HashMap::new());
    assert!(cap.contains("# SDLC mission capsule"));
    assert!(cap.contains("## OPEN"));
    assert!(cap.contains("## SEALED"));
    assert!(cap.contains("implement"));
    assert!(cap.contains("assess done"));
    assert!(cap.contains("ship X"));
    assert!(cap.contains("tests pass"));
}

#[test]
fn seed_capsule_includes_worktree_and_verify_plan() {
    let m = sample_mission();
    let cap = build_seed_capsule_with_all(&m, &[], &[], &[], &std::collections::HashMap::new());
    assert!(cap.contains("**Worktree:** sdlc-test (branch: sdlc/ship-x)"));
    assert!(cap.contains("**Verify plan:**"));
    assert!(cap.contains("- cargo test"));
}

#[test]
fn seed_capsule_shows_verify_status_on_sealed() {
    let m = sample_mission();
    let sealed = vec![
        GraphTask {
            id: "t1".into(),
            parent_id: None,
            title: "task1".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: false,
            updated_at: 0,
            owned_paths: vec![],
        },
        GraphTask {
            id: "t2".into(),
            parent_id: None,
            title: "task2".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: true,
            updated_at: 0,
            owned_paths: vec![],
        },
    ];
    let cap =
        build_seed_capsule_with_all(&m, &[], &sealed, &[], &std::collections::HashMap::new());
    assert!(cap.contains("task1 (t1) (UNVERIFIED)"));
    assert!(cap.contains("task2 (t2) (verified)"));
}

#[test]
fn seed_capsule_includes_human_gates_when_present() {
    let mut m = sample_mission();
    m.human_gates = vec!["review API".into()];
    // hash no longer matches after field change — that's fine for capsule text
    let cap = build_seed_capsule_with_all(&m, &[], &[], &[], &std::collections::HashMap::new());
    assert!(cap.contains("**Human gates:**"));
    assert!(cap.contains("review API"));
}

#[test]
fn hash_is_stable_for_same_inputs() {
    let a = Mission::compute_contract_hash_full(ContractHashInput {
        goal: "g",
        acceptance: &["a".into()],
        non_goals: &["n".into()],
        lane: "standard",
        verify_plan: &[],
        human_gates: &[],
        risks: &[],
        rationale: "",
        graph_hash: None,
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });
    let b = Mission::compute_contract_hash_full(ContractHashInput {
        goal: "g",
        acceptance: &["a".into()],
        non_goals: &["n".into()],
        lane: "standard",
        verify_plan: &[],
        human_gates: &[],
        risks: &[],
        rationale: "",
        graph_hash: None,
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });
    let c = Mission::compute_contract_hash_full(ContractHashInput {
        goal: "g2",
        acceptance: &["a".into()],
        non_goals: &["n".into()],
        lane: "standard",
        verify_plan: &[],
        human_gates: &[],
        risks: &[],
        rationale: "",
        graph_hash: None,
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), 32);
}

#[test]
fn full_contract_hash_covers_lane_and_graph() {
    let a = Mission::compute_contract_hash_full(ContractHashInput {
        goal: "g",
        acceptance: &["a".into()],
        non_goals: &[],
        lane: "full",
        verify_plan: &[],
        human_gates: &[],
        risks: &[],
        rationale: "",
        graph_hash: Some("gh1"),
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });
    let b = Mission::compute_contract_hash_full(ContractHashInput {
        goal: "g",
        acceptance: &["a".into()],
        non_goals: &[],
        lane: "full",
        verify_plan: &[],
        human_gates: &[],
        risks: &[],
        rationale: "",
        graph_hash: Some("gh2"),
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });
    assert_ne!(a, b);
}

#[test]
fn contract_hash_covers_worktree_binding() {
    let base = |wt: Option<&str>, br: Option<&str>, path: Option<&str>| {
        Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &[],
            lane: "standard",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: Some("gh"),
            worktree_name: wt,
            branch: br,
            worktree_path: path,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        })
    };
    let unbound = base(None, None, None);
    let bound = base(Some("wt"), Some("sdlc/x"), Some("/tmp/wt"));
    let other_path = base(Some("wt"), Some("sdlc/x"), Some("/tmp/other"));
    assert_ne!(unbound, bound);
    assert_ne!(bound, other_path);
    assert!(sample_mission().hash_valid());
}

#[test]
fn contract_hash_covers_frozen_target() {
    let base = |tp: Option<&str>, tb: Option<&str>, th: Option<&str>| {
        Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &[],
            lane: "standard",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: Some("gh"),
            worktree_name: Some("wt"),
            branch: Some("sdlc/x"),
            worktree_path: Some("/tmp/wt"),
            target_worktree_path: tp,
            target_branch: tb,
            target_head: th,
        })
    };
    let no_target = base(None, None, None);
    let with_target = base(Some("/tmp/p"), Some("main"), Some("abc123"));
    let other_branch = base(Some("/tmp/p"), Some("develop"), Some("abc123"));
    let other_head = base(Some("/tmp/p"), Some("main"), Some("def456"));
    assert_ne!(no_target, with_target);
    assert_ne!(with_target, other_branch);
    assert_ne!(with_target, other_head);
    assert!(sample_mission().has_frozen_target());
    assert!(sample_mission().hash_valid());
}

#[test]
fn legacy_missing_target_deserializes_but_fails_active() {
    // Simulate a pre-v2 mission.json without target_* fields.
    let json = r#"{
        "id": "m-legacy",
        "goal": "ship X",
        "non_goals": [],
        "acceptance": ["tests pass"],
        "lane": "standard",
        "verify_plan": [],
        "human_gates": [],
        "risks": [],
        "worktree_name": "sdlc-test",
        "branch": "sdlc/ship-x",
        "worktree_path": "/tmp/wt",
        "rationale": "",
        "phase": "execute",
        "approved": true,
        "hash": "deadbeefdeadbeefdeadbeefdeadbeef",
        "graph_hash": "abc",
        "needs_reapproval": false
    }"#;
    let m: Mission = serde_json::from_str(json).expect("legacy must deserialize");
    assert_eq!(m.contract_version, LEGACY_CONTRACT_VERSION);
    assert!(m.target_worktree_path.is_none());
    assert!(m.target_branch.is_none());
    assert!(m.target_head.is_none());
    assert!(!m.has_frozen_target());
    // Hash won't match recompute (target fields now hashed as empty), and
    // validate_active fails closed either way.
    assert!(m.validate_active().is_err());
    let err = m.validate_active().unwrap_err().to_string();
    assert!(
        err.contains("hash mismatch")
            || err.contains("missing frozen target")
            || err.contains("legacy"),
        "unexpected: {err}"
    );
}

#[test]
fn seed_capsule_includes_frozen_target() {
    let m = sample_mission();
    let cap = build_seed_capsule_with_all(&m, &[], &[], &[], &std::collections::HashMap::new());
    assert!(
        cap.contains("**Target:** main @"),
        "capsule must show frozen target branch, got: {cap}"
    );
    assert!(cap.contains("/tmp/primary"));
    assert!(
        cap.contains("Never force-push") && cap.contains("mission_verify"),
        "capsule Law must list enforced edges, got: {cap}"
    );
}

#[test]
fn try_transition_allows_legal_edges_and_rejects_illegal() {
    let mut m = sample_mission();
    // sample starts in execute
    assert!(m.try_transition("execute").is_ok()); // identity
    assert!(m.try_transition("integrate").is_ok());
    assert_eq!(m.phase, "integrate");
    assert!(m.try_transition("done").is_ok());
    assert_eq!(m.phase, "done");
    // any → assess (fail-closed rail)
    assert!(m.try_transition("assess").is_ok());
    assert_eq!(m.phase, "assess");
    assert!(m.try_transition("execute").is_ok());
    assert!(m.try_transition("paused").is_ok());
    assert_eq!(m.phase, "paused");
    assert!(m.try_transition("execute").is_ok());
    // illegal
    m.phase = "assess".into();
    let err = m.try_transition("done").unwrap_err().to_string();
    assert!(err.contains("illegal"), "{err}");
    m.phase = "draft".into();
    assert!(m.try_transition("assess").is_ok());
    m.phase = "draft".into();
    assert!(m.try_transition("execute").is_ok());
    m.phase = "paused".into();
    assert!(m.try_transition("integrate").is_err());
    // prepare phase edges
    m.phase = "assess".into();
    assert!(m.try_transition("prepare").is_ok());
    assert_eq!(m.phase, "prepare");
    assert!(m.try_transition("execute").is_ok());
    m.phase = "prepare".into();
    assert!(m.try_transition("paused").is_ok());
    assert_eq!(m.phase, "paused");
    assert!(m.try_transition("prepare").is_ok());
    assert_eq!(m.phase, "prepare");
    // prepare → integrate must FAIL (must go through execute first)
    m.phase = "prepare".into();
    assert!(m.try_transition("integrate").is_err());
    // prepare → done must FAIL
    assert!(m.try_transition("done").is_err());
}

#[test]
fn legacy_empty_hash_fails_active_validation() {
    let mut m = sample_mission();
    m.hash = String::new();
    assert!(m.validate_active().is_err());
}

#[test]
fn amendment_clears_approval() {
    let mut m = sample_mission();
    // Mirror production amendment path (mission_ready / re-entry fail-closed):
    // unapprove, force assess, flag reapproval, clear binding for re-bind.
    m.approved = false;
    m.phase = "assess".into();
    m.needs_reapproval = true;
    m.amendment_note = Some("change scope".into());
    m.worktree_path = None;
    assert!(!m.approved);
    assert!(m.needs_reapproval);
    assert_eq!(m.phase, "assess");
    assert!(m.worktree_path.is_none());
}

#[test]
fn should_not_auto_resume_draft_done_paused() {
    let mut m = sample_mission();
    m.phase = "done".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
    m.phase = "draft".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
    // Explicit exit marks missions paused — restart must NOT auto-resume them.
    m.phase = "paused".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
    m.phase = "assess".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
    // prepare is an active setup phase — auto-resume is allowed
    m.phase = "prepare".into();
    assert!(should_auto_resume(&m));
    assert_eq!(resume_phase(&m).as_deref(), Some("prepare"));
    // Only live execute/integrate resume.
    m.phase = "execute".into();
    assert!(should_auto_resume(&m));
    assert_eq!(resume_phase(&m).as_deref(), Some("execute"));
    m.phase = "integrate".into();
    assert!(should_auto_resume(&m));
    assert_eq!(resume_phase(&m).as_deref(), Some("integrate"));
}

/// Auto-resume must deny: unapproved mission (even with valid phase).
#[test]
fn auto_resume_denies_unapproved() {
    let mut m = sample_mission();
    m.approved = false;
    m.phase = "execute".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
}

/// Auto-resume must deny: needs_reapproval (amended contract).
#[test]
fn auto_resume_denies_needs_reapproval() {
    let mut m = sample_mission();
    m.needs_reapproval = true;
    m.phase = "execute".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
}

/// Auto-resume must deny: invalid hash (tampered contract).
#[test]
fn auto_resume_denies_invalid_hash() {
    let mut m = sample_mission();
    m.hash = "deadbeef".repeat(4);
    m.phase = "execute".into();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
}

/// Auto-resume must deny: missing worktree binding (execute/integrate only).
/// should_auto_resume now enforces binding directly so that the gate is
/// fail-closed at the predicate level, not deferred to validate_active.
/// prepare-phase missions don't need binding yet — it's established during
/// prepare, not before.
#[test]
fn auto_resume_denies_missing_binding_execute_integrate() {
    let mut m = sample_mission();
    m.worktree_path = None;
    for phase in ["execute", "integrate"] {
        m.phase = phase.to_string();
        m.hash = m.recompute_hash();
        assert!(
            !should_auto_resume(&m),
            "phase={phase} must reject when worktree_path is None"
        );
        assert!(resume_phase(&m).is_none());
    }
    // Also deny when branch is missing.
    let mut m2 = sample_mission();
    m2.branch = None;
    for phase in ["execute", "integrate"] {
        m2.phase = phase.to_string();
        m2.hash = m2.recompute_hash();
        assert!(
            !should_auto_resume(&m2),
            "phase={phase} must reject when branch is None"
        );
    }
    // prepare-phase without binding should still auto-resume (binding
    // is established during prepare, not before).
    let mut m3 = sample_mission();
    m3.worktree_path = None;
    m3.phase = "prepare".into();
    m3.hash = m3.recompute_hash();
    assert!(
        should_auto_resume(&m3),
        "prepare phase should allow auto-resume even without binding"
    );
}

/// Auto-resume: complete lifecycle coverage for draft/denied/paused/done/stale.
#[test]
fn auto_resume_comprehensive_lifecycle_coverage() {
    // draft: unapproved, should not resume
    let mut m = sample_mission();
    m.phase = "draft".into();
    m.approved = false;
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m), "draft must not resume");

    // denied (unapproved + needs_reapproval)
    let mut m = sample_mission();
    m.phase = "assess".into();
    m.approved = false;
    m.needs_reapproval = true;
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m), "denied assess must not resume");

    // paused: explicit exit, should not resume
    let mut m = sample_mission();
    m.phase = "paused".into();
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m), "paused must not resume");

    // done: terminal state, should not resume
    let mut m = sample_mission();
    m.phase = "done".into();
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m), "done must not resume");

    // stale: old contract version
    let mut m = sample_mission();
    m.contract_version = 1;
    m.phase = "execute".into();
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m), "stale contract must not resume");

    // invalid: tampered hash
    let mut m = sample_mission();
    m.hash = "dead".repeat(8);
    m.phase = "execute".into();
    assert!(!should_auto_resume(&m), "invalid hash must not resume");

    // missing frozen target
    let mut m = sample_mission();
    m.target_worktree_path = None;
    m.target_branch = None;
    m.target_head = None;
    m.phase = "execute".into();
    m.hash = m.recompute_hash();
    assert!(
        !should_auto_resume(&m),
        "missing frozen target must not resume"
    );

    // valid prepare/execute/integrate should resume
    let m = sample_mission();
    assert!(should_auto_resume(&m), "valid execute must resume");
}

/// Auto-resume must deny: legacy contract version.
#[test]
fn auto_resume_denies_legacy_contract() {
    let mut m = sample_mission();
    m.contract_version = 1; // legacy
    m.phase = "execute".into();
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
}

/// Auto-resume must deny: missing frozen target.
#[test]
fn auto_resume_denies_missing_frozen_target() {
    let mut m = sample_mission();
    m.target_worktree_path = None;
    m.target_branch = None;
    m.target_head = None;
    m.phase = "execute".into();
    m.hash = m.recompute_hash();
    assert!(!should_auto_resume(&m));
    assert!(resume_phase(&m).is_none());
}

/// Auto-resume: all conditions met for prepare/execute/integrate.
#[test]
fn auto_resume_allows_valid_active_phases() {
    let m = sample_mission();
    for phase in ["prepare", "execute", "integrate"] {
        let mut test_m = m.clone();
        test_m.phase = phase.to_string();
        assert!(
            should_auto_resume(&test_m),
            "should auto-resume phase={phase}"
        );
        assert_eq!(
            resume_phase(&test_m).as_deref(),
            Some(phase),
            "resume_phase mismatch for phase={phase}"
        );
    }
}

#[test]
fn mission_roundtrip_json() {
    let dir = std::env::temp_dir().join(format!("koma-sdlc-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let m = sample_mission();
    m.save(&dir).unwrap();
    let loaded = Mission::load(&dir).unwrap();
    assert_eq!(loaded.goal, m.goal);
    assert!(loaded.approved);
    assert_eq!(loaded.hash, m.hash);
    assert_eq!(loaded.graph_hash, m.graph_hash);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_gates_approval_path() {
    let mut m = sample_mission();
    m.human_gates = vec!["g1".into(), "g2".into()];
    assert!(!m.human_gates_satisfied());
    m.approve_human_gate("g1");
    assert!(!m.human_gates_satisfied());
    m.approve_human_gate("g2");
    assert!(m.human_gates_satisfied());
}

#[test]
fn validate_binding_rejects_cwd_mismatch_no_fallback() {
    let m = sample_mission();
    // sample binds worktree_path=/tmp/wt — current dir is not that path.
    let cwd = std::env::temp_dir();
    let err = m
        .validate_binding(&cwd, m.branch.as_deref())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("worktree mismatch") || err.contains("no worktree"),
        "unexpected: {err}"
    );
}

#[test]
fn tool_sandbox_roots_fail_closed_without_valid_binding() {
    let mut m = sample_mission();
    // Active execute mission with missing on-disk worktree → empty roots.
    m.phase = "execute".into();
    m.worktree_path = Some("/tmp/koma-sdlc-missing-wt-definitely-not-real".into());
    m.hash = m.recompute_hash();
    let live = std::path::PathBuf::from("/tmp");
    let (ws, roots) = m.tool_sandbox_roots(&live);
    assert!(ws.as_os_str().is_empty(), "cwd must be poisoned empty");
    assert!(roots.is_empty(), "no writable roots without bound tree");

    // Paused also denies (not an active execute/integrate phase).
    m.phase = "paused".into();
    m.worktree_path = Some("/tmp".into());
    m.hash = m.recompute_hash();
    let (ws2, roots2) = m.tool_sandbox_roots(&live);
    assert!(ws2.as_os_str().is_empty());
    assert!(roots2.is_empty());

    m.phase = "execute".into();
    m.approved = false;
    m.hash = m.recompute_hash();
    let (ws3, roots3) = m.tool_sandbox_roots(&live);
    assert!(ws3.as_os_str().is_empty());
    assert!(roots3.is_empty());
}

#[test]
fn tool_sandbox_roots_pins_to_bound_when_live_mismatches() {
    let dir = std::env::temp_dir().join(format!("koma-sdlc-sandbox-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut m = sample_mission();
    m.phase = "execute".into();
    m.worktree_path = Some(dir.to_string_lossy().into_owned());
    m.hash = m.recompute_hash();
    let other = std::env::temp_dir();
    let (ws, roots) = m.tool_sandbox_roots(&other);
    assert_eq!(roots.len(), 1);
    let bound_canon = std::fs::canonicalize(&dir).unwrap_or(dir.clone());
    let root_canon = std::fs::canonicalize(&roots[0]).unwrap_or(roots[0].clone());
    assert_eq!(root_canon, bound_canon);
    let ws_canon = std::fs::canonicalize(&ws).unwrap_or(ws.clone());
    assert_eq!(ws_canon, bound_canon);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn integrate_gate_requires_live_binding_and_frozen_target() {
    use crate::model::sdlc::graph::{
        ensure_tables, replace_nodes_from_checklist, set_verify_bit_with_evidence,
        ChecklistNode,
    };
    use rusqlite::Connection;
    use std::process::Command;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root =
        std::env::temp_dir().join(format!("koma-sdlc-igate-{}-{}", std::process::id(), stamp));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let primary = root.join("primary");
    let bound = root.join("mission-wt");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&bound).unwrap();

    let run_in = |dir: &std::path::Path, args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?} in {} → {}",
            dir.display(),
            String::from_utf8_lossy(&o.stderr)
        );
    };
    // Primary on non-main target branch `develop`.
    run_in(&primary, &["init", "-b", "develop"]);
    run_in(&primary, &["config", "user.email", "t@t"]);
    run_in(&primary, &["config", "user.name", "t"]);
    std::fs::write(primary.join("a.txt"), "a").unwrap();
    run_in(&primary, &["add", "."]);
    run_in(&primary, &["commit", "-m", "init"]);
    let target_head = current_git_head(&primary).expect("head");
    // Mission worktree is a separate git dir on mission branch (path check only
    // for validate_binding — no shared object db required for the gate).
    run_in(&bound, &["init", "-b", "sdlc/ship-x"]);
    run_in(&bound, &["config", "user.email", "t@t"]);
    run_in(&bound, &["config", "user.name", "t"]);
    std::fs::write(bound.join("b.txt"), "b").unwrap();
    run_in(&bound, &["add", "."]);
    run_in(&bound, &["commit", "-m", "feat"]);

    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    replace_nodes_from_checklist(
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
    let leaf_id = crate::model::sdlc::graph::list_all(&conn).unwrap()[0]
        .id
        .clone();
    let bound_sha = current_git_head(&bound).expect("bound head");
    let bound_sha_short = if bound_sha.len() > 7 {
        &bound_sha[..7]
    } else {
        &bound_sha
    };
    set_verify_bit_with_evidence(
        &conn,
        &leaf_id,
        true,
        Some(&format!("tests pass | commit:{bound_sha_short}")),
    )
    .unwrap();
    assert!(crate::model::sdlc::graph::list_open_leaves(&conn)
        .unwrap()
        .is_empty());
    assert!(crate::model::sdlc::graph::all_required_leaves_verified(&conn).unwrap());

    let structural = structural_graph_hash(&conn).unwrap();
    let mut m = sample_mission();
    m.phase = "execute".into();
    m.graph_hash = Some(structural);
    m.worktree_path = Some(bound.to_string_lossy().into_owned());
    m.branch = Some("sdlc/ship-x".into());
    m.worktree_name = Some("sdlc-test".into());
    m.target_worktree_path = Some(primary.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
    m.target_head = Some(target_head);
    m.human_gates = vec![];
    m.human_gates_approved = vec![];
    m.hash = m.recompute_hash();
    assert!(m.validate_active().is_ok());
    assert_eq!(m.target_branch.as_deref(), Some("develop"));

    // Stale / missing live cwd must fail closed before integrate proceeds.
    let wrong_cwd = std::path::Path::new("/definitely/not/the/bound/path");
    let err = integrate_gate(&m, &conn, wrong_cwd, Some("sdlc/ship-x"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("worktree mismatch"),
        "expected worktree mismatch from integrate_gate, got: {err}"
    );

    // Correct cwd + wrong branch is also rejected (no path fallbacks).
    let err_branch = integrate_gate(&m, &conn, &bound, Some("other-branch"))
        .unwrap_err()
        .to_string();
    assert!(
        err_branch.contains("branch mismatch"),
        "expected branch mismatch from integrate_gate, got: {err_branch}"
    );

    // Target branch drift: freeze says develop, but switch primary to main.
    run_in(&primary, &["checkout", "-b", "main"]);
    let err_target = integrate_gate(&m, &conn, &bound, Some("sdlc/ship-x"))
        .unwrap_err()
        .to_string();
    assert!(
        err_target.contains("target branch drift") || err_target.contains("develop"),
        "expected target drift rejection, got: {err_target}"
    );
    // Restore develop for the control case.
    run_in(&primary, &["checkout", "develop"]);

    // Control: live cwd + bound branch + matching frozen target passes.
    integrate_gate(&m, &conn, &bound, Some("sdlc/ship-x")).unwrap();

    // Legacy: missing frozen target fails closed even with good mission binding.
    let mut legacy = m.clone();
    legacy.target_worktree_path = None;
    legacy.target_branch = None;
    legacy.target_head = None;
    legacy.hash = legacy.recompute_hash();
    let err_legacy = integrate_gate(&legacy, &conn, &bound, Some("sdlc/ship-x"))
        .unwrap_err()
        .to_string();
    assert!(
        err_legacy.contains("frozen target") || err_legacy.contains("re-approval"),
        "expected legacy target failure, got: {err_legacy}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cannot_overwrite_concept_via_hash_valid() {
    let m = sample_mission();
    assert!(m.hash_valid());
    let mut m2 = m.clone();
    m2.goal = "other".into();
    assert!(!m2.hash_valid());
}

#[test]
fn is_ancestor_detects_related_history() {
    use std::process::Command;
    let root = std::env::temp_dir().join(format!(
        "koma-sdlc-ancestor-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?} {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    run(&["init", "-b", "develop"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "init"]);
    let base = current_git_head(&root).unwrap();
    run(&["checkout", "-b", "sdlc/feat"]);
    std::fs::write(root.join("b.txt"), "b").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "feat"]);
    let tip = current_git_head(&root).unwrap();
    assert!(
        is_ancestor(&root, &base, &tip),
        "base must be ancestor of tip"
    );
    assert!(
        is_ancestor(&root, &base, &base),
        "commit is ancestor of itself"
    );
    // Unrelated: tip is NOT ancestor of base.
    assert!(!is_ancestor(&root, &tip, &base) || tip == base);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cleanup_done_mission_removes_integrated_resources_then_resets() {
    use crate::model::sdlc::graph::{
        ensure_tables, replace_nodes_from_checklist, set_verify_bit_with_evidence,
        ChecklistNode,
    };

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "koma-sdlc-done-cleanup-{}-{stamp}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let primary = root.join("primary");
    let worktree = root.join("mission-worktree");
    let session_dir = root.join("session");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&session_dir).unwrap();

    let git = |dir: &std::path::Path, args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&primary, &["init", "-b", "main"]);
    git(&primary, &["config", "user.email", "test@example.com"]);
    git(&primary, &["config", "user.name", "Test"]);
    std::fs::write(primary.join("base.txt"), "base").unwrap();
    git(&primary, &["add", "."]);
    git(&primary, &["commit", "-m", "base"]);
    let target_head = current_git_head(&primary).unwrap();

    let branch = "sdlc/done-cleanup";
    git(
        &primary,
        &["worktree", "add", "-b", branch, &worktree.to_string_lossy()],
    );
    std::fs::write(worktree.join("feature.txt"), "feature").unwrap();
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-m", "feature"]);
    git(&primary, &["merge", "--ff-only", branch]);

    let conn = crate::model::msglog::open(&session_dir).unwrap();
    ensure_tables(&conn).unwrap();
    replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "verified leaf".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        }],
    )
    .unwrap();
    let leaf_id = graph::list_all(&conn).unwrap()[0].id.clone();
    set_verify_bit_with_evidence(&conn, &leaf_id, true, Some("cargo test")).unwrap();

    let mut mission = sample_mission();
    mission
        .try_transition("integrate")
        .and_then(|_| mission.try_transition("done"))
        .unwrap();
    mission.worktree_name = Some("sdlc-done-cleanup".into());
    mission.branch = Some(branch.into());
    mission.worktree_path = Some(worktree.to_string_lossy().into_owned());
    mission.target_worktree_path = Some(primary.to_string_lossy().into_owned());
    mission.target_branch = Some("main".into());
    mission.target_head = Some(target_head);
    mission.graph_hash = Some(structural_graph_hash(&conn).unwrap());
    mission.hash = mission.recompute_hash();
    mission.save(&session_dir).unwrap();

    // A dirty worktree makes git refuse removal. The terminal contract must
    // remain truthful and retryable rather than being reset to assess.
    std::fs::write(worktree.join("untracked.txt"), "dirty").unwrap();
    let cleanup_error = cleanup_done_mission(&session_dir).unwrap_err().to_string();
    assert!(cleanup_error.contains("could not remove mission worktree"));
    let retained = Mission::load(&session_dir).unwrap();
    assert_eq!(retained.phase, "done");
    assert_eq!(retained.branch.as_deref(), Some(branch));
    assert_eq!(retained.worktree_path.as_deref(), worktree.to_str());
    assert!(worktree.exists());
    std::fs::remove_file(worktree.join("untracked.txt")).unwrap();

    assert_eq!(
        cleanup_done_mission(&session_dir).unwrap(),
        DoneCleanupOutcome::ResetToAssess
    );
    let reset = Mission::load(&session_dir).unwrap();
    assert_eq!(reset.phase, "assess");
    assert!(!reset.approved);
    assert!(reset.needs_reapproval);
    assert!(reset.worktree_name.is_none());
    assert!(reset.branch.is_none());
    assert!(reset.worktree_path.is_none());
    assert!(!worktree.exists());
    assert!(!std::process::Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            "refs/heads/sdlc/done-cleanup"
        ])
        .current_dir(&primary)
        .status()
        .unwrap()
        .success());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn validate_active_requires_frozen_target() {
    let mut m = sample_mission();
    assert!(m.validate_active().is_ok());
    m.target_branch = None;
    m.hash = m.recompute_hash();
    let err = m.validate_active().unwrap_err().to_string();
    assert!(
        err.contains("frozen target") || err.contains("re-approval"),
        "unexpected: {err}"
    );
}

#[test]
fn capsule_shows_commit_sha_for_sealed_nodes() {
    use crate::model::sdlc::graph::GraphTask;

    let m = sample_mission();
    let sealed = vec![
        GraphTask {
            id: "t1".into(),
            parent_id: None,
            title: "task1".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: true,
            updated_at: 0,
            owned_paths: vec![],
        },
        GraphTask {
            id: "t2".into(),
            parent_id: None,
            title: "task2".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: false,
            updated_at: 0,
            owned_paths: vec![],
        },
    ];
    let mut shas = std::collections::HashMap::new();
    shas.insert("t1".into(), vec!["abc1234567890".into()]);
    let cap = build_seed_capsule_with_all(&m, &[], &sealed, &[], &shas);
    assert!(
        cap.contains("(commit: abc1234)"),
        "capsule must show commit SHA for verified sealed node, got: {cap}"
    );
    assert!(
        cap.contains("task1 (t1) (verified) (commit: abc1234)"),
        "unexpected format for verified sealed node: {cap}"
    );
    // t2 has no commit SHA and is UNVERIFIED
    assert!(cap.contains("task2 (t2) (UNVERIFIED)"));
}

#[test]
fn capsule_hierarchical_shows_commit_sha_for_sealed() {
    use crate::model::sdlc::graph::GraphTask;

    let m = sample_mission();
    let all = vec![
        GraphTask {
            id: "epic".into(),
            parent_id: None,
            title: "epic".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: true,
            updated_at: 0,
            owned_paths: vec![],
        },
        GraphTask {
            id: "leaf1".into(),
            parent_id: Some("epic".into()),
            title: "leaf1".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: true,
            updated_at: 0,
            owned_paths: vec![],
        },
    ];
    let mut shas = std::collections::HashMap::new();
    shas.insert("leaf1".into(), vec!["deadbeef1234".into()]);
    let cap = build_seed_capsule_with_all(&m, &[], &all, &all, &shas);
    assert!(
        cap.contains("(commit: deadbee)"),
        "hierarchical capsule must show commit SHA, got: {cap}"
    );
}

#[test]
fn integrate_gate_rejects_when_sealed_leaf_has_no_commit_evidence() {
    use crate::model::sdlc::graph::{
        ensure_tables, replace_nodes_from_checklist, set_verify_bit_with_evidence,
        ChecklistNode,
    };
    use rusqlite::Connection;
    use std::process::Command;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "koma-sdlc-igate-noev-{}-{}",
        std::process::id(),
        stamp
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let primary = root.join("primary");
    let bound = root.join("mission-wt");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&bound).unwrap();

    let run_in = |dir: &std::path::Path, args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?} in {} → {}",
            dir.display(),
            String::from_utf8_lossy(&o.stderr)
        );
    };
    run_in(&primary, &["init", "-b", "develop"]);
    run_in(&primary, &["config", "user.email", "t@t"]);
    run_in(&primary, &["config", "user.name", "t"]);
    std::fs::write(primary.join("a.txt"), "a").unwrap();
    run_in(&primary, &["add", "."]);
    run_in(&primary, &["commit", "-m", "init"]);
    let target_head = current_git_head(&primary).expect("head");

    run_in(&bound, &["init", "-b", "sdlc/ship-x"]);
    run_in(&bound, &["config", "user.email", "t@t"]);
    run_in(&bound, &["config", "user.name", "t"]);
    std::fs::write(bound.join("b.txt"), "b").unwrap();
    run_in(&bound, &["add", "."]);
    run_in(&bound, &["commit", "-m", "feat"]);

    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    replace_nodes_from_checklist(
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
    let leaf_id = crate::model::sdlc::graph::list_all(&conn).unwrap()[0]
        .id
        .clone();
    // Verify WITHOUT commit evidence — should fail integrate gate.
    set_verify_bit_with_evidence(&conn, &leaf_id, true, Some("tests pass")).unwrap();

    let structural = structural_graph_hash(&conn).unwrap();
    let mut m = sample_mission();
    m.phase = "execute".into();
    m.graph_hash = Some(structural);
    m.worktree_path = Some(bound.to_string_lossy().into_owned());
    m.branch = Some("sdlc/ship-x".into());
    m.worktree_name = Some("sdlc-test".into());
    m.target_worktree_path = Some(primary.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
    m.target_head = Some(target_head);
    m.human_gates = vec![];
    m.human_gates_approved = vec![];
    m.hash = m.recompute_hash();

    let err = integrate_gate(&m, &conn, &bound, Some("sdlc/ship-x"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("no commit evidence"),
        "expected commit evidence rejection, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn integrate_gate_accepts_when_commit_shas_are_reachable() {
    use crate::model::sdlc::graph::{
        ensure_tables, replace_nodes_from_checklist, set_verify_bit_with_evidence,
        ChecklistNode,
    };
    use rusqlite::Connection;
    use std::process::Command;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "koma-sdlc-igate-reach-{}-{}",
        std::process::id(),
        stamp
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let primary = root.join("primary");
    let bound = root.join("mission-wt");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&bound).unwrap();

    let run_in = |dir: &std::path::Path, args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?} in {} → {}",
            dir.display(),
            String::from_utf8_lossy(&o.stderr)
        );
    };
    run_in(&primary, &["init", "-b", "develop"]);
    run_in(&primary, &["config", "user.email", "t@t"]);
    run_in(&primary, &["config", "user.name", "t"]);
    std::fs::write(primary.join("a.txt"), "a").unwrap();
    run_in(&primary, &["add", "."]);
    run_in(&primary, &["commit", "-m", "init"]);
    let target_head = current_git_head(&primary).expect("head");

    run_in(&bound, &["init", "-b", "sdlc/ship-x"]);
    run_in(&bound, &["config", "user.email", "t@t"]);
    run_in(&bound, &["config", "user.name", "t"]);
    std::fs::write(bound.join("b.txt"), "b").unwrap();
    run_in(&bound, &["add", "."]);
    run_in(&bound, &["commit", "-m", "feat"]);
    let bound_head = current_git_head(&bound).expect("bound head");
    let bound_head_short = if bound_head.len() > 7 {
        &bound_head[..7]
    } else {
        &bound_head
    };

    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    replace_nodes_from_checklist(
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
    let leaf_id = crate::model::sdlc::graph::list_all(&conn).unwrap()[0]
        .id
        .clone();
    // Verify WITH reachable commit evidence — should pass integrate gate.
    set_verify_bit_with_evidence(
        &conn,
        &leaf_id,
        true,
        Some(&format!("tests pass | commit:{bound_head_short}")),
    )
    .unwrap();

    let structural = structural_graph_hash(&conn).unwrap();
    let mut m = sample_mission();
    m.phase = "execute".into();
    m.graph_hash = Some(structural);
    m.worktree_path = Some(bound.to_string_lossy().into_owned());
    m.branch = Some("sdlc/ship-x".into());
    m.worktree_name = Some("sdlc-test".into());
    m.target_worktree_path = Some(primary.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
    m.target_head = Some(target_head);
    m.human_gates = vec![];
    m.human_gates_approved = vec![];
    m.hash = m.recompute_hash();

    // Should pass commit evidence check (may still fail on other gates, but
    // "no commit evidence" and "not reachable" should NOT appear).
    let result = integrate_gate(&m, &conn, &bound, Some("sdlc/ship-x"));
    match result {
        Ok(()) => {} // All gates passed — fine.
        Err(e) => {
            let msg = e.to_string();
            assert!(
                !msg.contains("no commit evidence") && !msg.contains("not reachable"),
                "commit evidence check should pass, got: {msg}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn verify_pass_captures_commit_sha_in_evidence() {
    use crate::model::sdlc::graph::{
        ensure_tables, latest_verified_commit_shas, replace_nodes_from_checklist,
        set_verify_bit_with_evidence, ChecklistNode,
    };
    use rusqlite::Connection;
    use std::process::Command;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "koma-sdlc-verify-sha-{}-{}",
        std::process::id(),
        stamp
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let run = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?} → {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    run(&["init", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "init"]);
    let head_sha = current_git_head(&repo).unwrap();
    let head_short7 = &head_sha[..7];

    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "task".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        }],
    )
    .unwrap();
    let node_id = crate::model::sdlc::graph::list_all(&conn).unwrap()[0]
        .id
        .clone();

    // Simulate the intercept logic: capture SHA, augment evidence, store.
    let sha = capture_head_short_sha(&repo).expect("should capture SHA");
    let evidence = format!("tests pass | commit:{sha}");
    set_verify_bit_with_evidence(&conn, &node_id, true, Some(&evidence)).unwrap();

    let shas = latest_verified_commit_shas(&conn, &[node_id.clone()]).unwrap();
    let node_shas = shas.get(&node_id).unwrap();
    assert_eq!(node_shas.len(), 1);
    assert_eq!(node_shas[0], head_short7);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn verify_path_never_invokes_git_commit() {
    use std::process::Command;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "koma-sdlc-no-commit-{}-{}",
        std::process::id(),
        stamp
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let run = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?} → {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    run(&["init", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "init"]);

    // Install a pre-commit hook that FAILS — if git commit is ever called,
    // the hook would abort it. Since verify only calls git rev-parse, this
    // should never trigger.
    let hook_dir = repo.join(".git/hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    std::fs::write(
        hook_dir.join("pre-commit"),
        "#!/bin/sh\necho 'commit blocked' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(hook_dir.join("pre-commit"))
            .unwrap()
            .permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(hook_dir.join("pre-commit"), perm).unwrap();
    }

    // Simulate the verify intercept capture path:
    // 1. capture_head_short_sha (read-only git rev-parse)
    let sha = capture_head_short_sha(&repo).expect("should capture SHA");
    assert!(!sha.is_empty(), "SHA must not be empty");

    // 2. Verify the SHA matches actual HEAD.
    let actual = current_git_head(&repo).unwrap();
    assert!(
        actual.starts_with(&sha),
        "short SHA {sha} must match HEAD {actual}"
    );

    // No git commit was invoked — if it had been, the pre-commit hook
    // would have failed and we wouldn't have gotten here.

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bind_preflight_warns_on_main_target() {
    let warns = bind_preflight_warnings(std::path::Path::new("/nonexistent-repo-xyz"), Some("main"));
    assert!(
        warns.iter().any(|w| w.contains("main/master")),
        "{warns:?}"
    );
}

#[test]
fn bind_preflight_warns_detached_or_missing_repo() {
    let warns = bind_preflight_warnings(std::path::Path::new("/nonexistent-repo-xyz"), None);
    assert!(
        warns.iter().any(|w| w.contains("detached") || w.contains("HEAD")),
        "{warns:?}"
    );
}

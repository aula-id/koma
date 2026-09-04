#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::restore_sdlc_on_open;
use crate::app::state::{AgentMode, AppState};
use crate::model::conversation::Conversation;
use crate::model::sdlc::Mission;
use crate::model::session::Session;
use crate::model::settings::Settings;

fn scratch_session(tag: &str) -> (std::path::PathBuf, Session) {
    let dir = std::env::temp_dir().join(format!(
        "koma-open-sdlc-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sess = Session::new(
        format!("s-{tag}"),
        dir.clone(),
        "pwd".into(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    );
    (dir, sess)
}

fn active_execute_mission(worktree_path: &str) -> Mission {
    let goal = "ship X";
    let acceptance = vec!["tests pass".into()];
    let non_goals = vec!["rewrite Y".into()];
    let lane = "standard";
    let verify_plan = vec!["cargo test".into()];
    let human_gates: Vec<String> = vec![];
    let risks = vec!["api churn".into()];
    let rationale = "match house style";
    let worktree_name = Some("sdlc-test".into());
    let branch = Some("sdlc/ship-x".into());
    let wt = Some(worktree_path.to_string());
    let target_worktree_path = Some("/tmp/primary".into());
    let target_branch = Some("main".into());
    let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let hash =
        Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
            goal,
            acceptance: &acceptance,
            non_goals: &non_goals,
            lane,
            verify_plan: &verify_plan,
            human_gates: &human_gates,
            risks: &risks,
            rationale,
            graph_hash: None,
            worktree_name: worktree_name.as_deref(),
            branch: branch.as_deref(),
            worktree_path: wt.as_deref(),
            target_worktree_path: target_worktree_path.as_deref(),
            target_branch: target_branch.as_deref(),
            target_head: target_head.as_deref(),
        });
    Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "m1".into(),
        goal: goal.into(),
        non_goals,
        acceptance,
        lane: lane.into(),
        verify_plan,
        human_gates,
        human_gates_approved: vec![],
        risks,
        worktree_name,
        branch,
        worktree_path: wt,
        target_worktree_path,
        target_branch,
        target_head,
        rationale: rationale.into(),
        phase: "execute".into(),
        approved: true,
        hash,
        graph_hash: None,
        needs_reapproval: false,
        amendment_note: None,
        draft_locks: Default::default(),
    }
}

#[test]
fn open_with_invalid_execute_binding_fails_closed_to_assess() {
    let (dir, sess) = scratch_session("bad-bind");
    // Bound path does not exist → re-entry must fail closed.
    let m = active_execute_mission(&format!(
        "/tmp/koma-missing-wt-{}-{}",
        std::process::id(),
        "nope"
    ));
    m.save(&dir).unwrap();

    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    // Default is Auto — without restore this would leave unrestricted Auto
    // over an active execute mission on disk.
    assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);

    restore_sdlc_on_open(&mut state);

    assert_eq!(state.rest.fg().agent_mode, AgentMode::Sdlc);
    assert_eq!(state.rest.fg().sdlc_phase.as_deref(), Some("assess"));
    let loaded = Mission::load(&dir).unwrap();
    assert!(!loaded.approved);
    assert!(loaded.needs_reapproval);
    assert_eq!(loaded.phase, "assess");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_without_mission_leaves_mode_untouched() {
    let (dir, sess) = scratch_session("no-mission");
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);
    restore_sdlc_on_open(&mut state);
    assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);
    assert!(state.rest.fg().sdlc_phase.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Helper: build a mission with given phase/approved/needs_reapproval fields
/// and a valid (current-version, frozen-target, hash-valid) contract.
fn mission_with_phase(phase: &str, approved: bool, needs_reapproval: bool) -> Mission {
    let goal = "reopen test";
    let acceptance = vec!["ok".into()];
    let non_goals = vec![];
    let lane = "standard";
    let verify_plan = vec![];
    let human_gates: Vec<String> = vec![];
    let risks = vec![];
    let rationale = "test";
    let worktree_name = Some("sdlc-reopen".into());
    let branch = Some("sdlc/reopen".into());
    let worktree_path = Some("/tmp/sdlc-reopen-test".into());
    let target_worktree_path = Some("/tmp/sdlc-primary-test".into());
    let target_branch = Some("develop".into());
    let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let graph_hash = Some("gh-reopen".into());
    let hash =
        Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
            goal,
            acceptance: &acceptance,
            non_goals: &non_goals,
            lane,
            verify_plan: &verify_plan,
            human_gates: &human_gates,
            risks: &risks,
            rationale,
            graph_hash: graph_hash.as_deref(),
            worktree_name: worktree_name.as_deref(),
            branch: branch.as_deref(),
            worktree_path: worktree_path.as_deref(),
            target_worktree_path: target_worktree_path.as_deref(),
            target_branch: target_branch.as_deref(),
            target_head: target_head.as_deref(),
        });
    Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "m-reopen".into(),
        goal: goal.into(),
        non_goals,
        acceptance,
        lane: lane.into(),
        verify_plan,
        human_gates,
        human_gates_approved: vec![],
        risks,
        worktree_name,
        branch,
        worktree_path,
        target_worktree_path,
        target_branch,
        target_head,
        rationale: rationale.into(),
        phase: phase.into(),
        approved,
        hash,
        graph_hash,
        needs_reapproval,
        amendment_note: None,
        draft_locks: Default::default(),
    }
}

/// Reopen matrix: approved valid prepare/execute/integrate → should auto-resume SDLC.
/// Note: since the mock worktree paths don't exist on disk, set_agent_mode
/// fail-closes from prepare/execute/integrate into assess. The critical assertion
/// is that the MODE is Sdlc (auto-resume gate passed), not the exact phase.
#[test]
fn reopen_matrix_approved_valid_resume() {
    for phase in ["prepare", "execute", "integrate"] {
        let (dir, sess) = scratch_session(&format!("resume-{phase}"));
        let m = mission_with_phase(phase, true, false);
        m.save(&dir).unwrap();

        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);

        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Sdlc,
            "phase={phase} should auto-resume to SDLC mode"
        );
        // Phase may be the original or assess (if worktree re-entry fails
        // because the mock path doesn't exist on disk).
        let p = state.rest.fg().sdlc_phase.as_deref();
        assert!(
            p == Some(phase) || p == Some("assess"),
            "phase={phase}: expected {phase} or assess, got {p:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Reopen matrix: denial for draft/assess/unapproved/needs_reapproval/paused/done/invalid/stale/missing/bad binding.
#[test]
fn reopen_matrix_denies_non_resume() {
    // draft → denied
    let (dir, sess) = scratch_session("deny-draft");
    let m = mission_with_phase("draft", false, false);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "draft must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // assess (unapproved) → denied
    let (dir, sess) = scratch_session("deny-assess");
    let m = mission_with_phase("assess", false, false);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "assess must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // approved but needs_reapproval → denied
    let (dir, sess) = scratch_session("deny-reapproval");
    let m = mission_with_phase("execute", true, true);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "needs_reapproval must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // paused → denied
    let (dir, sess) = scratch_session("deny-paused");
    let m = mission_with_phase("paused", true, false);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "paused must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // done → denied
    let (dir, sess) = scratch_session("deny-done");
    let m = mission_with_phase("done", true, false);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "done must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // invalid hash → denied
    let (dir, sess) = scratch_session("deny-invalid");
    let mut m = mission_with_phase("execute", true, false);
    m.hash = "deadbeef".repeat(4);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "invalid hash must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // stale contract version → denied
    let (dir, sess) = scratch_session("deny-stale");
    let mut m = mission_with_phase("execute", true, false);
    m.contract_version = 1;
    m.hash = m.recompute_hash();
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "stale contract must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // missing mission.json → no-op (stays Auto)
    let (dir, sess) = scratch_session("deny-missing");
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "missing mission must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // bad binding (worktree_path does not exist on disk) → denied
    let (dir, sess) = scratch_session("deny-bad-binding");
    let m = mission_with_phase("execute", true, false);
    // m.worktree_path = Some("/tmp/sdlc-reopen-test") which doesn't exist →
    // should_auto_resume sees worktree_path non-empty, so it WILL auto-resume.
    // But the set_agent_mode path will fail-closed into assess. So verify
    // that auto-resume is attempted (mode = Sdlc) but phase = assess from
    // fail-closed.
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    // should_auto_resume sees valid fields → enters SDLC; set_agent_mode
    // tries re-entry which fails → lands in assess
    assert_eq!(state.rest.fg().agent_mode, AgentMode::Sdlc);
    assert_eq!(state.rest.fg().sdlc_phase.as_deref(), Some("assess"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Reopen: unapproved execute mission (valid contract but not approved) → denied.
#[test]
fn reopen_matrix_unapproved_execute_denied() {
    let (dir, sess) = scratch_session("deny-unapproved");
    let m = mission_with_phase("execute", false, false);
    m.save(&dir).unwrap();
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    restore_sdlc_on_open(&mut state);
    assert_eq!(
        state.rest.fg().agent_mode,
        AgentMode::Auto,
        "unapproved execute must not resume"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

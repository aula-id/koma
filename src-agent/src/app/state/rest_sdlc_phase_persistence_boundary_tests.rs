#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::app::state::AgentMode;
use crate::model::sdlc::Mission;

fn scratch(tag: &str) -> (std::path::PathBuf, crate::model::session::Session) {
    let dir = std::env::temp_dir().join(format!(
        "koma-sdlc-boundary-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sess = crate::model::session::Session::new(
        format!("s-{tag}"),
        dir.clone(),
        "pwd".into(),
        crate::model::settings::Settings::default(),
        crate::model::conversation::Conversation::from_messages(vec![]),
    );
    (dir, sess)
}

fn test_mission(phase: &str) -> Mission {
    let hash =
        Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
            goal: "test",
            acceptance: &[],
            non_goals: &[],
            lane: "standard",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: Some("gh-test"),
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        });
    Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "m-test".into(),
        goal: "test".into(),
        non_goals: vec![],
        acceptance: vec![],
        lane: "standard".into(),
        verify_plan: vec![],
        human_gates: vec![],
        human_gates_approved: vec![],
        risks: vec![],
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
        rationale: String::new(),
        phase: phase.into(),
        approved: false,
        hash,
        graph_hash: Some("gh-test".into()),
        needs_reapproval: false,
        amendment_note: None,
    }
}

/// Normal dual-write: both disk and runtime are updated atomically.
#[test]
fn dual_write_updates_disk_and_runtime() {
    let (dir, sess) = scratch("dual");
    let mut m = test_mission("assess");
    m.save(&dir).unwrap();

    let mut rest = AppStateRest::new();
    rest.sessions[0].session = Some(sess);
    rest.sessions[0].sdlc_phase = Some("assess".to_string());
    rest.sessions[0].pending_sdlc_keeper_llm = Some("stale".into());
    rest.sessions[0].sdlc_keeper_llm_inflight = true;
    let epoch_before = rest.sessions[0].sdlc_keeper_epoch;

    let result = rest.apply_sdlc_phase_with_mission(0, &mut m, "execute");
    assert!(result.is_ok(), "boundary should succeed: {result:?}");

    // Runtime updated.
    assert_eq!(rest.sessions[0].sdlc_phase.as_deref(), Some("execute"));
    // Keeper invalidated (phase changed).
    assert!(rest.sessions[0].pending_sdlc_keeper_llm.is_none());
    assert!(!rest.sessions[0].sdlc_keeper_llm_inflight);
    assert_eq!(rest.sessions[0].sdlc_keeper_epoch, epoch_before + 1);

    // Disk updated.
    let loaded = Mission::load(&dir).unwrap();
    assert_eq!(loaded.phase, "execute");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Missing session index returns Err and does not panic.
#[test]
fn missing_session_returns_err() {
    let mut rest = AppStateRest::new();
    let mut m = test_mission("assess");
    let result = rest.apply_sdlc_phase_with_mission(99, &mut m, "execute");
    assert!(result.is_err());
}

/// Missing/corrupt mission on disk makes apply_sdlc_phase return Err.
/// force_sdlc_assess_safe then puts runtime into non-executable assess state.
#[test]
fn missing_mission_returns_err_and_force_assess_works() {
    let (dir, sess) = scratch("no-mission");
    let mut rest = AppStateRest::new();
    rest.sessions[0].session = Some(sess);
    rest.sessions[0].sdlc_phase = Some("execute".to_string());
    rest.sessions[0].pending_sdlc_keeper_llm = Some("stale".into());
    let e0 = rest.sessions[0].sdlc_keeper_epoch;

    // No mission.json on disk → Err.
    let result = rest.apply_sdlc_phase(0, "execute");
    assert!(result.is_err());

    // Force safe assess: runtime goes to assess, keeper invalidated.
    rest.force_sdlc_assess_safe(0);
    assert_eq!(rest.sessions[0].sdlc_phase.as_deref(), Some("assess"));
    assert!(rest.sessions[0].pending_sdlc_keeper_llm.is_none());
    assert_eq!(rest.sessions[0].sdlc_keeper_epoch, e0 + 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Phase change invalidates keeper exactly once; idempotent same-phase does
/// not bump the epoch.
#[test]
fn phase_change_invalidates_keeper_exactly_once() {
    let (dir, sess) = scratch("keeper-once");
    let mut m = test_mission("assess");
    m.save(&dir).unwrap();

    let mut rest = AppStateRest::new();
    rest.sessions[0].session = Some(sess);
    rest.sessions[0].sdlc_phase = Some("assess".to_string());
    let epoch_before = rest.sessions[0].sdlc_keeper_epoch;

    // Phase changes assess → execute → exactly one bump.
    rest.apply_sdlc_phase_with_mission(0, &mut m, "execute")
        .unwrap();
    assert_eq!(rest.sessions[0].sdlc_keeper_epoch, epoch_before + 1);

    // Same phase again → no additional bump.
    rest.apply_sdlc_phase_with_mission(0, &mut m, "execute")
        .unwrap();
    assert_eq!(rest.sessions[0].sdlc_keeper_epoch, epoch_before + 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// force_sdlc_assess_safe sets assess, invalidates keeper, and is a no-op
/// for an invalid session index.
#[test]
fn force_assess_safe_sets_phase_and_is_noop_for_invalid() {
    let mut rest = AppStateRest::new();
    rest.sessions[0].agent_mode = AgentMode::Sdlc;
    rest.sessions[0].sdlc_phase = Some("execute".to_string());
    rest.sessions[0].pending_sdlc_keeper_llm = Some("stale".into());
    rest.sessions[0].sdlc_keeper_llm_inflight = true;
    rest.sessions[0].sdlc_keeper_due = true;
    let epoch_before = rest.sessions[0].sdlc_keeper_epoch;

    rest.force_sdlc_assess_safe(0);

    assert_eq!(rest.sessions[0].sdlc_phase.as_deref(), Some("assess"));
    assert!(rest.sessions[0].pending_sdlc_keeper_llm.is_none());
    assert!(!rest.sessions[0].sdlc_keeper_llm_inflight);
    assert!(!rest.sessions[0].sdlc_keeper_due);
    assert_eq!(rest.sessions[0].sdlc_keeper_epoch, epoch_before + 1);

    // Invalid index → no panic.
    rest.force_sdlc_assess_safe(99);
}

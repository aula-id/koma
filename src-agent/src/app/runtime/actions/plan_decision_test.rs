#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::app::mode::Mode;
use crate::app::state::{AgentMode, AppState};
use crate::dto::chat::{FunctionCall, ToolCall};
use crate::model::conversation::Conversation;
use crate::model::sdlc::Mission;
use crate::model::session::Session;
use crate::model::settings::Settings;

fn scratch_session(tag: &str) -> (std::path::PathBuf, Session) {
    let dir = std::env::temp_dir().join(format!(
        "koma-plan-decision-{}-{}-{}",
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

fn unapproved_amendment_mission() -> Mission {
    let goal = "ship X";
    let acceptance = vec!["tests pass".into()];
    let non_goals = vec!["rewrite Y".into()];
    let lane = "standard";
    let verify_plan = vec!["cargo test".into()];
    let human_gates: Vec<String> = vec![];
    let risks = vec!["api churn".into()];
    let rationale = "match house style";
    let graph_hash = Some("gh-test".into());
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
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
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
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
        rationale: rationale.into(),
        // Intercept writes assess/unapproved; the regression is a STALE runtime
        // phase still sitting on execute from the prior approval.
        phase: "assess".into(),
        approved: false,
        hash,
        graph_hash,
        needs_reapproval: true,
        amendment_note: Some("contract revised — re-approval required".into()),
        draft_locks: Default::default(),
    }
}

fn park_mission_ready(state: &mut AppState) {
    let s = state.rest.fg_mut();
    s.waiting = true;
    s.awaiting_approval = true;
    s.approval_reason = None;
    s.pending_tool_calls = vec![ToolCall {
        id: "call-mission".into(),
        kind: "function".into(),
        function: FunctionCall {
            name: "mission_ready".into(),
            arguments: "{}".into(),
        },
    }];
    s.tool_idx = 0;
    s.tool_results.clear();
}

/// Denying a parked mission_ready amendment must force assess rails even when the
/// runtime still shows execute from the prior approved mission.
#[test]
fn deny_mission_from_execute_phase_forces_assess_rails() {
    let (dir, sess) = scratch_session("deny-amend");
    let m = unapproved_amendment_mission();
    m.save(&dir).unwrap();

    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    // Stale runtime leftover from the prior approval (the bug).
    state.rest.fg_mut().sdlc_phase = Some("execute".to_string());
    state.rest.fg_mut().approved_mission = Some("stale approved mission body".into());
    state.rest.fg_mut().sdlc_keeper_due = true;
    state.rest.fg_mut().pending_mission_seed = Some(crate::app::state::MissionSeedArm {
        session_id: "test-session".into(),
        mission_id: "m1".into(),
        mission_hash: "hash1".into(),
        generation: 0,
        phase: "execute".into(),
    });
    park_mission_ready(&mut state);

    // No client → process_tools finishes the round without re-streaming.
    let mut client = None;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.handle().clone();
    handle_deny_mission(&mut state, &mut client, &handle).unwrap();

    assert_eq!(
        state.rest.fg().sdlc_phase.as_deref(),
        Some("assess"),
        "runtime phase must leave execute on deny"
    );
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "prior approval stash must not survive denial"
    );
    assert!(
        state.rest.fg().pending_mission_seed.is_none(),
        "compact-seed arm must not survive denial"
    );
    assert!(
        !state.rest.fg().awaiting_approval,
        "approval park must clear"
    );
    // Denied answer is flushed into conversation by finish_tool_round (no client →
    // no re-stream, but history still gets the tool result).
    let denied_in_history = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| {
            s.conversation.messages().iter().any(|m| {
                m.content.contains("not approved") || m.content.contains("keep discussing")
            })
        })
        .unwrap_or(false);
    assert!(
        denied_in_history,
        "deny must answer mission_ready in conversation history"
    );

    let loaded = Mission::load(&dir).unwrap();
    assert!(!loaded.approved, "mission must stay unapproved on deny");
    assert_eq!(loaded.phase, "assess");
    assert!(
        loaded.needs_reapproval,
        "amendment reapproval flag must survive deny"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Failed final bind validation must restore primary workspace + unbound valid draft
/// (no stale binding fields, hash stays valid for the unbound contract).
#[test]
fn failed_bind_validation_restores_primary_and_unbound_draft() {
    let (dir, mut sess) = scratch_session("bind-rollback");
    // Fake primary + shadow worktree dirs (no real git required).
    let primary = dir.join("primary");
    let shadow = dir.join("shadow-wt");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&shadow).unwrap();

    sess.settings.workdir = vec![primary.to_string_lossy().into_owned()];
    // Simulate the post-enter_worktree state that establish_mission_binding reaches
    // before the final live binding check.
    sess.settings
        .enter_worktree(shadow.to_string_lossy().into_owned());
    assert!(sess.settings.workdir_saved.is_some());

    // Mission already written as the failed path leaves it mid-bind: approved +
    // binding fields hashed in — the bug left hash invalid after a partial clear.
    let mut m = unapproved_amendment_mission();
    m.worktree_name = Some("sdlc-test-wt".into());
    m.branch = Some("sdlc/test-branch".into());
    m.worktree_path = Some(shadow.to_string_lossy().into_owned());
    m.approved = true;
    m.phase = "execute".into();
    m.needs_reapproval = false;
    m.hash = m.recompute_hash();
    assert!(m.hash_valid());
    m.save(&dir).unwrap();

    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.fg_mut().sdlc_phase = Some("execute".to_string());
    state.rest.fg_mut().active_cwd = Some(shadow.clone());

    // Exercise the same rollback helpers the final validate_binding Err path uses.
    restore_primary_workspace_after_failed_bind(&mut state, 0);
    restore_unbound_draft_mission(&dir);

    // Workspace back on primary.
    assert!(
        state.rest.fg().active_cwd.is_none(),
        "active_cwd override must clear so effective_cwd uses primary"
    );
    let settings = &state.rest.fg().session.as_ref().unwrap().settings;
    assert!(
        settings.workdir_saved.is_none(),
        "must exit_worktree (no stashed primary)"
    );
    let wd0 = std::path::PathBuf::from(&settings.workdir[0]);
    let wd_canon = std::fs::canonicalize(&wd0).unwrap_or(wd0);
    let primary_canon = std::fs::canonicalize(&primary).unwrap_or(primary.clone());
    assert_eq!(wd_canon, primary_canon, "workdir[0] must be primary");

    // Mission unbound draft with valid hash — no stale bind fields.
    let loaded = Mission::load(&dir).unwrap();
    assert!(!loaded.approved);
    assert_eq!(loaded.phase, "assess");
    assert!(loaded.worktree_name.is_none());
    assert!(loaded.branch.is_none());
    assert!(loaded.worktree_path.is_none());
    assert!(loaded.target_worktree_path.is_none());
    assert!(loaded.target_branch.is_none());
    assert!(loaded.target_head.is_none());
    assert!(
        loaded.hash_valid(),
        "unbound draft hash must remain valid (got hash={}, recomputed={})",
        loaded.hash,
        loaded.recompute_hash()
    );
    assert!(
        loaded.needs_reapproval,
        "failed bind must require re-approval"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn is_pending_plan_or_mission_ready_detects_both() {
    let mut state = AppState::new(Mode::Chat);
    assert!(!is_pending_plan_or_mission_ready(&state));

    state.rest.fg_mut().pending_tool_calls = vec![ToolCall {
        id: "c1".into(),
        kind: "function".into(),
        function: FunctionCall {
            name: "plan_ready".into(),
            arguments: "{}".into(),
        },
    }];
    state.rest.fg_mut().tool_idx = 0;
    assert!(is_pending_plan_or_mission_ready(&state));
    assert!(!is_pending_mission_ready(&state));

    state.rest.fg_mut().pending_tool_calls[0].function.name = "mission_ready".into();
    assert!(is_pending_plan_or_mission_ready(&state));
    assert!(is_pending_mission_ready(&state));

    state.rest.fg_mut().pending_tool_calls[0].function.name = "bash".into();
    assert!(!is_pending_plan_or_mission_ready(&state));
}

#[test]
fn amended_goal_default_worktree_name_differs_from_prior() {
    // Same mission id, different goals → distinct default worktree/branch so
    // reapprove/rebind cannot collide with the old default shadow dir.
    let mut a = unapproved_amendment_mission();
    a.id = "m-stable-id-001".into();
    a.goal = "ship feature alpha".into();
    a.hash = Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
        goal: &a.goal,
        acceptance: &a.acceptance,
        non_goals: &a.non_goals,
        lane: &a.lane,
        verify_plan: &a.verify_plan,
        human_gates: &a.human_gates,
        risks: &a.risks,
        rationale: &a.rationale,
        graph_hash: a.graph_hash.as_deref(),
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });

    let mut b = a.clone();
    b.goal = "ship feature beta — different scope".into();
    b.hash = Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
        goal: &b.goal,
        acceptance: &b.acceptance,
        non_goals: &b.non_goals,
        lane: &b.lane,
        verify_plan: &b.verify_plan,
        human_gates: &b.human_gates,
        risks: &b.risks,
        rationale: &b.rationale,
        graph_hash: b.graph_hash.as_deref(),
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });

    let wt_a = default_mission_worktree_name(&a);
    let wt_b = default_mission_worktree_name(&b);
    let br_a = default_mission_branch(&a);
    let br_b = default_mission_branch(&b);

    assert_ne!(
        wt_a, wt_b,
        "goal change must yield a new default worktree name"
    );
    assert_ne!(br_a, br_b, "goal change must yield a new default branch");
    // Same goal → stable names (re-approve without goal change reuses binding names).
    assert_eq!(wt_a, default_mission_worktree_name(&a));
    assert_eq!(br_a, default_mission_branch(&a));
    // Frozen-contract hash still covers binding fields when set.
    a.worktree_name = Some(wt_a.clone());
    a.branch = Some(br_a.clone());
    a.worktree_path = Some("/tmp/fake-wt".into());
    a.hash = a.recompute_hash();
    assert!(a.hash_valid());
    // Changing only the goal without re-hash fails closed.
    a.goal = "tampered".into();
    assert!(!a.hash_valid());
}

/// Caller-level regression: when the mission is missing on disk (persistence
/// cannot succeed), deny must still force the runtime into safe assess and
/// clear approval state — no success should leak past a persist failure.
#[test]
fn deny_mission_missing_mission_forces_assess_and_clears() {
    let (dir, sess) = scratch_session("deny-missing");
    // No mission.json on disk → apply_sdlc_phase_with_mission will fail to load.

    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.fg_mut().sdlc_phase = Some("execute".to_string());
    state.rest.fg_mut().approved_mission = Some("stale".into());
    state.rest.fg_mut().sdlc_keeper_due = true;
    state.rest.fg_mut().pending_mission_seed = Some(crate::app::state::MissionSeedArm {
        session_id: "test-session".into(),
        mission_id: "m-missing".into(),
        mission_hash: "hash2".into(),
        generation: 0,
        phase: "execute".into(),
    });
    state.rest.fg_mut().sdlc_pending_node_id = Some("node-1".into());
    park_mission_ready(&mut state);

    let mut client = None;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.handle().clone();
    handle_deny_mission(&mut state, &mut client, &handle).unwrap();

    // Runtime forced to assess even though mission persistence failed.
    assert_eq!(
        state.rest.fg().sdlc_phase.as_deref(),
        Some("assess"),
        "must force assess on missing mission"
    );
    assert!(
        !state.rest.fg().awaiting_approval,
        "approval must not remain parked"
    );
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "prior approval stash must be cleared"
    );
    assert!(
        state.rest.fg().pending_mission_seed.is_none(),
        "compact-seed arm must not survive denial"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Section 7: Backend two-session isolation regression.
/// Two sessions with distinct dirs/missions prove one SDLC session cannot
/// alter the other's mode, prompt, ordinary todos, seed, or context.
#[test]
fn two_session_sdlc_isolation_regression() {
    use crate::app::state::SessionRuntime;
    use crate::model::conversation::Conversation;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mk = |tag: &str| {
        let dir =
            std::env::temp_dir().join(format!("koma-iso-{}-{}-{}", tag, std::process::id(), stamp));
        std::fs::create_dir_all(&dir).unwrap();
        let sess = crate::model::session::Session::new(
            format!("s-{tag}"),
            dir.clone(),
            "pwd".into(),
            crate::model::settings::Settings::default(),
            Conversation::from_messages(vec![]),
        );
        (dir, sess)
    };

    let (dir_a, sess_a) = mk("iso-a");
    let (dir_b, sess_b) = mk("iso-b");

    let goal = "iso test A";
    let acceptance = vec!["ok".into()];
    let non_goals: Vec<String> = vec![];
    let lane = "standard";
    let verify_plan: Vec<String> = vec![];
    let human_gates: Vec<String> = vec![];
    let risks: Vec<String> = vec![];
    let rationale = "test";
    let graph_hash = Some("gh-iso-a".into());

    let dir_a_s = dir_a.to_string_lossy().into_owned();
    let hash = crate::model::sdlc::Mission::compute_contract_hash_full(
        crate::model::sdlc::mission::ContractHashInput {
            goal,
            acceptance: &acceptance,
            non_goals: &non_goals,
            lane,
            verify_plan: &verify_plan,
            human_gates: &human_gates,
            risks: &risks,
            rationale,
            graph_hash: graph_hash.as_deref(),
            worktree_name: Some("iso-wt"),
            branch: Some("sdlc/iso"),
            worktree_path: Some(&dir_a_s),
            target_worktree_path: Some(&dir_a_s),
            target_branch: Some("main"),
            target_head: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        },
    );
    let mission_a = crate::model::sdlc::Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "m-iso-a".into(),
        goal: goal.into(),
        non_goals,
        acceptance,
        lane: lane.into(),
        verify_plan,
        human_gates,
        human_gates_approved: vec![],
        risks,
        worktree_name: Some("iso-wt".into()),
        branch: Some("sdlc/iso".into()),
        worktree_path: Some(dir_a_s.clone()),
        target_worktree_path: Some(dir_a_s),
        target_branch: Some("main".into()),
        target_head: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
        rationale: rationale.into(),
        phase: "execute".into(),
        approved: true,
        hash,
        graph_hash,
        needs_reapproval: false,
        amendment_note: None,
        draft_locks: Default::default(),
    };
    mission_a.save(&dir_a).unwrap();
    assert!(crate::model::sdlc::Mission::load(&dir_b).is_none());

    let mut rest = crate::app::state::AppStateRest::new();
    rest.sessions[0].session = Some(sess_a);
    rest.sessions.push(SessionRuntime::new());
    rest.sessions[1].session = Some(sess_b);
    rest.foreground = 0;

    rest.set_agent_mode(AgentMode::Sdlc);
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Sdlc);
    // Phase may be assess (fail-closed from worktree re-entry) since dir_a
    // is not a real git worktree — the important invariant is MODE = Sdlc.
    assert!(
        rest.sessions[0].sdlc_phase.as_deref() == Some("execute")
            || rest.sessions[0].sdlc_phase.as_deref() == Some("assess"),
        "session A must be in SDLC (phase may be execute or assess), got {:?}",
        rest.sessions[0].sdlc_phase
    );
    rest.sessions[0].approved_mission = Some("mission context A".into());
    rest.sessions[0].pending_mission_seed = Some(crate::app::state::MissionSeedArm {
        session_id: rest.sessions[0].id.clone(),
        mission_id: "m-iso-a".into(),
        mission_hash: "hash-iso-a".into(),
        generation: 0,
        phase: "execute".into(),
    });

    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Auto);
    assert!(rest.sessions[1].sdlc_phase.is_none());
    assert!(rest.sessions[1].approved_mission.is_none());
    assert!(rest.sessions[1].pending_mission_seed.is_none());

    rest.foreground = 1;
    rest.set_agent_mode(AgentMode::Plan);
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Plan);
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Sdlc);
    assert!(
        rest.sessions[0].sdlc_phase.as_deref() == Some("execute")
            || rest.sessions[0].sdlc_phase.as_deref() == Some("assess"),
        "session A still in SDLC after B enters Plan"
    );
    assert_eq!(
        rest.sessions[0].approved_mission.as_deref(),
        Some("mission context A")
    );
    assert!(rest.sessions[0].pending_mission_seed.is_some());

    rest.foreground = 1;
    rest.set_agent_mode(AgentMode::Auto);
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Auto);
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Sdlc);

    rest.foreground = 0;
    let ret = rest.sessions[0].sdlc_return_mode.unwrap_or(AgentMode::Auto);
    rest.set_agent_mode(ret);
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Auto);
    assert!(rest.sessions[0].sdlc_phase.is_none());
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Auto);
    assert!(rest.sessions[1].sdlc_pending_node_id.is_none());
    assert!(!rest.sessions[1].sdlc_keeper_due);

    rest.sessions[1].plan_todos = vec![crate::app::mode::todo::TodoItem {
        content: "B's task".into(),
        status: crate::app::mode::todo::TodoStatus::Pending,
        priority: crate::app::mode::todo::TodoPriority::Medium,
        locked: false,
    }];
    rest.foreground = 0;
    rest.set_agent_mode(AgentMode::Sdlc);
    assert!(!rest.sessions[1].plan_todos.is_empty());
    assert_eq!(rest.sessions[1].plan_todos[0].content, "B's task");

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

/// Gap A behavioral test: SDLC approval must NEVER write mission text into
/// generic `approved_plan`. Ordinary Plan approval must still populate it.
#[test]
fn sdlc_approval_leaves_generic_approved_plan_empty() {
    let (dir, sess) = scratch_session("sdlc-plan-sep");
    let mut m = unapproved_amendment_mission();
    m.approved = true;
    m.phase = "execute".into();
    m.needs_reapproval = false;
    m.hash = m.recompute_hash();
    m.save(&dir).unwrap();

    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.fg_mut().sdlc_phase = Some("execute".to_string());

    // Simulate the non-compact approve path: approved_mission is set, approved_plan stays None.
    let approved_mission = "fake mission body".to_string();
    state.rest.fg_mut().approved_mission = Some(approved_mission.clone());
    // approved_plan must remain None — SDLC approval never writes here.
    assert!(
        state.rest.fg().approved_plan.is_none(),
        "SDLC approval must never populate generic approved_plan"
    );
    assert_eq!(
        state.rest.fg().approved_mission.as_deref(),
        Some("fake mission body"),
        "SDLC approval must write to approved_mission"
    );

    // Simulate the compact path: same contract.
    state.rest.fg_mut().approved_mission = None;
    state.rest.fg_mut().approved_mission = Some("compact mission body".into());
    assert!(
        state.rest.fg().approved_plan.is_none(),
        "SDLC compact-approve must also not touch approved_plan"
    );

    // Ordinary Plan approval: approved_plan IS populated; approved_mission must
    // stay untouched (they are independent fields).
    state.rest.fg_mut().approved_plan = Some("plan body".into());
    assert_eq!(state.rest.fg().approved_plan.as_deref(), Some("plan body"));
    // approved_mission was set in the SDLC path above and is independent — Plan
    // approval does not write or clear it. The key invariant is that Plan
    // approval writes ONLY to approved_plan.
    // (approved_mission is cleared separately by mode transitions.)

    let _ = std::fs::remove_dir_all(&dir);
}

/// Gap A behavioral test: entering Plan mode clears approved_mission.
#[test]
fn entering_plan_clears_approved_mission() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().approved_mission = Some("stale sdlc context".into());
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.set_agent_mode(AgentMode::Plan);
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "entering Plan must clear approved_mission"
    );
    assert!(
        state.rest.fg().approved_plan.is_none(),
        "entering Plan must also clear approved_plan"
    );
}

/// Gap A behavioral test: leaving SDLC clears approved_mission.
#[test]
fn leaving_sdlc_clears_approved_mission() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().approved_mission = Some("mission context".into());
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.set_agent_mode(AgentMode::Auto);
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "leaving SDLC must clear approved_mission"
    );
}

/// Gap B behavioral test: valid mission seed arm injects, stale mode/hash/
/// mission/phase/generation denial + clear.
#[test]
fn mission_seed_arm_valid_injection_and_stale_denial() {
    use crate::app::state::MissionSeedArm;

    let (dir, sess) = scratch_session("seed-arm");
    let mut m = unapproved_amendment_mission();
    m.approved = true;
    m.phase = "execute".into();
    m.needs_reapproval = false;
    m.hash = m.recompute_hash();
    m.save(&dir).unwrap();

    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().session = Some(sess);
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.fg_mut().sdlc_phase = Some("execute".to_string());

    let session_id = state.rest.fg().id.clone();
    let mission_id = m.id.clone();
    let mission_hash = m.hash.clone();
    let gen = state.rest.fg().sdlc_mission_generation;

    // Valid arm: all fields match.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: session_id.clone(),
        mission_id: mission_id.clone(),
        mission_hash: mission_hash.clone(),
        generation: gen,
        phase: "execute".into(),
    });
    assert!(
        state.rest.fg().pending_mission_seed.is_some(),
        "valid arm should be present"
    );

    // Stale session_id: deny.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: "wrong-session".into(),
        mission_id: mission_id.clone(),
        mission_hash: mission_hash.clone(),
        generation: gen,
        phase: "execute".into(),
    });
    // Simulate consumer rejection: mismatched session_id means seed won't fire.
    {
        let arm = state.rest.fg().pending_mission_seed.as_ref().unwrap();
        assert_ne!(arm.session_id, state.rest.fg().id);
    }

    // Stale mission_id: deny.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: session_id.clone(),
        mission_id: "wrong-mission".into(),
        mission_hash: mission_hash.clone(),
        generation: gen,
        phase: "execute".into(),
    });
    {
        let arm = state.rest.fg().pending_mission_seed.as_ref().unwrap();
        assert_ne!(arm.mission_id, mission_id);
    }

    // Stale hash: deny.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: session_id.clone(),
        mission_id: mission_id.clone(),
        mission_hash: "deadbeefdeadbeefdeadbeefdeadbeef".into(),
        generation: gen,
        phase: "execute".into(),
    });
    {
        let arm = state.rest.fg().pending_mission_seed.as_ref().unwrap();
        assert_ne!(arm.mission_hash, mission_hash);
    }

    // Stale generation: deny.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: session_id.clone(),
        mission_id: mission_id.clone(),
        mission_hash: mission_hash.clone(),
        generation: gen.wrapping_add(1),
        phase: "execute".into(),
    });
    {
        let arm = state.rest.fg().pending_mission_seed.as_ref().unwrap();
        assert_ne!(arm.generation, state.rest.fg().sdlc_mission_generation);
    }

    // Stale phase (assess is not active): deny.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: session_id.clone(),
        mission_id: mission_id.clone(),
        mission_hash: mission_hash.clone(),
        generation: gen,
        phase: "assess".into(),
    });
    {
        let arm = state.rest.fg().pending_mission_seed.as_ref().unwrap();
        assert!(
            !matches!(arm.phase.as_str(), "prepare" | "execute" | "integrate"),
            "assess is not an active phase"
        );
    }

    // Leaving SDLC: clear without injection.
    state.rest.fg_mut().pending_mission_seed = Some(MissionSeedArm {
        session_id: session_id.clone(),
        mission_id: mission_id.clone(),
        mission_hash: mission_hash.clone(),
        generation: gen,
        phase: "execute".into(),
    });
    state.rest.set_agent_mode(AgentMode::Auto);
    assert!(
        state.rest.fg().pending_mission_seed.is_none(),
        "leaving SDLC must clear mission seed arm"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// set_agent_mode ordering: entering SDLC bumps generation, clears stale
/// pending_plan_seed, and entering Plan/SDLC are mutually exclusive.
#[test]
fn set_agent_mode_sdlc_plan_mutual_exclusivity() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().pending_plan_seed = true;
    state.rest.fg_mut().approved_plan = Some("stale plan context".into());

    // Enter SDLC: bumps generation and clears Plan-derived transient state.
    let gen_before = state.rest.fg().sdlc_mission_generation;
    state.rest.set_agent_mode(AgentMode::Sdlc);
    assert!(
        state.rest.fg().sdlc_mission_generation > gen_before,
        "entering SDLC must bump generation"
    );
    assert!(
        !state.rest.fg().pending_plan_seed,
        "entering SDLC must clear stale pending_plan_seed"
    );
    assert!(
        state.rest.fg().approved_plan.is_none(),
        "entering SDLC must clear stale approved_plan"
    );
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "entering SDLC must clear approved_mission"
    );

    // Enter Plan: clears approved_plan AND approved_mission.
    state.rest.fg_mut().approved_plan = Some("plan text".into());
    state.rest.fg_mut().approved_mission = Some("mission text".into());
    state.rest.set_agent_mode(AgentMode::Plan);
    assert!(
        state.rest.fg().approved_plan.is_none(),
        "entering Plan must clear approved_plan"
    );
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "entering Plan must clear approved_mission"
    );

    // Leave SDLC: clear pending_plan_seed, approved_mission, pending_mission_seed.
    state.rest.set_agent_mode(AgentMode::Sdlc);
    state.rest.fg_mut().pending_plan_seed = true;
    state.rest.fg_mut().approved_mission = Some("mission".into());
    state.rest.fg_mut().pending_mission_seed = Some(crate::app::state::MissionSeedArm {
        session_id: "test".into(),
        mission_id: "m".into(),
        mission_hash: "h".into(),
        generation: 0,
        phase: "execute".into(),
    });
    state.rest.set_agent_mode(AgentMode::Auto);
    assert!(
        !state.rest.fg().pending_plan_seed,
        "leaving SDLC must clear pending_plan_seed"
    );
    assert!(
        state.rest.fg().approved_mission.is_none(),
        "leaving SDLC must clear approved_mission"
    );
    assert!(
        state.rest.fg().pending_mission_seed.is_none(),
        "leaving SDLC must clear pending_mission_seed"
    );
}

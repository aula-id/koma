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
    let hash = Mission::compute_contract_hash_full(
        goal,
        &acceptance,
        &non_goals,
        lane,
        &verify_plan,
        &human_gates,
        &risks,
        rationale,
        graph_hash.as_deref(),
        None,
        None,
        None,
        None,
        None,
        None,
    );
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
    state.rest.fg_mut().approved_plan = Some("stale approved mission body".into());
    state.rest.fg_mut().sdlc_keeper_due = true;
    state.rest.fg_mut().pending_mission_seed = true;
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
        state.rest.fg().approved_plan.is_none(),
        "prior approval stash must not survive denial"
    );
    assert!(
        !state.rest.fg().pending_mission_seed,
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
    a.hash = Mission::compute_contract_hash_full(
        &a.goal,
        &a.acceptance,
        &a.non_goals,
        &a.lane,
        &a.verify_plan,
        &a.human_gates,
        &a.risks,
        &a.rationale,
        a.graph_hash.as_deref(),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let mut b = a.clone();
    b.goal = "ship feature beta — different scope".into();
    b.hash = Mission::compute_contract_hash_full(
        &b.goal,
        &b.acceptance,
        &b.non_goals,
        &b.lane,
        &b.verify_plan,
        &b.human_gates,
        &b.risks,
        &b.rationale,
        b.graph_hash.as_deref(),
        None,
        None,
        None,
        None,
        None,
        None,
    );

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

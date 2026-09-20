#![allow(clippy::unwrap_used)]

use super::*;
use crate::app::mode::Mode;
use crate::app::state::{AgentMode, MissionSeedArm};
use crate::model::conversation::Conversation;
use crate::model::sdlc::Mission;
use crate::model::session::Session;
use crate::model::settings::Settings;

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bound_execute_mission() -> Mission {
    let mut mission = Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "mission-seed".into(),
        goal: "ship seed validation".into(),
        non_goals: vec![],
        acceptance: vec!["seed is injected once".into()],
        lane: "standard".into(),
        verify_plan: vec![],
        human_gates: vec![],
        human_gates_approved: vec![],
        risks: vec![],
        worktree_name: Some("wt-seed".into()),
        branch: Some("sdlc/seed".into()),
        worktree_path: Some("/tmp/wt-seed".into()),
        target_worktree_path: Some("/tmp/target-seed".into()),
        target_branch: Some("develop".into()),
        target_head: Some("0123456789012345678901234567890123456789".into()),
        rationale: "test".into(),
        phase: "execute".into(),
        approved: true,
        hash: String::new(),
        graph_hash: Some("graph-seed".into()),
        needs_reapproval: false,
        amendment_note: None,
        draft_locks: Default::default(),
    };
    mission.hash = mission.recompute_hash();
    mission
}

fn armed_state(path: &std::path::Path, mission: &Mission) -> AppState {
    let mut state = AppState::new(Mode::Chat);
    let session = Session::new(
        "seed-session".into(),
        path.to_path_buf(),
        "pwd".into(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    );
    let rt = state.rest.fg_mut();
    rt.id = "seed-session".into();
    rt.session = Some(session);
    rt.agent_mode = AgentMode::Sdlc;
    rt.sdlc_phase = Some("execute".into());
    rt.pending_mission_seed = Some(MissionSeedArm {
        session_id: rt.id.clone(),
        mission_id: mission.id.clone(),
        mission_hash: mission.hash.clone(),
        generation: rt.sdlc_mission_generation,
        phase: "execute".into(),
    });
    state
}

#[test]
fn plan_seed_compact_does_not_append_session_image_inventory() {
    let path = std::env::temp_dir().join(format!(
        "koma-drains-img-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let _scratch = Scratch(path.clone());
    let images = path.join("images");
    std::fs::create_dir_all(&images).unwrap();
    std::fs::write(
        images.join("01-a.png"),
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89",
    )
    .unwrap();
    std::fs::write(path.join("plan.md"), "Unreviewed replacement on disk\n").unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut state = AppState::new(Mode::Chat);
    let session = Session::new(
        "img-session".into(),
        path.clone(),
        "pwd".into(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    );
    {
        let rt = state.rest.fg_mut();
        rt.id = "img-session".into();
        rt.session = Some(session);
        rt.pending_plan_seed = true;
        rt.pending_plan_seed_body = Some("Reviewed body captured at approval".into());
    }
    apply_compaction_result(
        &mut state,
        0,
        &None,
        runtime.handle(),
        "summary body".into(),
        vec![],
    );
    let msgs = state
        .rest
        .fg()
        .session
        .as_ref()
        .unwrap()
        .conversation
        .messages();
    let joined: String = msgs.iter().map(|m| m.content.clone()).collect();
    assert!(
        !joined.contains("session images still on disk"),
        "must not dump session-wide image inventory: {joined}"
    );
    assert!(!joined.contains("[Image #1] images/01-a.png"));
    assert!(joined.contains("Approved plan (execute now)"));
    assert!(joined.contains("Reviewed body captured at approval"));
    assert!(!joined.contains("Unreviewed replacement on disk"));
    assert!(!state.rest.fg().pending_plan_seed);
    assert!(state.rest.fg().pending_plan_seed_body.is_none());
}

#[test]
fn plan_seed_rejects_missing_snapshot_and_read_only_modes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (mode, armed, body) in [
        (AgentMode::Auto, true, None),
        (AgentMode::Auto, false, Some("Old captured body")),
        (AgentMode::Plan, true, Some("Previously approved body")),
        (AgentMode::Sdlc, true, Some("Previously approved body")),
    ] {
        let path = std::env::temp_dir().join(format!(
            "koma-plan-seed-reject-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let _scratch = Scratch(path.clone());
        std::fs::write(path.join("plan.md"), "Historical disk plan").unwrap();
        let mut state = AppState::new(Mode::Chat);
        {
            let rt = state.rest.fg_mut();
            rt.session = Some(Session::new(
                "stale-seed".into(),
                path,
                "pwd".into(),
                Settings::default(),
                Conversation::from_messages(vec![]),
            ));
            rt.agent_mode = mode;
            rt.pending_plan_seed = armed;
            rt.pending_plan_seed_body = body.map(str::to_string);
        }
        apply_compaction_result(
            &mut state,
            0,
            &None,
            runtime.handle(),
            "Summary of prior conversation".into(),
            vec![],
        );
        let joined: String = state
            .rest
            .fg()
            .session
            .as_ref()
            .unwrap()
            .conversation
            .messages()
            .iter()
            .map(|m| m.content.as_str())
            .collect();
        assert!(
            !joined.contains("Approved plan (execute now)"),
            "mode={mode:?}"
        );
        assert!(!joined.contains("Historical disk plan"));
        assert!(!state.rest.fg().pending_plan_seed);
        assert!(state.rest.fg().pending_plan_seed_body.is_none());
    }
}

#[test]
fn execute_seed_injects_when_bound_and_stale_loaded_phase_is_cleared() {
    let path = std::env::temp_dir().join(format!(
        "koma-drains-seed-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let _scratch = Scratch(path.clone());
    let mut mission = bound_execute_mission();
    mission.save(&path).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut valid = armed_state(&path, &mission);
    apply_compaction_result(
        &mut valid,
        0,
        &None,
        runtime.handle(),
        "summary".into(),
        vec![],
    );
    assert!(valid
        .rest
        .fg()
        .session
        .as_ref()
        .unwrap()
        .conversation
        .messages()
        .iter()
        .any(|message| message.content.contains("Approved mission (execute now)")));
    assert!(valid.rest.fg().pending_mission_seed.is_none());

    // Disk phase no longer matches the arm (arm still execute from armed_state).
    mission.phase = "prepare".into();
    mission.save(&path).unwrap();
    let mut stale = armed_state(&path, &mission);
    apply_compaction_result(
        &mut stale,
        0,
        &None,
        runtime.handle(),
        "summary".into(),
        vec![],
    );
    assert!(!stale
        .rest
        .fg()
        .session
        .as_ref()
        .unwrap()
        .conversation
        .messages()
        .iter()
        .any(|message| message.content.contains("Approved mission (execute now)")));
    assert!(stale.rest.fg().pending_mission_seed.is_none());
}

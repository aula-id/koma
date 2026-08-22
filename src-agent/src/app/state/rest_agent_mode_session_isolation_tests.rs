#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::app::state::SessionRuntime;

#[test]
fn agent_mode_and_sdlc_phase_are_per_session() {
    let mut rest = AppStateRest::new();
    // Second session slot (background).
    rest.sessions.push(SessionRuntime::new());
    assert_eq!(rest.sessions.len(), 2);
    rest.foreground = 0;

    rest.set_agent_mode(AgentMode::Sdlc);
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Sdlc);
    assert_eq!(rest.sessions[0].sdlc_phase.as_deref(), Some("assess"));
    // Background session must remain Auto / no phase.
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Auto);
    assert!(rest.sessions[1].sdlc_phase.is_none());

    // Flip foreground and enter SDLC on the other session independently.
    rest.foreground = 1;
    rest.set_agent_mode(AgentMode::Sdlc);
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Sdlc);
    assert_eq!(rest.sessions[1].sdlc_phase.as_deref(), Some("assess"));
    // First session stays in its own envelope.
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Sdlc);
    assert_eq!(rest.sessions[0].sdlc_phase.as_deref(), Some("assess"));

    // Exit only the current foreground.
    let ret = rest.sessions[1].sdlc_return_mode.unwrap_or(AgentMode::Auto);
    rest.set_agent_mode(ret);
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Auto);
    assert!(rest.sessions[1].sdlc_phase.is_none());
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Sdlc);
    assert_eq!(rest.sessions[0].sdlc_phase.as_deref(), Some("assess"));
}

#[test]
fn pending_mission_seed_is_per_session() {
    let mut rest = AppStateRest::new();
    rest.sessions.push(SessionRuntime::new());
    rest.sessions[0].pending_mission_seed = Some(crate::app::state::runtime::MissionSeedArm {
        session_id: "s0".into(),
        mission_id: "m0".into(),
        mission_hash: "h0".into(),
        generation: 0,
        phase: "execute".into(),
    });
    assert!(rest.sessions[1].pending_mission_seed.is_none());
    rest.sessions[1].pending_plan_seed = true;
    assert!(rest.sessions[0].pending_mission_seed.is_some());
    assert!(!rest.sessions[0].pending_plan_seed);
}

#[test]
fn set_agent_mode_at_plan_does_not_touch_unrelated_foreground() {
    use crate::model::conversation::Conversation;
    use crate::model::session::Session;
    use crate::model::settings::Settings;

    let mut rest = AppStateRest::new();
    rest.sessions.push(SessionRuntime::new());
    assert_eq!(rest.sessions.len(), 2);

    // Distinct on-disk dirs so plan_todos / rebuild_system can write safely.
    let mk_sess = |tag: &str| {
        let dir = std::env::temp_dir().join(format!(
            "koma-plan-iso-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (
            dir.clone(),
            Session::new(
                format!("s-{tag}"),
                dir,
                "pwd".into(),
                Settings::default(),
                Conversation::from_messages(vec![]),
            ),
        )
    };
    let (dir0, sess0) = mk_sess("fg");
    let (dir1, sess1) = mk_sess("bg");
    rest.sessions[0].session = Some(sess0);
    rest.sessions[1].session = Some(sess1);

    // Foreground stays Normal; stream targets background session 1.
    rest.foreground = 0;
    rest.sessions[0].agent_mode = AgentMode::Normal;
    rest.sessions[1].agent_mode = AgentMode::Auto;

    rest.set_agent_mode_at(1, AgentMode::Plan);

    // Target only.
    assert_eq!(rest.sessions[1].agent_mode, AgentMode::Plan);
    assert_eq!(rest.sessions[1].plan_return_mode, Some(AgentMode::Auto));
    assert!(rest.sessions[1]
        .session
        .as_ref()
        .is_some_and(|s| s.plan_mode_hint));

    // Unrelated foreground untouched.
    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Normal);
    assert!(rest.sessions[0].plan_return_mode.is_none());
    assert!(rest.sessions[0]
        .session
        .as_ref()
        .is_some_and(|s| !s.plan_mode_hint));
    assert_eq!(rest.foreground, 0);

    let _ = std::fs::remove_dir_all(&dir0);
    let _ = std::fs::remove_dir_all(&dir1);
}

#[test]
fn leaving_sdlc_resets_active_cwd_to_primary_and_invalidates_keeper() {
    use crate::model::conversation::Conversation;
    use crate::model::session::Session;
    use crate::model::settings::Settings;

    let dir = std::env::temp_dir().join(format!(
        "koma-sdlc-exit-cwd-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // Fake primary + shadow worktree dirs.
    let primary = dir.join("primary");
    let shadow = dir.join("shadow-wt");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&shadow).unwrap();

    let mut settings = Settings {
        workdir: vec![primary.to_string_lossy().into_owned()],
        ..Settings::default()
    };
    // Simulate entered mission worktree.
    settings.enter_worktree(shadow.to_string_lossy().into_owned());
    assert!(settings.workdir_saved.is_some());

    let sess = Session::new(
        "s-exit".into(),
        dir.clone(),
        "pwd".into(),
        settings,
        Conversation::from_messages(vec![]),
    );

    let mut rest = AppStateRest::new();
    rest.sessions[0].session = Some(sess);
    rest.sessions[0].agent_mode = AgentMode::Sdlc;
    rest.sessions[0].sdlc_phase = Some("execute".into());
    rest.sessions[0].sdlc_return_mode = Some(AgentMode::Auto);
    rest.sessions[0].sdlc_prev_short_send = Some(false);
    rest.sessions[0].active_cwd = Some(shadow.clone());
    rest.sessions[0].pending_sdlc_keeper_llm = Some("stale inject".into());
    rest.sessions[0].sdlc_keeper_llm_inflight = true;
    rest.sessions[0].sdlc_keeper_due = true;
    let epoch_before = rest.sessions[0].sdlc_keeper_epoch;

    rest.set_agent_mode(AgentMode::Auto);

    assert_eq!(rest.sessions[0].agent_mode, AgentMode::Auto);
    assert!(rest.sessions[0].sdlc_phase.is_none());
    // active_cwd snapped to primary (post exit_worktree workdir).
    let cwd = rest.sessions[0].active_cwd.clone().expect("active_cwd set");
    let cwd_canon = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    let primary_canon = std::fs::canonicalize(&primary).unwrap_or(primary.clone());
    assert_eq!(
        cwd_canon, primary_canon,
        "exit must reset active_cwd to primary"
    );
    // settings no longer in entered worktree.
    assert!(rest.sessions[0]
        .session
        .as_ref()
        .is_some_and(|s| s.settings.workdir_saved.is_none()));
    // Keeper rails cleared + epoch bumped.
    assert!(rest.sessions[0].pending_sdlc_keeper_llm.is_none());
    assert!(!rest.sessions[0].sdlc_keeper_llm_inflight);
    assert!(!rest.sessions[0].sdlc_keeper_due);
    assert!(rest.sessions[0].sdlc_keeper_epoch > epoch_before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalidate_sdlc_keeper_llm_drops_staged_and_bumps_epoch() {
    let mut rt = SessionRuntime::new();
    rt.pending_sdlc_keeper_llm = Some("inject me".into());
    rt.sdlc_keeper_llm_inflight = true;
    rt.sdlc_keeper_due = true;
    let e0 = rt.sdlc_keeper_epoch;
    rt.invalidate_sdlc_keeper_llm();
    assert!(rt.pending_sdlc_keeper_llm.is_none());
    assert!(!rt.sdlc_keeper_llm_inflight);
    assert!(!rt.sdlc_keeper_due);
    assert_eq!(rt.sdlc_keeper_epoch, e0.wrapping_add(1));
    // Second invalidate keeps dropping + bumping (idempotent clear).
    rt.pending_sdlc_keeper_llm = Some("again".into());
    rt.invalidate_sdlc_keeper_llm();
    assert!(rt.pending_sdlc_keeper_llm.is_none());
    assert_eq!(rt.sdlc_keeper_epoch, e0.wrapping_add(2));
}

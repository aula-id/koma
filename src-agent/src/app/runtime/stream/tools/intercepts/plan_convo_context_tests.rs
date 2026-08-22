use super::*;
use crate::app::mode::Mode;

#[test]
fn stale_mission_is_ignored_outside_sdlc_but_plan_semantics_remain() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().agent_mode = AgentMode::Auto;
    state.rest.fg_mut().approved_mission = Some("stale mission".into());
    assert!(!build_convo_context(&state, 0).contains("APPROVED MISSION"));

    state.rest.fg_mut().approved_plan = Some("ordinary plan".into());
    let context = build_convo_context(&state, 0);
    assert!(context.contains("APPROVED PLAN:\nordinary plan"));
    assert!(!context.contains("APPROVED MISSION"));

    state.rest.fg_mut().approved_plan = None;
    state.rest.fg_mut().approved_plan = Some("stale plan".into());
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    let context = build_convo_context(&state, 0);
    assert!(context.contains("APPROVED MISSION:\nstale mission"));
    assert!(!context.contains("APPROVED PLAN"));
}

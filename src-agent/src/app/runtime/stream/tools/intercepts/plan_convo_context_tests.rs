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
    // Mission TAC only while prepare/execute/integrate — not bare SDLC with no phase.
    state.rest.fg_mut().sdlc_phase = None;
    let context = build_convo_context(&state, 0);
    assert!(
        !context.contains("APPROVED MISSION"),
        "done/assess/none must not inject mission TAC"
    );
    assert!(!context.contains("APPROVED PLAN"));

    state.rest.fg_mut().sdlc_phase = Some("execute".into());
    let context = build_convo_context(&state, 0);
    assert!(context.contains("APPROVED MISSION:\nstale mission"));
    assert!(!context.contains("APPROVED PLAN"));
}

#[test]
fn mission_tac_ignored_in_done_and_assess_phases() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().agent_mode = AgentMode::Sdlc;
    state.rest.fg_mut().approved_mission = Some("m".into());
    for phase in [Some("assess"), Some("done"), Some("paused"), None] {
        state.rest.fg_mut().sdlc_phase = phase.map(str::to_string);
        assert!(
            !build_convo_context(&state, 0).contains("APPROVED MISSION"),
            "phase={phase:?}"
        );
    }
}

#[test]
fn plan_mode_does_not_treat_old_approval_as_execution_authority() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.fg_mut().agent_mode = AgentMode::Plan;
    state.rest.fg_mut().approved_plan = Some("old approved implementation".into());
    assert!(!build_convo_context(&state, 0).contains("APPROVED PLAN"));
}

#[test]
fn plan_gate_denies_external_and_mutating_calls_but_keeps_inspection() {
    use crate::dto::chat::{FunctionCall, ToolCall};
    for (name, arguments, allowed) in [
        ("mcp__notes__read", "{}", false),
        ("write", "{}", false),
        (
            "git_operator",
            r#"{"args":["branch","new-feature"]}"#,
            false,
        ),
        ("browser_tabs", r#"{"action":"navigate"}"#, false),
        ("git_operator", r#"{"args":["status","--short"]}"#, true),
        ("read", r#"{"path":"notes.txt"}"#, true),
    ] {
        let mut state = AppState::new(Mode::Chat);
        state.rest.fg_mut().agent_mode = AgentMode::Plan;
        let call = ToolCall {
            id: "plan-policy".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        };
        let flow = intercept_plan_readonly_gate(&mut state, 0, &call);
        assert_eq!(
            matches!(flow, InterceptFlow::Fallthrough),
            allowed,
            "{name}: {arguments}"
        );
        assert_eq!(state.rest.fg().tool_results.len(), usize::from(!allowed));
        assert_eq!(state.rest.fg().tool_idx, usize::from(!allowed));
    }
}

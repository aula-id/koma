use super::next_mode_for_backtab;
use crate::app::state::AgentMode;

#[test]
fn assess_and_done_advance_sdlc_to_auto() {
    assert_eq!(
        next_mode_for_backtab(AgentMode::Sdlc, Some("assess"), false),
        AgentMode::Auto
    );
    assert_eq!(
        next_mode_for_backtab(AgentMode::Sdlc, Some("done"), false),
        AgentMode::Auto
    );
}

#[test]
fn execute_integrate_and_missing_phase_stay_locked() {
    assert_eq!(
        next_mode_for_backtab(AgentMode::Sdlc, Some("execute"), false),
        AgentMode::Sdlc
    );
    assert_eq!(
        next_mode_for_backtab(AgentMode::Sdlc, Some("integrate"), false),
        AgentMode::Sdlc
    );
    assert_eq!(
        next_mode_for_backtab(AgentMode::Sdlc, None, false),
        AgentMode::Sdlc
    );
    assert_eq!(
        next_mode_for_backtab(AgentMode::Sdlc, Some("paused"), false),
        AgentMode::Sdlc
    );
}

#[test]
fn non_sdlc_modes_use_normal_cycle() {
    assert_eq!(
        next_mode_for_backtab(AgentMode::Auto, None, false),
        AgentMode::Normal
    );
    assert_eq!(
        next_mode_for_backtab(AgentMode::Normal, None, false),
        AgentMode::Plan
    );
    assert_eq!(
        next_mode_for_backtab(AgentMode::Plan, None, false),
        AgentMode::Sdlc
    );
}

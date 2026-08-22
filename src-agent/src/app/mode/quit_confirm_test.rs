use super::*;

#[test]
fn new_state_starts_in_choice_phase() {
    let s = QuitConfirmState::new(2, 5);
    assert_eq!(s.phase, QuitConfirmPhase::Choice);
    assert!(!s.is_exiting());
    assert_eq!(s.working, 2);
    assert_eq!(s.total, 5);
    assert_eq!(s.selected, 2); // cancel is safe default
}

#[test]
fn exiting_state_has_exiting_phase() {
    let s = QuitConfirmState::exiting(0);
    assert_eq!(s.phase, QuitConfirmPhase::Exiting);
    assert_eq!(s.selected, 0);
    assert!(s.is_exiting());
}

#[test]
fn phase_transition_to_exiting() {
    let mut s = QuitConfirmState::new(1, 3);
    assert_eq!(s.phase, QuitConfirmPhase::Choice);
    s.phase = QuitConfirmPhase::Exiting;
    assert!(s.is_exiting());
}

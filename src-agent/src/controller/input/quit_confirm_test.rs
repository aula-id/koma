use super::*;
use crate::app::mode::QuitConfirmPhase;
use ratatui::crossterm::event::KeyModifiers;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn choice_phase_dispatches_actions() {
    let mut s = QuitConfirmState::new(1, 3);
    let mut rest = AppStateRest::default();

    // Enter on default (cancel) → QuitCancel
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Enter)),
        Action::QuitCancel
    ));

    // k shortcut → QuitKillAll
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Char('k'))),
        Action::QuitKillAll
    ));

    // d shortcut → QuitDetach
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Char('d'))),
        Action::QuitDetach
    ));
}

#[test]
fn choice_phase_navigates() {
    let mut s = QuitConfirmState::new(1, 3);
    let mut rest = AppStateRest::default();

    assert_eq!(s.selected, 2);
    handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Left));
    assert_eq!(s.selected, 1);
    handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Left));
    assert_eq!(s.selected, 0);
    // Can't go below 0
    handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Left));
    assert_eq!(s.selected, 0);
}

#[test]
fn exiting_phase_swallows_all_keys() {
    let mut s = QuitConfirmState::new(1, 3);
    s.phase = QuitConfirmPhase::Exiting;
    let mut rest = AppStateRest::default();

    // All of these should return Action::None
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Enter)),
        Action::None
    ));
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Char('k'))),
        Action::None
    ));
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Char('d'))),
        Action::None
    ));
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Esc)),
        Action::None
    ));
    assert!(matches!(
        handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Left)),
        Action::None
    ));
}

#[test]
fn exiting_phase_does_not_move_selection() {
    let mut s = QuitConfirmState::new(1, 3);
    s.selected = 0;
    s.phase = QuitConfirmPhase::Exiting;
    let mut rest = AppStateRest::default();

    handle_quit_confirm(&mut s, &mut rest, key(KeyCode::Right));
    assert_eq!(s.selected, 0, "selection must not change in Exiting phase");
}

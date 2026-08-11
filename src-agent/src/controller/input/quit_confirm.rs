//! Key handler for the `/quit` confirm overlay (`Mode::QuitConfirm`).
//!
//! The overlay is a navigable horizontal button row — `[close window (quit)]`,
//! `[detach]`, `[cancel]` (indices 0/1/2). Two ways to drive it:
//!
//!   * NAVIGATE then activate: Left/Right (or `h`/`l`) and Tab/Shift+Tab move
//!     focus across the row (mutating `s.selected`); Enter activates the focused
//!     button.
//!   * DIRECT shortcuts: `k` close window (quit), `d` detach, `Esc` cancel — fire
//!     their action immediately regardless of focus. (`Ctrl+C` is intercepted
//!     globally in [`super::handle_key`] before this handler ever sees it.)
//!
//! NOTE: this LOCAL handler runs only in the single-process TUI (and the headless
//! daemon, which never has a TTY in the overlay) — an ATTACHED client intercepts
//! these keys client-side in `client::input::handle_quit_confirm_key`, where `[k]`
//! KILLS this window's session-daemon (daemon-per-session) and `[d]` detaches it,
//! rather than the local whole-process quit this handler performs.
//!
//! Activation maps the focused index to an action:
//!   `0` → [`Action::QuitKillAll`], `1` → [`Action::QuitDetach`],
//!   `2` → [`Action::QuitCancel`].
//!
//! The overlay carries its own snapshot state (the busy-session count + focused
//! index), so `_rest` is unused — mirroring [`super::handle_session_hub`].

use super::Action;
use crate::app::mode::QuitConfirmState;
use crate::app::state::AppStateRest;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// Map a focused button index to its action. Order matches the view + the
/// event-loop click hit-test: `0` = close window (quit), `1` = minimize (detach),
/// `2` = cancel. Out-of-range falls back to the safe cancel.
fn action_for(idx: usize) -> Action {
    match idx {
        0 => Action::QuitKillAll,
        1 => Action::QuitDetach,
        _ => Action::QuitCancel,
    }
}

/// Handle a key press inside the quit-confirm overlay.
///
/// Navigation (Left/Right, `h`/`l`, Tab/Shift+Tab) mutates `s.selected` and
/// returns [`Action::None`]; Enter activates the focused button; the direct
/// `k`/`d`/`Esc` shortcuts fire immediately. Every other key is swallowed so a
/// stray press can't leak into the chat input underneath or accidentally exit.
///
/// While in the `Exiting` phase, ALL keys are swallowed — the user has already
/// committed to an action and we show a braille spinner until the process exits.
pub fn handle_quit_confirm(
    s: &mut QuitConfirmState,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    // Exiting phase: swallow every key to prevent duplicate quit/kill requests.
    if s.is_exiting() {
        let _ = key; // suppress unused-variable warning
        return Action::None;
    }
    match key.code {
        // --- Navigate the button row (focus only; no action) ---
        KeyCode::Left | KeyCode::Char('h') => {
            s.selected = s.selected.saturating_sub(1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') => {
            s.selected = (s.selected + 1).min(2);
            Action::None
        }
        KeyCode::Tab => {
            s.selected = (s.selected + 1) % 3;
            Action::None
        }
        // crossterm reports Shift+Tab as BackTab.
        KeyCode::BackTab => {
            s.selected = (s.selected + 2) % 3;
            Action::None
        }
        // --- Activate the focused button ---
        KeyCode::Enter => action_for(s.selected),
        // --- Direct shortcuts (fire regardless of focus) ---
        KeyCode::Char('k') | KeyCode::Char('K') => Action::QuitKillAll,
        KeyCode::Char('d') | KeyCode::Char('D') => Action::QuitDetach,
        KeyCode::Esc => Action::QuitCancel,
        // No text entry: swallow every other key so nothing leaks through.
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
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
}

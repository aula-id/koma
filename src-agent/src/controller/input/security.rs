//! Key handler for the `/security` daemon control panel (`Mode::Security`).
//!
//! A read-only control panel — no sub-modes and no editing, so the dispatch is
//! simple: escape/quit, cursor navigation, and daemon lifecycle keys.
//!
//! Key map:
//! - `Esc`      → `Action::CloseSecurity` (return to Chat)
//! - `Ctrl+C`   → `Action::Quit`
//! - `Up`       → move cursor up in the ACTIVE pane (tools or deps)
//! - `Down`     → move cursor down in the ACTIVE pane (tools or deps)
//! - `h`/`H`    → toggle the body pane (tools ⇄ dependencies); mode-state only
//! - `i`/`I`    → (deps pane only) `Action::SecurityInstall` the selected dependency
//! - `t`        → `Action::SecurityToggle` (enable/disable + start/stop)
//! - `s`        → `Action::SecurityStart`
//! - `x`        → `Action::SecurityStop`
//! - `r`        → `Action::SecurityRestart`
//! - `Enter`/`Space` → `Action::SecurityToggleTool` (toggle the selected tool active)
//! - `d`        → `Action::SecurityToggleDomain` (toggle every tool in the selected tool's domain)

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::SecurityState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/security` control panel.
pub fn handle_security(s: &mut SecurityState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Keep the cursor clamp honest: the daemon may have started/stopped since the
    // panel opened (the view re-reads status live, but this mode-state copy didn't),
    // so refresh from the live manager before handling navigation.
    if let Some(m) = rest.sec_manager.as_ref() {
        s.status = m.status();
    }
    // Keep the mode-state's inactive mirror in step with the authoritative set on
    // `rest` (the action handlers mutate `rest.sec_inactive` then refresh, but a
    // re-entry into the panel reads this on each key).
    s.inactive = rest.sec_inactive.clone();
    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => Action::CloseSecurity,
        KeyCode::Up => {
            s.move_up();
            Action::None
        }
        KeyCode::Down => {
            s.move_down();
            Action::None
        }
        KeyCode::Enter | KeyCode::Char(' ') => Action::SecurityToggleTool,
        KeyCode::Char('d') | KeyCode::Char('D') => Action::SecurityToggleDomain,
        // Toggle the body pane (tools ⇄ dependencies). Pure mode-state mutation, like
        // the cursor moves — no runtime round-trip needed.
        KeyCode::Char('h') | KeyCode::Char('H') => {
            s.toggle_health_view();
            Action::None
        }
        // Install/repair the selected dependency — ONLY in the dependency pane, and only
        // when a row is actually selected.
        KeyCode::Char('i') | KeyCode::Char('I') => {
            if s.health_view {
                match s.install_health.get(s.health_selected) {
                    Some(e) => Action::SecurityInstall(e.key.clone()),
                    None => Action::None,
                }
            } else {
                Action::None
            }
        }
        KeyCode::Char('t') | KeyCode::Char('T') => Action::SecurityToggle,
        KeyCode::Char('s') | KeyCode::Char('S') => Action::SecurityStart,
        KeyCode::Char('x') | KeyCode::Char('X') => Action::SecurityStop,
        KeyCode::Char('r') | KeyCode::Char('R') => Action::SecurityRestart,
        _ => Action::None,
    }
}

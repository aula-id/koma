//! Key handler for the `/security` daemon control panel (`Mode::Security`).
//!
//! Key map:
//! - `Esc`      → `Action::CloseSecurity`
//! - `Up`/`Down` → move cursor in the active pane
//! - `h`/`H`    → cycle body panes (tools → deps → refused)
//! - `i`/`I`    → (deps pane) install selected dependency
//! - `r`        → restart daemon
//! - `Enter`/`Space` → toggle selected row (daemon / YOLO / tool)
//! - `d`        → toggle selected tool's domain
//! - `a`        → enable all tools
//! - `n`        → disable all tools
//! - `u`        → refresh embedded daemon sources + restart
//! - `c`        → try start container runtime (colima when present)
//! - `l`        → cycle cyber lane

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::{SecPane, SecurityState};
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key press inside the `/security` control panel.
pub fn handle_security(s: &mut SecurityState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Keep the cursor clamp honest: the daemon may have started/stopped since the
    // panel opened. `status()` is memory + cached runtime only (no subprocess probes).
    if let Some(m) = rest.sec_manager.as_ref() {
        s.status = m.status();
    }
    // Self-heal install-health once the daemon is up.
    if s.install_health.is_empty()
        && s.status.running
        && crate::app::runtime::commands::security::kick_off_health_probe(rest)
    {
        s.health_fetching = true;
    }
    s.inactive = rest.sec_inactive.clone();
    s.yolo_armed = rest.yolo_armed;
    // Lane label for the status strip.
    s.lane_label = rest
        .fg()
        .session
        .as_ref()
        .and_then(|x| x.settings.cyber_lane)
        .map(|l| l.label().to_string())
        .unwrap_or_else(|| "none".into());

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
        KeyCode::Char('a') | KeyCode::Char('A') => Action::SecurityEnableAllTools,
        KeyCode::Char('n') | KeyCode::Char('N') => Action::SecurityDisableAllTools,
        KeyCode::Char('u') | KeyCode::Char('U') => Action::SecurityRefreshToolkit,
        KeyCode::Char('c') | KeyCode::Char('C') => Action::SecurityEnsureRuntime,
        KeyCode::Char('l') | KeyCode::Char('L') => {
            use crate::app::sec::lane::Lane;
            let next = match rest.fg().session.as_ref().and_then(|x| x.settings.cyber_lane) {
                None => Some(Lane::Research),
                Some(Lane::Research) => Some(Lane::Red),
                Some(Lane::Red) => Some(Lane::Blue),
                Some(Lane::Blue) => None,
            };
            Action::SecurityLane(next)
        }
        KeyCode::Char('h') | KeyCode::Char('H') => {
            s.cycle_pane();
            Action::None
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            if s.pane == SecPane::Deps {
                match s
                    .health_items()
                    .get(s.health_selected)
                    .and_then(|&idx| s.install_health.get(idx))
                {
                    Some(e) => Action::SecurityInstall(e.key.clone()),
                    None => Action::None,
                }
            } else {
                Action::None
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => Action::SecurityRestart,
        _ => Action::None,
    }
}

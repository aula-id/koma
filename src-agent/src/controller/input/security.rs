//! Key handler for the `/security` daemon control panel (`Mode::Security`).
//!
//! Letters (do not drift):
//! - `s` start/stop daemon
//! - `c` runtime
//! - `u` refresh dashboard (catalog index + images) — not a daemon restart
//! - `o` arm/disarm YOLO
//! - `e` enter/leave yolo mode
//! - `l` lane
//! - `h` cycle panes · `1` `2` `3` jump
//! - `f` filter · `/` search
//! - `p` `a` `x` pull/activate/remove (Images: selected composition; Tools: selected
//!   tool's image, only when it is a real composition and needs the op)
//! - `r` Images refresh only
//! - `y` `n` confirms only (no security confirm) → `Action::None`
//! - Space/Enter activate on Images; no-op on Tools/Refused
//! - Esc closes, unless catalog search is active (then clears the query)

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::{SecPane, SecurityState};

use super::Action;

/// Handle a key press inside the `/security` control panel.
pub fn handle_security(
    s: &mut SecurityState,
    rest: &mut crate::app::state::AppStateRest,
    key: KeyEvent,
) -> Action {
    // Keep the cursor clamp honest: the daemon may have started/stopped since the
    // panel opened. `status()` is memory + cached runtime only (no subprocess probes).
    if let Some(m) = rest.sec_manager.as_ref() {
        s.status = m.status();
    }
    s.inactive = rest.sec_inactive.clone();
    s.yolo_armed = rest.yolo_armed;
    s.lane_label = rest
        .fg()
        .session
        .as_ref()
        .and_then(|x| x.settings.cyber_lane)
        .map(|l| l.label().to_string())
        .unwrap_or_else(|| "none".into());
    s.agent_mode_label = rest.agent_mode().label().to_string();

    if s.catalog_searching {
        return handle_catalog_search(s, key);
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
        KeyCode::Left => {
            if s.pane == SecPane::Tools {
                s.cycle_map(-1);
            }
            Action::None
        }
        KeyCode::Right => {
            if s.pane == SecPane::Tools {
                s.cycle_map(1);
            }
            Action::None
        }
        KeyCode::Enter | KeyCode::Char(' ') => images_activate_or_none(s),
        KeyCode::Char('1') => {
            s.set_pane(SecPane::Tools);
            Action::None
        }
        KeyCode::Char('2') => {
            s.set_pane(SecPane::Images);
            Action::None
        }
        KeyCode::Char('3') => {
            s.set_pane(SecPane::Refused);
            Action::None
        }
        KeyCode::Char('h') | KeyCode::Char('H') => {
            s.cycle_pane();
            Action::None
        }
        KeyCode::Char('f') | KeyCode::Char('F') => {
            s.cycle_filter();
            Action::None
        }
        KeyCode::Char('/') => {
            s.catalog_searching = true;
            s.catalog_query.clear();
            s.selected = 0;
            s.clamp_cursors();
            Action::None
        }
        KeyCode::Char('s') | KeyCode::Char('S') => Action::SecurityToggleDaemon,
        KeyCode::Char('c') | KeyCode::Char('C') => Action::SecurityEnsureRuntime,
        KeyCode::Char('u') | KeyCode::Char('U') => Action::SecurityRefreshDashboard,
        KeyCode::Char('o') | KeyCode::Char('O') => Action::SecurityToggleYolo,
        KeyCode::Char('e') | KeyCode::Char('E') => Action::SecurityEnterYolo,
        KeyCode::Char('l') | KeyCode::Char('L') => lane_action(rest),
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('n') | KeyCode::Char('N') => {
            Action::None
        }
        KeyCode::Char('p') | KeyCode::Char('P') => pull_action(s),
        KeyCode::Char('a') | KeyCode::Char('A') => activate_action(s),
        KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Delete => remove_action(s),
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if s.pane == SecPane::Images {
                Action::SecurityImageRefresh
            } else {
                Action::None
            }
        }
        _ => Action::None,
    }
}

fn handle_catalog_search(s: &mut SecurityState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.catalog_searching = false;
            s.catalog_query.clear();
            s.selected = 0;
            s.clamp_cursors();
            Action::None
        }
        KeyCode::Backspace => {
            s.catalog_query.pop();
            s.selected = 0;
            s.clamp_cursors();
            Action::None
        }
        KeyCode::Up => {
            s.move_up();
            Action::None
        }
        KeyCode::Down => {
            s.move_down();
            Action::None
        }
        KeyCode::Left => {
            if s.pane == SecPane::Tools {
                s.cycle_map(-1);
            }
            Action::None
        }
        KeyCode::Right => {
            if s.pane == SecPane::Tools {
                s.cycle_map(1);
            }
            Action::None
        }
        KeyCode::Enter => {
            s.catalog_searching = false;
            s.selected = 0;
            s.clamp_cursors();
            Action::None
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            s.catalog_query.push(c);
            s.selected = 0;
            s.clamp_cursors();
            Action::None
        }
        _ => Action::None,
    }
}

fn lane_action(rest: &crate::app::state::AppStateRest) -> Action {
    use crate::app::sec::lane::Lane;
    let next = match rest.fg().session.as_ref().and_then(|x| x.settings.cyber_lane) {
        None => Some(Lane::Research),
        Some(Lane::Research) => Some(Lane::Red),
        Some(Lane::Red) => Some(Lane::Blue),
        Some(Lane::Blue) => None,
    };
    Action::SecurityLane(next)
}

fn images_activate_or_none(s: &SecurityState) -> Action {
    if s.pane == SecPane::Images {
        match s.selected_image() {
            Some(row) => Action::SecurityImageActivate(row.id.clone()),
            None => Action::None,
        }
    } else {
        Action::None
    }
}

fn pull_action(s: &SecurityState) -> Action {
    if s.pane == SecPane::Images {
        return match s.selected_image() {
            Some(row) => Action::SecurityImagePull(row.id.clone()),
            None => Action::None,
        };
    }
    if s.pane == SecPane::Tools {
        if let Some(row) = s.selected_catalog() {
            if crate::app::mode::security::is_real_composition(&row.image) && row.ready_mark == "remote"
            {
                return Action::SecurityImagePull(row.image.clone());
            }
        }
    }
    Action::None
}

fn activate_action(s: &SecurityState) -> Action {
    if s.pane == SecPane::Images {
        return match s.selected_image() {
            Some(row) => Action::SecurityImageActivate(row.id.clone()),
            None => Action::None,
        };
    }
    if s.pane == SecPane::Tools {
        if let Some(row) = s.selected_catalog() {
            if crate::app::mode::security::is_real_composition(&row.image)
                && row.ready_mark == "local"
            {
                return Action::SecurityImageActivate(row.image.clone());
            }
        }
    }
    Action::None
}

fn remove_action(s: &SecurityState) -> Action {
    if s.pane == SecPane::Images {
        return match s.selected_image() {
            Some(row) => Action::SecurityImageRemove(row.id.clone()),
            None => Action::None,
        };
    }
    if s.pane == SecPane::Tools {
        if let Some(row) = s.selected_catalog() {
            let local_idle = row.ready_mark == "local"
                && s.images
                    .iter()
                    .find(|img| img.id.eq_ignore_ascii_case(&row.image))
                    .map(|img| img.local && !img.active)
                    .unwrap_or(false);
            if crate::app::mode::security::is_real_composition(&row.image) && local_idle {
                return Action::SecurityImageRemove(row.image.clone());
            }
        }
    }
    Action::None
}

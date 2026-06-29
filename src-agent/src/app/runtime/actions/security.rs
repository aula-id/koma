//! Action handlers for the `/security` daemon control panel.
//!
//! Actions:
//! - `CloseSecurity`        — return to Chat.
//! - `SecurityToggle`       — flip `security_enabled`; start or stop the daemon accordingly.
//! - `SecurityStart`        — start the daemon (no-op when already running).
//! - `SecurityStop`         — stop the daemon (no-op when not running).
//! - `SecurityRestart`      — stop then start the daemon.
//! - `SecurityToggleTool`   — toggle the selected tool's active state (membership in
//!   `state.rest.sec_inactive`).
//! - `SecurityToggleDomain` — toggle every tool sharing the selected tool's domain.
//!
//! After every lifecycle action the open `Mode::Security` state is refreshed from
//! the live manager so the panel reflects the new daemon state immediately.

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;

/// Handle `Action::CloseSecurity`: return to Chat.
pub(super) fn handle_close_security(state: &mut AppState) -> Result<()> {
    state.mode = Mode::Chat;
    state.rest.status = "ready".into();
    Ok(())
}

/// Handle `Action::SecurityToggle`: flip `security_enabled`.
///
/// If now enabled → start the daemon (or no-op when already running).
/// If now disabled → stop the daemon.
/// Refreshes the open panel's status snapshot after the state change.
pub(super) fn handle_security_toggle(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = !state.rest.security_enabled;
    if state.rest.security_enabled {
        if let Some(m) = state.rest.sec_manager.as_ref() {
            m.start(state.rest.sec_token.clone());
        }
        state.rest.status = "security: enabling daemon…".into();
    } else {
        if let Some(m) = state.rest.sec_manager.as_ref() {
            m.stop();
        }
        state.rest.status = "security: daemon stopped".into();
    }
    refresh_security_state(state);
    Ok(())
}

/// Handle `Action::SecurityStart`: start the daemon.
///
/// Sets `security_enabled` to true so subsequent turns advertise sec_ tools.
pub(super) fn handle_security_start(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = true;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.start(state.rest.sec_token.clone());
    }
    state.rest.status = "security: starting daemon…".into();
    refresh_security_state(state);
    Ok(())
}

/// Handle `Action::SecurityStop`: stop the daemon.
///
/// Clears `security_enabled` so sec_ tools are no longer advertised.
pub(super) fn handle_security_stop(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = false;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.stop();
    }
    state.rest.status = "security: daemon stopped".into();
    refresh_security_state(state);
    Ok(())
}

/// Handle `Action::SecurityRestart`: stop then start the daemon.
///
/// Keeps `security_enabled = true` so sec_ tools remain advertised after restart.
pub(super) fn handle_security_restart(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = true;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.restart(state.rest.sec_token.clone());
    }
    state.rest.status = "security: restarting daemon…".into();
    refresh_security_state(state);
    Ok(())
}

/// Handle `Action::SecurityToggleTool`: flip the currently-selected tool's active
/// state by toggling its name in `state.rest.sec_inactive`.
///
/// The selected tool is `status.tools[selected]`. If it is currently active (not in
/// the set) it is disabled (inserted); if disabled it is re-enabled (removed). A no-op
/// when there is no selected tool (empty inventory). Refreshes the panel after.
pub(super) fn handle_security_toggle_tool(state: &mut AppState) -> Result<()> {
    // Resolve the selected tool's name out of the open panel, then mutate `rest`.
    let name = if let Mode::Security(s) = &state.mode {
        s.status.tools.get(s.selected).map(|t| t.name.clone())
    } else {
        None
    };
    if let Some(name) = name {
        if state.rest.sec_inactive.contains(&name) {
            state.rest.sec_inactive.remove(&name);
            state.rest.status = format!("security: {name} enabled");
        } else {
            state.rest.sec_inactive.insert(name.clone());
            state.rest.status = format!("security: {name} disabled");
        }
    }
    refresh_security_state(state);
    Ok(())
}

/// Handle `Action::SecurityToggleDomain`: toggle every tool sharing the selected
/// tool's domain.
///
/// If ALL tools in that domain are currently active, disable them all; otherwise (any
/// already disabled) enable them all. A no-op when there is no selected tool. Refreshes
/// the panel after.
pub(super) fn handle_security_toggle_domain(state: &mut AppState) -> Result<()> {
    // Resolve the selected tool's domain + every tool name in it out of the panel.
    let domain_tools: Option<(String, Vec<String>)> = if let Mode::Security(s) = &state.mode {
        s.status.tools.get(s.selected).map(|sel| {
            let domain = sel.domain.clone();
            let names = s
                .status
                .tools
                .iter()
                .filter(|t| t.domain == domain)
                .map(|t| t.name.clone())
                .collect::<Vec<_>>();
            (domain, names)
        })
    } else {
        None
    };
    if let Some((domain, names)) = domain_tools {
        // Disable the whole domain only when every member is currently active;
        // otherwise enable all (so a mixed/partly-disabled domain flips fully on).
        let all_active = names.iter().all(|n| !state.rest.sec_inactive.contains(n));
        if all_active {
            for n in &names {
                state.rest.sec_inactive.insert(n.clone());
            }
            state.rest.status = format!("security: domain [{domain}] disabled");
        } else {
            for n in &names {
                state.rest.sec_inactive.remove(n);
            }
            state.rest.status = format!("security: domain [{domain}] enabled");
        }
    }
    refresh_security_state(state);
    Ok(())
}

/// Re-read the live daemon status from the manager and refresh the open
/// `Mode::Security` state so the panel updates immediately. If the mode is
/// not `Security` (the action was dispatched from somewhere else), this is a
/// no-op — the panel will pick up fresh status the next time it opens.
fn refresh_security_state(state: &mut AppState) {
    let status = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.status())
        .unwrap_or_default();
    let inactive = state.rest.sec_inactive.clone();
    if let Mode::Security(s) = &mut state.mode {
        s.refresh(status, inactive);
    }
}

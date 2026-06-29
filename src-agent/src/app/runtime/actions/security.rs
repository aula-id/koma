//! Action handlers for the `/security` daemon control panel.
//!
//! Five actions:
//! - `CloseSecurity`    — return to Chat.
//! - `SecurityToggle`   — flip `security_enabled`; start or stop the daemon accordingly.
//! - `SecurityStart`    — start the daemon (no-op when already running).
//! - `SecurityStop`     — stop the daemon (no-op when not running).
//! - `SecurityRestart`  — stop then start the daemon.
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

/// Re-read the live daemon status from the manager and refresh the open
/// `Mode::Security` state so the panel updates immediately. If the mode is
/// not `Security` (the action was dispatched from somewhere else), this is a
/// no-op — the panel will pick up fresh status the next time it opens.
fn refresh_security_state(state: &mut AppState) {
    if let Mode::Security(s) = &mut state.mode {
        let status = state
            .rest
            .sec_manager
            .as_ref()
            .map(|m| m.status())
            .unwrap_or_default();
        s.refresh(status);
    }
}

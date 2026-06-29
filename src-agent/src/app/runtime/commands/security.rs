//! The `/security` command: open the security daemon control panel.

use anyhow::Result;

use crate::app::mode::{Mode, SecurityState};
use crate::app::state::AppState;

/// Handle the `/security` command: open the security daemon control panel.
///
/// Does NOT require an active session — the control panel is global state
/// (daemon lifecycle), not session-scoped. Opens regardless of whether the
/// daemon is installed or running (it shows the "not installed" / "stopped"
/// state faithfully so the user can see why tools aren't appearing).
pub(super) fn handle_security(state: &mut AppState) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    // Read live status from the manager; fall back to Default (not installed,
    // not running, no tools) when there is no manager.
    let status = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.status())
        .unwrap_or_default();
    // Fetch the per-dependency install-health ONCE on open. `health()` is a heavy IPC
    // round-trip, so it is fetched here (and after an install) and carried thereafter —
    // never on a plain refresh. `Err` / no manager / daemon stopped → empty vec, and
    // the panel still opens (the deps pane just shows "no health data").
    let install_health = state
        .rest
        .sec_manager
        .as_ref()
        .and_then(|m| m.health().ok())
        .unwrap_or_default();
    let st = SecurityState::new(status, state.rest.sec_inactive.clone(), install_health);
    state.mode = Mode::Security(Box::new(st));
    Ok(())
}

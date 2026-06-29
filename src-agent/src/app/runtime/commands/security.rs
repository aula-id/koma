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
    let st = SecurityState::new(status);
    state.mode = Mode::Security(Box::new(st));
    Ok(())
}

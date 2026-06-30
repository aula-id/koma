//! The `/security` command: open the security daemon control panel.

use anyhow::Result;

use crate::app::mode::{Mode, SecurityState};
use crate::app::state::{AppState, AppStateRest};

/// Kick off a NON-BLOCKING per-dependency health probe if one is warranted, stashing the
/// receiver in `rest.sec_health_rx` for `service_global` to drain. Returns `true` iff a
/// probe was actually started (so the caller can flip `SecurityState::health_fetching`).
///
/// The single place that starts a probe, so panel-open and the input-path self-heal share
/// identical kick-off semantics. A probe is started only when:
/// - a daemon manager exists (`rest.sec_manager` is `Some`), AND
/// - the daemon is running (a stopped daemon has no health to report — `health()` would
///   just error), AND
/// - no probe is already in flight (`rest.sec_health_rx.is_none()` — don't double-fire).
///
/// `health_async` returns immediately (the blocking IPC runs on a blocking-pool thread),
/// so this never blocks the UI/input thread.
pub(crate) fn kick_off_health_probe(rest: &mut AppStateRest) -> bool {
    if rest.sec_health_rx.is_some() {
        return false;
    }
    let Some(mgr) = rest.sec_manager.as_ref() else {
        return false;
    };
    if !mgr.is_running() {
        return false;
    }
    rest.sec_health_rx = Some(mgr.health_async());
    true
}

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
    // Per-dependency install-health is fetched NON-BLOCKING. `health()` is a cold ~1-2s
    // IPC round-trip; running it here on the UI thread froze the panel on first open.
    // Instead open with an EMPTY list and kick off the async probe (when the daemon is
    // running) — `service_global` drains the result into `install_health` and clears the
    // spinner. A stopped daemon / no manager starts no probe (the deps pane shows
    // "no health data" until it is started, exactly as before).
    let fetching = kick_off_health_probe(&mut state.rest);
    let mut st = SecurityState::new(
        status,
        state.rest.sec_inactive.clone(),
        state.rest.yolo_armed,
        Vec::new(),
    );
    st.health_fetching = fetching;
    state.mode = Mode::Security(Box::new(st));
    Ok(())
}

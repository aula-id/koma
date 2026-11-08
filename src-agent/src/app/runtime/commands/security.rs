//! The `/security` command: open the security daemon control panel.

use anyhow::Result;

use crate::app::mode::{Mode, SecurityState};
use crate::app::state::AppState;

/// Reload the dashboard catalog + image inventory.
///
/// Seeds from the embedded index immediately (so a stopped daemon still has rows).
/// If the daemon is running, replaces rows from `catalog index`. A failed or empty
/// daemon reply keeps the file rows. Does not call `health()` and does not block
/// when the daemon is not running.
pub(crate) fn reload_security_catalog(state: &mut AppState) {
    if !matches!(state.mode(), Mode::Security(_)) {
        return;
    }
    if let Mode::Security(s) = state.mode_mut() {
        s.seed_embedded_catalog();
        s.refresh_images();
    }
    let running = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.is_running())
        .unwrap_or(false);
    if !running {
        return;
    }
    let raw = state
        .rest
        .sec_manager
        .as_ref()
        .and_then(|m| m.catalog_op("index", &serde_json::json!({})).ok());
    let Some(raw) = raw else {
        return;
    };
    let Some(rows) = crate::security::parse_catalog_index(&raw) else {
        return;
    };
    if rows.is_empty() {
        return;
    }
    if let Mode::Security(s) = state.mode_mut() {
        s.set_catalog_index(rows);
    }
}

/// Handle the `/security` command: open the security daemon control panel.
///
/// Does NOT require an active session — the control panel is global state
/// (daemon lifecycle), not session-scoped. Opens regardless of whether the
/// daemon is installed or running (it shows the stopped state faithfully).
/// Opening a panel is always safe mid-stream (read-only view; the turn keeps
/// streaming), so there is no busy guard here.
pub(super) fn handle_security(state: &mut AppState) -> Result<()> {
    // Force a fresh container-runtime probe off the UI thread (single-flight cache).
    // `status()` below only *reads* the cache — never blocks on `docker info`.
    crate::security::runtime_status_kick(true);
    let status = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.status())
        .unwrap_or_else(|| {
            let (runtime_ok, runtime) = crate::security::runtime_status_cached();
            crate::app::sec::SecStatus {
                runtime_ok,
                runtime,
                ..Default::default()
            }
        });
    let mut st = SecurityState::new(status, state.rest.sec_inactive.clone(), state.rest.yolo_armed);
    st.refresh_images();
    st.lane_label = state
        .rest
        .fg()
        .session
        .as_ref()
        .and_then(|x| x.settings.cyber_lane)
        .map(|l| l.label().to_string())
        .unwrap_or_else(|| "none".into());
    st.agent_mode_label = state.rest.agent_mode().label().to_string();
    *state.mode_mut() = Mode::Security(Box::new(st));
    // File rows are already seeded. If the daemon is up, replace them from index.
    reload_security_catalog(state);
    Ok(())
}

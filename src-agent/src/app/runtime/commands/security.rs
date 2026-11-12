//! The `/security` command: ask the OpenTUI client to enter the secure room.

use anyhow::Result;

use crate::app::mode::Mode;
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

/// `/security` asks the OpenTUI client to hand this session to the secure room.
/// It does not open a panel. `koma legacy` drains the same flag into a status
/// line and stays in the coding chat.
pub(super) fn handle_security(state: &mut AppState) -> Result<()> {
    state.rest.fg_mut().status = "/security is not part of legacy".into();
    state.rest.secure_handoff_pending = true;
    Ok(())
}

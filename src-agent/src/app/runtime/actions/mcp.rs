//! Action handlers for the MCP dashboard: CreateMcp, SaveMcp, DeleteMcp,
//! CloseMcp.
//!
//! Unlike the agents dashboard (which writes markdown files via a data-layer
//! API), MCP servers live in the GLOBAL `config.json`. So create/save/delete here
//! mutate `state.rest.config.mcp_servers` directly and persist with
//! [`AppConfig::save`], then refresh the in-mode snapshot from the saved config.
//!
//! Live reconnect is wired in: after a successful `config.save()`,
//! [`persist_and_finish`] calls [`McpManager::reconnect`](crate::app::mcp::McpManager::reconnect),
//! which tears down the old connections and reconnects from the just-saved server
//! set in the background. No restart needed; the status line reflects the live
//! change and per-server counts refresh on subsequent renders.

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;

/// Handle `Action::CloseMcp`: discard any in-flight drafts and return to Chat.
pub(super) fn handle_close_mcp(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}

/// Handle `Action::CreateMcp`: append a new server (built from the Create drafts)
/// to `config.mcp_servers`, persist, and refresh the snapshot.
///
/// On success the dashboard returns to Browse with the new server selected; on a
/// save error the status line reports it and the editor stays open so the draft
/// isn't lost.
pub(super) fn handle_create_mcp(state: &mut AppState) -> Result<()> {
    let Mode::Mcp(m) = state.mode() else {
        return Ok(());
    };
    let entry = m.to_entry();
    let uuid = entry.uuid.clone();
    let name = entry.name.clone();

    state.rest.config.mcp_servers.push(entry);
    persist_and_finish(state, &uuid, format!("mcp server saved: {name} — connecting…"));
    Ok(())
}

/// Handle `Action::SaveMcp`: overwrite the server whose uuid matches the Edit
/// draft with the drafts' values, persist, and refresh the snapshot.
pub(super) fn handle_save_mcp(state: &mut AppState) -> Result<()> {
    let Mode::Mcp(m) = state.mode() else {
        return Ok(());
    };
    let entry = m.to_entry();
    let uuid = entry.uuid.clone();
    let name = entry.name.clone();

    // Find the live config entry by uuid and replace it. If it somehow vanished
    // (config edited under us), fall back to appending so the edit isn't lost.
    if let Some(slot) = state
        .rest
        .config
        .mcp_servers
        .iter_mut()
        .find(|s| s.uuid == uuid)
    {
        *slot = entry;
    } else {
        state.rest.config.mcp_servers.push(entry);
    }
    persist_and_finish(state, &uuid, format!("mcp server saved: {name} — connecting…"));
    Ok(())
}

/// Handle `Action::DeleteMcp`: remove the selected server from `config.mcp_servers`
/// by uuid, persist, and refresh the snapshot.
pub(super) fn handle_delete_mcp(state: &mut AppState) -> Result<()> {
    let Mode::Mcp(m) = state.mode() else {
        return Ok(());
    };
    let Some(server) = m.current() else {
        // Nothing selected (empty list): just drop back to Browse.
        if let Mode::Mcp(m) = state.mode_mut() {
            m.cancel();
        }
        return Ok(());
    };
    let uuid = server.uuid.clone();
    let name = server.name.clone();

    state.rest.config.mcp_servers.retain(|s| s.uuid != uuid);
    // After a delete there's no entry to re-select, so pass an empty uuid (the
    // snapshot refresh just clamps the cursor).
    persist_and_finish(state, "", format!("mcp server removed: {name}"));
    Ok(())
}

/// Shared tail for create/save/delete: persist the config, refresh the in-mode
/// snapshot from `config.mcp_servers`, select the entry with `select_uuid` (when
/// non-empty and present), drop back to Browse, and set the status line.
///
/// On a save FAILURE the config was still mutated in memory (it just isn't on
/// disk yet); we report the error and leave the editor open so the user can retry
/// rather than silently losing their draft.
fn persist_and_finish(state: &mut AppState, select_uuid: &str, ok_status: String) {
    match state.rest.config.save() {
        Ok(()) => {
            let servers = state.rest.config.mcp_servers.clone();
            if let Mode::Mcp(m) = state.mode_mut() {
                m.reload(&servers);
                if !select_uuid.is_empty() {
                    if let Some(i) = m.servers.iter().position(|s| s.uuid == select_uuid) {
                        m.list_sel = i;
                    }
                }
                m.cancel();
            }
            // LIVE reconnect: tear down the old MCP connections and reconnect from
            // the just-saved server set, in the background. No restart needed. With
            // no manager (MCP never initialised) or zero servers this is a no-op.
            if let Some(m) = state.rest.mcp_manager.as_ref() {
                m.reconnect(&servers);
            }
            state.rest.fg_mut().status = ok_status;
        }
        Err(e) => {
            state.rest.fg_mut().status = format!("save failed: {e}");
        }
    }
}

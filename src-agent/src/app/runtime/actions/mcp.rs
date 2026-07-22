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
pub(super) fn handle_create_mcp(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let Mode::Mcp(m) = state.mode() else {
        return Ok(());
    };
    let entry = m.to_entry();
    let uuid = entry.uuid.clone();
    let name = entry.name.clone();

    state.rest.config.mcp_servers.push(entry);
    persist_and_finish(
        state,
        handle,
        &uuid,
        format!("mcp server saved: {name} — connecting…"),
    );
    Ok(())
}

/// Handle `Action::SaveMcp`: overwrite the server whose uuid matches the Edit
/// draft with the drafts' values, persist, and refresh the snapshot.
pub(super) fn handle_save_mcp(state: &mut AppState, handle: &tokio::runtime::Handle) -> Result<()> {
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
    persist_and_finish(
        state,
        handle,
        &uuid,
        format!("mcp server saved: {name} — connecting…"),
    );
    Ok(())
}

/// Handle `Action::DeleteMcp`: remove the selected server from `config.mcp_servers`
/// by uuid, persist, and refresh the snapshot.
pub(super) fn handle_delete_mcp(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
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
    persist_and_finish(state, handle, "", format!("mcp server removed: {name}"));
    Ok(())
}

/// Construct `state.rest.mcp_manager` on demand if it is still `None`.
///
/// TRAP: the manager is deliberately left `None` at daemon boot when
/// `config.mcp_servers` is empty (see `lifecycle::run_daemon`'s MCP setup) — no
/// manager, no global MCP daemon spawned, byte-identical to a build without MCP.
/// That means the FIRST server ever added to a live, already-running daemon has
/// no manager to reconnect: both `save_and_reload_mcp` and `persist_and_finish`
/// gate their reconnect on `mcp_manager.is_some()`, so without this call the new
/// server would land in `config.json` but never advertise any `mcp__` tools until
/// the daemon is restarted. Call this BEFORE that `is_some()` check so a
/// just-saved first server gets a live manager immediately.
///
/// Mirrors the boot-path construction in `lifecycle::run_daemon` exactly: ensure
/// the singleton global MCP daemon is running, connect a PROXY to it, and fall
/// back to a LOCAL `connect_all` if either step fails (never worse than today).
/// No-op if a manager already exists, or if there are no ENABLED servers to
/// connect (so a user with zero/all-disabled servers never spawns the global
/// daemon). Never panics — a connect failure is logged and `mcp_manager` stays
/// `None`.
fn ensure_mcp_manager(state: &mut AppState, handle: &tokio::runtime::Handle) {
    if state.rest.mcp_manager.is_some() {
        return;
    }
    if !state.rest.config.mcp_servers.iter().any(|s| s.enabled) {
        return;
    }

    let proxy = crate::model::store::mcp_daemon_sock_path().and_then(|sock| {
        crate::app::runtime::manage::ensure_mcp_daemon_running()
            .and_then(|()| crate::app::mcp::McpManager::connect_proxy(handle, sock))
    });
    state.rest.mcp_manager = Some(match proxy {
        // Proxying to the shared global daemon: the dedup win.
        Ok(proxy) => proxy,
        // FALLBACK: any ensure/connect failure ⇒ own the connections locally, so
        // this daemon still has working MCP (just not shared).
        Err(e) => {
            crate::model::store::append_global_error_log(
                "mcp",
                &format!("global daemon unavailable ({e:#}); using local servers"),
            );
            crate::app::mcp::McpManager::connect_all(handle, &state.rest.config.mcp_servers)
        }
    });
}

/// Persist `config.json` and LIVE-reconnect the MCP manager from the just-saved server
/// set — the MODE-INDEPENDENT core of [`persist_and_finish`], callable by the GUI config
/// setters (the `SetMcpServer`/`DeleteMcpServer`/`EnableMcpServer` daemon handlers), which
/// own no `Mode::Mcp` to refresh. Returns the save `Result` so the caller surfaces an
/// error; the reconnect is best-effort and spawned off the event-loop thread for the same
/// reason [`persist_and_finish`] does it (a `Proxy`-backend reconnect blocks on unix-socket
/// round-trips to the MCP daemon). With no manager or zero servers the reconnect is a no-op.
pub(in crate::app::runtime) fn save_and_reload_mcp(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    state.rest.config.save()?;
    ensure_mcp_manager(state, handle);
    let servers = state.rest.config.mcp_servers.clone();
    if let Some(m) = state.rest.mcp_manager.as_ref() {
        let mgr = m.clone();
        std::thread::spawn(move || {
            mgr.reconnect(&servers);
        });
    }
    Ok(())
}

/// Shared tail for create/save/delete: persist the config, refresh the in-mode
/// snapshot from `config.mcp_servers`, select the entry with `select_uuid` (when
/// non-empty and present), drop back to Browse, and set the status line.
///
/// On a save FAILURE the config was still mutated in memory (it just isn't on
/// disk yet); we report the error and leave the editor open so the user can retry
/// rather than silently losing their draft.
fn persist_and_finish(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    select_uuid: &str,
    ok_status: String,
) {
    match state.rest.config.save() {
        Ok(()) => {
            ensure_mcp_manager(state, handle);
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
            //
            // `reconnect` itself is spawned onto a bare OS thread rather than called
            // inline: on the `Proxy` backend it does TWO blocking unix-socket round-
            // trips to the global MCP daemon (each bounded by a 65s IO timeout), and
            // this handler runs synchronously on the event-loop thread — a slow/wedged
            // daemon would otherwise freeze all input (keys, Esc double-tap timing,
            // rendering) for up to ~130s. On the `Local` backend `reconnect` is already
            // cheap (a couple of mutex-guarded ops that spawn async work on the tokio
            // handle), so running it on a throwaway thread costs only a thread spawn.
            // The eventual server/tool-count state becomes visible via the cached
            // `server_status_cached` reads (see `app/mcp/mod.rs`).
            if let Some(m) = state.rest.mcp_manager.as_ref() {
                let mgr = m.clone();
                let servers_cloned = servers.clone();
                std::thread::spawn(move || {
                    mgr.reconnect(&servers_cloned);
                });
            }
            state.rest.fg_mut().status = ok_status;
        }
        Err(e) => {
            state.rest.fg_mut().status = format!("save failed: {e}");
        }
    }
}

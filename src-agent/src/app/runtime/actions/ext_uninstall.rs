//! The MANAGER-INDEPENDENT uninstall core, extracted from
//! [`crate::app::runtime::event_loop::daemon::hub::requests_ext`]'s `uninstall_extension`
//! so BOTH the GUI store path (the daemon hub, which wraps this + emits its `DaemonEvent`s)
//! AND the TUI `/extension` path (`actions::extensions`, which calls it directly + toasts)
//! run the IDENTICAL 9-step nuke.
//!
//! Lives under `crate::app::runtime` (not `crate::app::ext`) BECAUSE the nuke calls two
//! runtime-internal helpers — [`crate::app::runtime::actions::save_and_reload_mcp`]
//! (`pub(in crate::app::runtime)`) and [`crate::app::runtime::manage::broadcast_unload_extension`]
//! — that are only visible within this module tree. The fs/registry-only halves it delegates
//! to (`snapshot_manifest` / `unload_ext_footprint` / `is_safe_ext_id` / `sweep_agent_overrides`)
//! stay in [`crate::app::ext::uninstall`], reachable from both paths.

use crate::app::state::AppState;
use crate::model::store;

/// Run the COMPLETE uninstall nuke for extension `id` on THIS daemon — the audited 9-step
/// sequence, in order, WITHOUT the hub (so the TUI can call it directly): snapshot the
/// manifest before it's gone (1); unload THIS daemon's live footprint (2/4 + in-memory
/// clears); fan the same unload out to every OTHER live session-daemon (3); remove the
/// on-disk package dir (6); purge the extension's catalogue contributions + deregister
/// orphan MCP-server rows + drop the registry entry, then ONE save + a live MCP reconnect
/// (5/8); sweep same-named agent overrides (7); nuke the declared workspace_dir (9); and
/// refresh the dir cache + system prompt (10). Best-effort throughout (every failure is
/// logged, never fatal — matching the pre-extraction hub behaviour), so it currently always
/// returns `Ok(())`; the `Result` is the stable contract the callers surface (the TUI toasts
/// an `Err`, the hub would map it to `ok:false`).
pub(crate) fn uninstall_extension_core(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    id: &str,
) -> Result<(), String> {
    // (1) Snapshot the manifest ONCE — its sub-agent names + workspace_dir — BEFORE the
    // dir is deleted in step 6 (after which the manifest is unreadable).
    let snap = crate::app::ext::uninstall::snapshot_manifest(id);

    // (2/4 + footprint) Unload THIS daemon's live in-memory footprint: stop the child,
    // purge its contributed MCP tools, drop its context blob / buffered prompts /
    // ext-agent registry. Shared with the fan-out `unload_extension` handler.
    crate::app::ext::uninstall::unload_ext_footprint(state, id);

    // (3) Fan the same in-memory unload out to every OTHER live session-daemon, so none
    // keeps serving a stale copy until its next boot. OFF the event loop (a bare OS
    // thread): the blocking socket sweep — and the harmless self-connect it includes —
    // must never wedge the loop. Best-effort; failures are logged inside, never fatal.
    {
        let ext_id = id.to_string();
        std::thread::spawn(move || {
            crate::app::runtime::manage::broadcast_unload_extension(&ext_id);
        });
    }

    // (6) Remove the unpacked package dir. Guard the id against a path-escape before
    // joining (defense in depth — the id comes from the client): only a well-formed
    // reverse-DNS id is a real installed dir name, and anything else can't match a
    // registry entry.
    if crate::app::ext::uninstall::is_safe_ext_id(id) {
        if let Ok(dir) = store::extensions_dir() {
            let target = dir.join(id);
            if let Err(e) = std::fs::remove_dir_all(&target) {
                // A missing dir (already gone) is fine; log anything else.
                if e.kind() != std::io::ErrorKind::NotFound {
                    store::append_global_error_log(
                        "ext-uninstall",
                        &format!("remove {}: {e}", target.display()),
                    );
                }
            }
        }
    } else {
        store::append_global_error_log(
            "ext-uninstall",
            &format!("refusing to remove dir for unsafe extension id {id:?}"),
        );
    }

    // (5a) Config mutations. W12b: PURGE the extension's CATALOGUE contributions (its
    // key-backed providers, the models served by them or by its oauth conns, the conns,
    // its preferred-model record). Then DEREGISTER orphan MCP-server rows (ext-owned, or
    // whose command lives under extensions/<id>/ — a bundled MCP binary now deleted). Then
    // DROP the registry entry. `main_reset` flags that a purged model held the GLOBAL Main
    // role (resolution self-heals to koma-free; we toast the reset).
    let purge = state.rest.config.purge_extension(id);
    let _mcp_rows_removed = state.rest.config.remove_ext_mcp_servers(id);
    state.rest.config.remove_extension_by_id(id);

    // (8 + 5b) The SINGLE save covering all three mutations above, PLUS the live MCP
    // reconnect from the just-saved server set (which drops any removed orphan row's live
    // connection). `save_and_reload_mcp` IS the one save on this path.
    if let Err(e) = crate::app::runtime::actions::save_and_reload_mcp(state, handle) {
        store::append_global_error_log(
            "ext-uninstall",
            &format!("save/reload config after uninstall {id}: {e:#}"),
        );
    }

    // (7) Sweep same-named agent-override files (global + every session) left by a user
    // who saved an edited copy of one of this extension's sub-agents. The same-name caveat
    // is documented on the helper.
    crate::app::ext::uninstall::sweep_agent_overrides(&snap.sub_agent_names);

    // (9) Nuke the extension's declared workspace_dir (validated against the SAME policy
    // as install; a missing/rejected dir is skipped). User-approved data deletion — the
    // confirm named this dir before the request was ever sent.
    if let Some(ws) = snap.workspace_dir.as_deref() {
        crate::model::ext_workspace::remove_workspace_dir(ws);
    }

    // Surface a purged Main-role assignment as a foreground toast (delivered via the
    // snapshot diff) — mirrors how a dangling Main provider is otherwise reported.
    if purge.main_reset {
        state
            .rest
            .fg_mut()
            .set_toast_info(format!("main model reset: extension {id} uninstalled"));
    }

    // (10) A workspace root may now point at a deleted dir; refresh the dir cache + the
    // system prompt so the "# Extension workspaces" note drops the uninstalled extension
    // (it is no longer in `installed_extensions`, so `rebuild_system` excludes it). The
    // stale root string self-heals on the next boot's re-derive. Mirrors the install tail.
    if state.rest.fg().session.is_some() {
        if let Some(roots) = state.rest.fg().session.as_ref().map(|s| s.workdirs()) {
            crate::tool::dircache::reindex(roots, state.rest.fg().dir_cache.clone());
        }
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            sess.rebuild_system();
        }
    }

    Ok(())
}

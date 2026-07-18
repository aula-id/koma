//! The MANAGER-INDEPENDENT install core, extracted from
//! [`crate::app::runtime::event_loop::daemon::hub::requests_ext`]'s `finish_install` so
//! BOTH the GUI store path (the daemon hub, which wraps this + emits its `DaemonEvent`s)
//! AND the TUI `/store` path (`event_loop::global::drains::drain_store`, which calls this
//! directly once a `StoreEvent::InstallArtifact` lands) run the IDENTICAL on-loop
//! verify+install tail — a TUI-installed extension is byte-identical to a GUI-installed
//! one.
//!
//! Lives under `crate::app::runtime` (not `crate::app::ext`) for the SAME reason
//! [`super::ext_uninstall::uninstall_extension_core`] does: the tail calls
//! [`super::save_and_reload_mcp`] (`pub(in crate::app::runtime)`), only visible within
//! this module tree. The fs/verify halves it delegates to (`install::install_from_zip` /
//! `install::install_dev_unsigned`, `register::register_mcp_servers` /
//! `register::register_contributions`) stay in [`crate::app::ext`], reachable from both
//! paths.

use anyhow::Result;

use crate::app::state::AppState;
use crate::model::app_config::InstalledExtension;
use crate::model::store;

/// Verify + unpack a downloaded extension zip, upsert + persist the registry, register
/// its `contributes` + any manifest-declared MCP servers, auto-start it if daemon-kind,
/// and widen the active session's workspace roots — the ON-LOOP tail of an install, run
/// AFTER the network download (`ext::ext_store::kick_off_store_install` / the daemon hub's
/// own fetch) lands. `id` is the id the artifact was REQUESTED for (used only for the
/// debug-unsigned-fallback log line — the manifest's OWN id, read back out of the zip by
/// `install_from_zip`, is what's actually installed/returned).
///
/// Mirrors the pre-extraction `DaemonHub::finish_install`'s `Ok`/`Err` branches
/// verbatim, minus the hub's `send_to`/`send_installed_extensions` replies — the caller
/// surfaces the result its own way (a `DaemonEvent` for the hub, in-mode fields for the
/// TUI `/store` drain). Best-effort throughout the registration/start/workspace steps
/// (every failure there is logged, never fails the overall install — matching the
/// pre-extraction hub behaviour); only the verify/unpack step itself can fail the whole
/// call.
pub(in crate::app::runtime) fn install_extension_core(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    id: &str,
    zip: &[u8],
    sha256: &str,
    signature: Option<&str>,
) -> Result<InstalledExtension, String> {
    // Clone the manager Arcs so the later `&mut` config mutations don't overlap a
    // `state.rest` borrow.
    let mcp = state.rest.mcp_manager.clone();
    let ext_mgr = state.rest.ext_manager.clone();

    let installed: Result<InstalledExtension> = match (signature, sha256.trim().is_empty()) {
        // Signed + integrity present → the production fail-closed path.
        (Some(sig), false) => crate::app::ext::install::install_from_zip(zip, sha256, sig),
        // No signature (or no advertised digest): koma.run signing not live yet.
        _ => install_unsigned_fallback(id, zip),
    };

    let ext = installed.map_err(|e| {
        store::append_global_error_log(
            "ext install",
            &format!("verify/unpack failed for extension {id}: {e:#}"),
        );
        format!("{e:#}")
    })?;

    state.rest.config.upsert_extension(ext.clone());
    // Auto-register any manifest-declared bundled MCP servers (e.g. a standalone
    // stdio server shipped alongside the extension's own daemon) BEFORE the single
    // save+reload below, so a fresh install never needs the user to hand-add an
    // McpServerEntry — see `register::register_mcp_servers`.
    if let Err(e) = crate::app::ext::register::register_mcp_servers(&ext, &mut state.rest.config) {
        store::append_global_error_log(
            "ext-install",
            &format!("register mcp servers for {}: {e:#}", ext.id),
        );
    }
    // ONE save covering both the registry upsert and any registered MCP-server rows,
    // plus a live MCP reconnect from the just-saved server set — mirrors
    // `uninstall_extension_core`'s single `save_and_reload_mcp` call.
    if let Err(e) = super::save_and_reload_mcp(state, handle) {
        store::append_global_error_log(
            "ext-install",
            &format!("save config after install {}: {e:#}", ext.id),
        );
    }
    // Register contributions (tools → live MCP snapshot) + auto-start a daemon-kind
    // child. Both best-effort: a failure is logged, not fatal — the extension is
    // installed on disk + in the registry regardless.
    if let Some(mgr) = &ext_mgr {
        if let Err(e) = crate::app::ext::register::register_contributions(&ext, mcp.as_ref(), mgr) {
            store::append_global_error_log(
                "ext-install",
                &format!("register contributions for {}: {e:#}", ext.id),
            );
        }
        if ext.kind == "daemon" {
            if let Err(e) = mgr.ensure_started(&ext) {
                store::append_global_error_log(
                    "ext-install",
                    &format!("start extension {}: {e:#}", ext.id),
                );
            }
        }
    }
    // Widen the ACTIVE session's workspace roots so writes into this extension's
    // declared `workspace_dir` pass the harness WITHOUT a daemon restart. When a root
    // is added, reindex the dir cache (`@`/dir_list pick it up) and rebuild the system
    // prompt so its "# Extension workspaces" note names the new root immediately.
    {
        let installed_list = state.rest.config.installed_extensions.clone();
        let added = match state.rest.fg_mut().session.as_mut() {
            Some(sess) => crate::model::ext_workspace::inject_extension_workspaces(
                &installed_list,
                &mut sess.settings.workdir,
            ),
            None => Vec::new(),
        };
        if !added.is_empty() {
            if let Some(roots) = state.rest.fg().session.as_ref().map(|s| s.workdirs()) {
                crate::tool::dircache::reindex(roots, state.rest.fg().dir_cache.clone());
            }
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
            }
        }
    }

    Ok(ext)
}

/// The DEBUG-only unsigned install fallback: write the zip to a temp file and install it
/// via [`crate::app::ext::install::install_dev_unsigned`] (which skips signature
/// verification), so the end-to-end store→install flow is testable before koma.run's
/// signing infra is live. LOUDLY logged. A release build has no such path — an unsigned
/// artifact is rejected.
#[cfg(debug_assertions)]
fn install_unsigned_fallback(id: &str, zip: &[u8]) -> Result<InstalledExtension> {
    store::append_global_error_log(
        "ext-install",
        &format!("UNSIGNED dev install of {id} (koma.run sent no signature — debug build only)"),
    );
    let tmp = std::env::temp_dir().join(format!("koma-ext-dl-{}.zip", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, zip)
        .map_err(|e| anyhow::anyhow!("write temp zip {}: {e}", tmp.display()))?;
    let r = crate::app::ext::install::install_dev_unsigned(&tmp);
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Release builds reject an unsigned artifact — the signature gate can never be
/// bypassed in production (see `install::install_dev_unsigned`'s `cfg(debug_assertions)`).
#[cfg(not(debug_assertions))]
fn install_unsigned_fallback(_id: &str, _zip: &[u8]) -> Result<InstalledExtension> {
    anyhow::bail!("extension artifact is unsigned; refusing to install")
}

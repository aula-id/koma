//! Registration hook: wires an installed extension's `contributes.tools` /
//! `contributes.sub_agents` into the rest of koma once its daemon is started.
//!
//! This lives OUTSIDE [`super::ExtHostManager`] on purpose. The manager only
//! knows how to spawn/handshake/invoke a single extension process — it has no
//! visibility into `AppState`'s [`crate::app::mcp::McpManager`] or
//! [`crate::model::agent_def::AgentRegistry`] (see the module docs on
//! [`super::ExtHostManager`] and its `#![allow(dead_code)]` note: the tool-system
//! wiring is a follow-up wave, built at the RUNTIME layer, not inside the
//! manager). So [`register_contributions`]/[`purge_contributions`] are free
//! functions taking whatever app-state handles each contribution kind needs,
//! called from `app::runtime::lifecycle::build_startup` (today, right after
//! `ExtHostManager::ensure_started` succeeds) and from any future install/
//! enable/disable/uninstall command handler.
//!
//! ## The two contribution kinds handled here
//!
//! - **`contributes.tools`** — pushed into the LIVE [`McpManager`] snapshot via
//!   [`McpManager::register_extension_tools`]/[`McpManager::purge_extension_tools`],
//!   because that snapshot is cached in memory (not re-read from disk per
//!   request) — an explicit push/pop is required on every start/stop.
//! - **`contributes.sub_agents`** — needs NO explicit action here.
//!   [`crate::model::agent_def::registry::merge_extension_sub_agents`] reads
//!   `AppConfig::installed_extensions` + each entry's on-disk `manifest.json`
//!   FRESH on every [`crate::model::agent_def::AgentRegistry::load`] call, so an
//!   enabled extension's sub-agents simply appear on the very next load, and
//!   disappear the moment its config entry is removed or disabled — no cache to
//!   invalidate, no reload trigger to wire up.
//!
//! `contributes.models` / `contributes.panels` are OTHER waves (model-catalogue
//! wiring / panel UI) — untouched here.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use koma_extension::protocol::ExtensionManifest;

use crate::app::mcp::McpManager;
use crate::model::app_config::{AppConfig, InstalledExtension, McpServerEntry, McpTransport};
use crate::model::store;

use super::{install, ExtHostManager};

/// Read `<extensions_dir>/<ext.id>/manifest.json` and register `ext`'s
/// `contributes.tools` into `mcp_manager` (if one exists at this call site — see
/// below). Best-effort: a missing/unparsable manifest is returned as `Err` (the
/// caller logs it) rather than panicking; a manifest with no `contributes.tools`
/// is simply a no-op registration.
///
/// `mcp_manager` is `Option` because it can legitimately be absent at the boot
/// call site: in `--daemon` mode, `build_startup` builds `ext_manager` and runs
/// this loop BEFORE `run_daemon` builds the (possibly `Proxy`) `McpManager` for
/// that session. When absent, extension tools are simply not advertised for
/// that process — routing them through the global MCP daemon's `Proxy` wire
/// protocol is a later wave (see [`McpManager::register_extension_tools`]'s
/// docs). `contributes.sub_agents` needs no action here at all — see the module
/// docs. `contributes.models`/`contributes.panels`: wave B-models / wave D.
pub fn register_contributions(
    ext: &InstalledExtension,
    mcp_manager: Option<&Arc<McpManager>>,
    ext_manager: &Arc<ExtHostManager>,
) -> Result<()> {
    let manifest = read_manifest(&ext.id)?;

    if !manifest.contributes.tools.is_empty() {
        if let Some(mgr) = mcp_manager {
            mgr.register_extension_tools(
                &ext.id,
                &manifest.contributes.tools,
                Arc::clone(ext_manager),
            );
        }
    }

    Ok(())
}

/// Undo [`register_contributions`]'s tool registration for `ext_id` (called on
/// uninstall or disable). `contributes.sub_agents` needs no undo step either —
/// removing/disabling the config entry (the caller's job, via
/// `AppConfig::remove_extension_by_id` or flipping `enabled` + `save()`) is
/// enough; the next `AgentRegistry::load` simply won't find it. A no-op if
/// `mcp_manager` is absent (mirrors [`register_contributions`]).
pub fn purge_contributions(ext_id: &str, mcp_manager: Option<&Arc<McpManager>>) {
    if let Some(mgr) = mcp_manager {
        mgr.purge_extension_tools(ext_id);
    }
}

/// Register `ext`'s manifest-declared [`ManifestMcpServer`](koma_extension::protocol::ManifestMcpServer)
/// entries into the global MCP catalogue (`AppConfig::mcp_servers`) — the fix for the "fresh
/// install shows 'No MCP servers'" gap: a bundled stdio MCP binary (e.g. the Workflow
/// extension's `bin/workflow-mcp`) previously needed the user to hand-add an `McpServerEntry`
/// through the MCP settings before it was ever usable.
///
/// **UPSERT, keyed on `(ext_id, name)`.** A row THIS extension previously registered under
/// the SAME declared name is replaced in place — its `uuid` and `enabled` flag are
/// preserved (a user who disabled it keeps it disabled across an upgrade), only `command`/
/// `args`/`env`-carrying fields move; a name this extension declared before but no longer
/// does (a version that stopped shipping a server) is dropped. A row with no provenance
/// (`ext_id: None`, user-created) or belonging to a DIFFERENT extension is NEVER touched —
/// see [`resolve_server_name`] for the collision-avoidance this guarantees on the shared
/// `mcp__<name>__<tool>` advertise namespace.
///
/// `command` resolves to an ABSOLUTE path under `extensions/<id>/` via [`std::path::Path::join`]
/// (platform-native separators — no hardcoded `/`, so this is Windows-safe), through the
/// SAME containment guard [`install::safe_exec_rel`] applies to `runtime.exec`: an `exec`
/// that escapes the install dir, or doesn't exist on disk after unpack, fails the WHOLE
/// registration (fail-closed, matching `unpack`'s own `runtime.exec` check) rather than
/// silently registering a broken entry.
///
/// Returns the count registered. Pure mutation of `config` — the caller persists (`save()`
/// or `save_and_reload_mcp`) and triggers a live MCP reload afterwards, exactly like every
/// other `AppConfig` setter in this codebase.
pub fn register_mcp_servers(ext: &InstalledExtension, config: &mut AppConfig) -> Result<usize> {
    let manifest = read_manifest(&ext.id)?;

    if manifest.mcp_servers.is_empty() {
        // Nothing declared THIS call — still drop any stale rows a PRIOR version of this
        // extension registered, so an upgrade that removes a bundled server doesn't leave
        // a dead orphan behind (mirrors what a full uninstall would do to them).
        config
            .mcp_servers
            .retain(|s| s.ext_id.as_deref() != Some(ext.id.as_str()));
        return Ok(0);
    }

    let ext_dir = store::extensions_dir()?.join(&ext.id);

    // Resolve + validate every declared server BEFORE mutating `config` at all — fail-closed:
    // one bad entry fails the WHOLE registration rather than leaving a partial/broken set.
    let mut resolved: Vec<(String, std::path::PathBuf, Vec<String>)> =
        Vec::with_capacity(manifest.mcp_servers.len());
    for decl in &manifest.mcp_servers {
        let exec_path = install::safe_exec_rel(&decl.exec, &ext_dir).with_context(|| {
            format!(
                "extension {} declares mcp_servers[name={}].exec {:?}",
                ext.id, decl.name, decl.exec
            )
        })?;
        if !exec_path.is_file() {
            bail!(
                "extension {} declares mcp_servers[name={}].exec {:?} but it was not found \
                 under {} after unpack",
                ext.id,
                decl.name,
                decl.exec,
                ext_dir.display()
            );
        }
        let name = resolve_server_name(config, &ext.id, &decl.name);
        resolved.push((name, exec_path, decl.args.clone()));
    }

    // Drop this extension's stale rows (declared by a PRIOR version but not this one) before
    // upserting the current set. Never touches a row owned by a different extension or a
    // user-created one — the retain predicate only ever matches THIS extension's own rows.
    let keep: HashSet<&str> = resolved.iter().map(|(name, _, _)| name.as_str()).collect();
    config
        .mcp_servers
        .retain(|s| s.ext_id.as_deref() != Some(ext.id.as_str()) || keep.contains(s.name.as_str()));

    for (name, exec_path, args) in &resolved {
        // Find THIS extension's existing row under the resolved name (if any) so the upsert
        // preserves its `uuid`/`enabled`/`env` rather than resetting them on every reinstall.
        let existing = config
            .mcp_servers
            .iter()
            .find(|s| s.ext_id.as_deref() == Some(ext.id.as_str()) && &s.name == name)
            .cloned();
        let entry = McpServerEntry {
            uuid: existing
                .as_ref()
                .map(|e| e.uuid.clone())
                .unwrap_or_default(),
            name: name.clone(),
            enabled: existing.as_ref().map(|e| e.enabled).unwrap_or(true),
            transport: McpTransport::Stdio,
            command: exec_path.to_string_lossy().into_owned(),
            args: args.clone(),
            env: existing.map(|e| e.env).unwrap_or_default(),
            url: String::new(),
            ext_id: Some(ext.id.clone()),
        };
        // `upsert_mcp_server` mints a fresh uuid only when `entry.uuid` arrives empty (a
        // brand-new row); a preserved uuid above replaces the existing row in place.
        config.upsert_mcp_server(entry);
    }

    Ok(resolved.len())
}

/// Resolve the [`McpServerEntry::name`] a manifest-declared server registers under:
/// `name` itself, unless a row already exists under that EXACT name and belongs to
/// someone else — a different extension, or a user-created row (`ext_id: None`) — in
/// which case this extension's own id is prefixed (`"<ext_id>:<name>"`) so the two can
/// never collide on the shared `mcp__<name>__<tool>` advertise namespace (see
/// `app::mcp::util::sanitize_server_name`). A row THIS extension already owns under
/// `name` is not a collision — it's the exact upsert target `register_mcp_servers`
/// replaces in place.
fn resolve_server_name(config: &AppConfig, ext_id: &str, name: &str) -> String {
    let collides = config
        .mcp_servers
        .iter()
        .any(|s| s.name == name && s.ext_id.as_deref() != Some(ext_id));
    if collides {
        format!("{ext_id}:{name}")
    } else {
        name.to_string()
    }
}

/// Read + parse `<extensions_dir>/<id>/manifest.json`.
fn read_manifest(id: &str) -> Result<ExtensionManifest> {
    let path = store::extensions_dir()?.join(id).join("manifest.json");
    let bytes =
        std::fs::read(&path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))
}

#[cfg(test)]
#[path = "register_test.rs"]
mod tests;

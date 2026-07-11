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

use std::sync::Arc;

use anyhow::Result;

use koma_extension::protocol::ExtensionManifest;

use crate::app::mcp::McpManager;
use crate::model::app_config::InstalledExtension;
use crate::model::store;

use super::ExtHostManager;

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
            mgr.register_extension_tools(&ext.id, &manifest.contributes.tools, Arc::clone(ext_manager));
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

/// Read + parse `<extensions_dir>/<id>/manifest.json`.
fn read_manifest(id: &str) -> Result<ExtensionManifest> {
    let path = store::extensions_dir()?.join(id).join("manifest.json");
    let bytes = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing manifest.json (nothing installed at that path) is a clean `Err`,
    /// not a panic — `register_contributions` propagates it so the caller logs
    /// and moves on (mirrors the boot loop's existing `ensure_started` failure
    /// handling in `build_startup`).
    #[test]
    fn register_contributions_errors_on_missing_manifest() {
        let ext = InstalledExtension {
            id: format!("run.koma.example.does-not-exist-{}", uuid::Uuid::new_v4()),
            version: "0.0.1".to_string(),
            tier: "free".to_string(),
            granted: vec![],
            enabled: true,
            kind: "daemon".to_string(),
            exec: "bin/tool".to_string(),
        };
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let ext_manager = ExtHostManager::new(rt.handle());
        let result = register_contributions(&ext, None, &ext_manager);
        assert!(result.is_err(), "a missing manifest.json must be a clean Err");
    }
}

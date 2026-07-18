//! The `/extension` command: open the installed-extension manager, plus the shared
//! [`build_extensions_state`] row builder the TUI action handlers + the ext-screen pop-back
//! drain reuse (so a Browse list, a post-uninstall rebuild, and an ExtScreen→Detail return
//! all derive rows the same way).

use anyhow::Result;

use crate::app::mode::{ExtRow, ExtSubMode, ExtTuiScreen, ExtensionsState, Mode};
use crate::app::state::{AppState, AppStateRest};
use crate::model::store;

/// Handle the `/extension` command: open the installed-extension dashboard in Browse.
///
/// Does NOT require an active session — the registry is global (config + on-disk
/// manifests). Opening a read-only panel is always safe mid-stream.
pub(super) fn handle_extensions(state: &mut AppState) -> Result<()> {
    let st = build_extensions_state(&state.rest, ExtSubMode::Browse, None);
    *state.mode_mut() = Mode::Extensions(Box::new(st));
    Ok(())
}

/// Build the `/extension` dashboard state: one [`ExtRow`] per `config.installed_extensions`
/// entry, enriched with the on-disk manifest (name / description / contribution counts /
/// panels-count / tui_screens / workspace_dir) and the LIVE running status
/// ([`crate::app::ext::ExtHostManager::is_running`]). `select_id` pre-selects that row when
/// present (else row 0); `sub_mode` sets the starting sub-mode (Browse for `/extension`,
/// Detail for the ExtScreen pop-back). Reachable crate-wide (`pub(crate)`) so
/// `actions::extensions` (rebuild after uninstall + ExtScreen close) and
/// `drains::drain_ext_screen` (ext-driven `{close:true}`) reuse the exact same derivation.
pub(crate) fn build_extensions_state(
    rest: &AppStateRest,
    sub_mode: ExtSubMode,
    select_id: Option<&str>,
) -> ExtensionsState {
    let rows: Vec<ExtRow> = rest
        .config
        .installed_extensions
        .iter()
        .map(|e| {
            let info = read_ext_manifest(&e.id);
            let running = rest
                .ext_manager
                .as_ref()
                .map(|m| m.is_running(&e.id))
                .unwrap_or(false);
            ExtRow {
                id: e.id.clone(),
                name: if info.name.is_empty() {
                    e.id.clone()
                } else {
                    info.name
                },
                version: e.version.clone(),
                tier: e.tier.clone(),
                kind: e.kind.clone(),
                enabled: e.enabled,
                running,
                description: info.description,
                granted: e.granted.clone(),
                tools: info.tools,
                panels: info.panels,
                sub_agents: info.sub_agents,
                models: info.models,
                tui_screens: info.tui_screens,
                workspace_dir: info.workspace_dir,
            }
        })
        .collect();

    let list_sel = select_id
        .and_then(|id| rows.iter().position(|r| r.id == id))
        .unwrap_or(0);

    ExtensionsState {
        rows,
        list_sel,
        sub_mode,
        screen_sel: 0,
        error: None,
    }
}

/// The manifest-sourced half of one [`ExtRow`], read best-effort off
/// `extensions/<id>/manifest.json`. The registry (`InstalledExtension`) carries no
/// contributions/description/screens, so this is a fresh re-read on every build.
struct ExtManifestInfo {
    name: String,
    description: String,
    tools: usize,
    panels: usize,
    sub_agents: usize,
    models: usize,
    tui_screens: Vec<ExtTuiScreen>,
    workspace_dir: Option<String>,
}

impl Default for ExtManifestInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            tools: 0,
            panels: 0,
            sub_agents: 0,
            models: 0,
            tui_screens: Vec::new(),
            workspace_dir: None,
        }
    }
}

/// Read `extensions/<id>/manifest.json` and project the render-facing bits. A
/// missing/unreadable/unparsable manifest degrades to defaults (name empty → caller falls
/// back to the id; zero counts; no screens) — never fails the whole list over one bad entry.
/// A parse failure is logged (visible), a missing file is a silent no-op, mirroring
/// `requests_ext::read_ext_manifest_info`.
fn read_ext_manifest(id: &str) -> ExtManifestInfo {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return ExtManifestInfo::default(),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return ExtManifestInfo::default(),
    };
    let manifest: koma_extension::protocol::ExtensionManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            return ExtManifestInfo::default();
        }
    };
    let c = &manifest.contributes;
    ExtManifestInfo {
        name: manifest.name,
        description: manifest.description,
        tools: c.tools.len(),
        panels: c.panels.len(),
        sub_agents: c.sub_agents.len(),
        models: c.models.len(),
        tui_screens: c
            .tui_screens
            .iter()
            .map(|s| ExtTuiScreen {
                id: s.id.clone(),
                title: s.title.clone(),
            })
            .collect(),
        workspace_dir: manifest
            .workspace_dir
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}

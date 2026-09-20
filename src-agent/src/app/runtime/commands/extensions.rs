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
pub(super) fn handle_extensions(
    args: &str,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let args = args.trim();
    let mode = if args.eq_ignore_ascii_case("use") {
        ExtSubMode::UsePicker
    } else {
        ExtSubMode::Browse
    };
    if !args.is_empty() && mode != ExtSubMode::UsePicker {
        state
            .rest
            .fg_mut()
            .set_toast("usage: /extension or /extension use".into());
        return Ok(());
    }
    if mode == ExtSubMode::UsePicker && state.rest.fg().session.is_none() {
        state
            .rest
            .fg_mut()
            .set_toast("Open a session before selecting an extension.".into());
        return Ok(());
    }
    // Pick up activation policy changes made by another session daemon.
    if !state.rest.sessions.iter().any(|s| s.is_working()) {
        state.rest.config.installed_extensions =
            crate::model::app_config::AppConfig::load().installed_extensions;
        for idx in 0..state.rest.sessions.len() {
            if let Err(error) = refresh_session(state, idx, handle) {
                state.rest.fg_mut().set_toast(error.to_string());
            }
        }
    }
    let mut st = build_extensions_state(&state.rest, mode, None);
    if mode == ExtSubMode::UsePicker {
        st.rows.retain(|e| e.enabled);
    }
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
                activation: e.activation,
                active: e.active_in(
                    rest.fg()
                        .session
                        .as_ref()
                        .map(|s| s.settings.active_extensions.as_slice())
                        .unwrap_or_default(),
                ),
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
#[derive(Default)]
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

/// Apply a validated selection batch for either the TUI picker or headless CLI.
/// Workspace, tools, and model-facing context follow the session selection together.
pub(crate) fn set_session_extensions(
    state: &mut AppState,
    idx: usize,
    handle: &tokio::runtime::Handle,
    load: &[String],
    unload: &[String],
) -> Result<()> {
    anyhow::ensure!(
        !state.rest.sessions[idx].is_working(),
        "Wait for the current turn to finish before changing extensions."
    );
    anyhow::ensure!(
        !load.iter().any(|id| unload.contains(id)),
        "An extension cannot be loaded and unloaded together."
    );
    // Validate the complete batch before changing selection or creating roots.
    for id in load.iter().chain(unload) {
        let ext = state
            .rest
            .config
            .installed_extensions
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| anyhow::anyhow!("Extension '{id}' is not installed."))?;
        if load.contains(id) {
            anyhow::ensure!(
                ext.enabled,
                "Extension '{id}' is disabled; enable it in /extension first."
            );
        } else {
            anyhow::ensure!(
                ext.activation != crate::model::app_config::ExtensionActivation::Global,
                "Extension '{id}' is global. Set it to on-demand in /extension first."
            );
        }
    }
    for id in load {
        if let Some(raw) = crate::model::ext_workspace::read_workspace_dir(id) {
            crate::model::ext_workspace::validate_workspace_dir(&raw)?;
        }
    }
    let sess = state.rest.sessions[idx]
        .session
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("No active session."))?;
    let before = sess.settings.clone();
    sess.settings
        .active_extensions
        .retain(|id| !unload.contains(id));
    for id in load {
        if !sess.settings.active_extensions.contains(id) {
            sess.settings.active_extensions.push(id.clone());
        }
    }
    if let Err(error) = refresh_session(state, idx, handle) {
        if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
            sess.settings = before;
        }
        let _ = refresh_session(state, idx, handle);
        return Err(error);
    }
    Ok(())
}

/// Reconcile one session after activation policy changes. Workspace provenance
/// is saved with selection so resume cannot mistake extension roots for user roots.
pub(crate) fn refresh_session_if_needed(
    state: &mut AppState,
    idx: usize,
    handle: &tokio::runtime::Handle,
) -> Result<bool> {
    let rt = &state.rest.sessions[idx];
    let Some(sess) = &rt.session else {
        return Ok(false);
    };
    if rt
        .extension_scope
        .as_ref()
        .is_some_and(|(installed, selected)| {
            installed == &state.rest.config.installed_extensions
                && selected == &sess.settings.active_extensions
        })
    {
        return Ok(false);
    }
    refresh_session(state, idx, handle)?;
    Ok(true)
}

pub(crate) fn refresh_session(
    state: &mut AppState,
    idx: usize,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let selected = state.rest.sessions[idx]
        .session
        .as_ref()
        .map(|s| s.settings.active_extensions.as_slice())
        .unwrap_or_default();
    if state.rest.mcp_manager.is_none()
        && state.rest.ext_manager.is_some()
        && state
            .rest
            .config
            .installed_extensions
            .iter()
            .any(|e| e.active_in(selected))
    {
        // No configured MCP servers still needs an inert manager for extension tools.
        state.rest.mcp_manager = Some(crate::app::mcp::McpManager::connect_all(handle, &[]));
    }
    let config = &state.rest.config;
    let rt = &mut state.rest.sessions[idx];
    let Some(sess) = rt.session.as_mut() else {
        return Ok(());
    };
    let before = sess.workdirs();
    let registry = crate::model::agent_def::AgentRegistry::load_for_session(
        Some(&sess.path),
        config,
        &sess.settings.active_extensions,
    );
    sess.rebuild_system_with(&registry, config);
    sess.save()?;
    if before != sess.workdirs() {
        // Old background indexers own the old Arc; they cannot republish paths
        // from an unloaded extension into the new session index.
        rt.dir_cache =
            std::sync::Arc::new(std::sync::RwLock::new(crate::tool::DirCache::default()));
        crate::tool::dircache::reindex(sess.workdirs(), rt.dir_cache.clone());
    }
    rt.extension_scope = Some((
        config.installed_extensions.clone(),
        sess.settings.active_extensions.clone(),
    ));
    rt.pending_ext_prompts.retain(|(id, _)| {
        config
            .installed_extensions
            .iter()
            .any(|e| &e.id == id && e.active_in(&sess.settings.active_extensions))
    });
    if let Some(mgr) = &state.rest.ext_manager {
        for ext in config
            .installed_extensions
            .iter()
            .filter(|e| e.active_in(&sess.settings.active_extensions))
        {
            if let Err(error) = crate::app::ext::register::register_contributions(
                ext,
                state.rest.mcp_manager.as_ref(),
                mgr,
            ) {
                store::append_global_error_log(
                    "extension activation",
                    &format!("{}: {error:#}", ext.id),
                );
            }
            if ext.kind == "daemon" && !mgr.is_running(&ext.id) {
                let mgr = mgr.clone();
                let ext = ext.clone();
                handle.spawn_blocking(move || {
                    if let Err(error) = mgr.ensure_started(&ext) {
                        store::append_global_error_log(
                            "extension activation",
                            &format!("{}: {error:#}", ext.id),
                        );
                    }
                });
            }
        }
    }
    Ok(())
}

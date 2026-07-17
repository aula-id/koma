//! [`ExtensionsState`] — the working state for the in-app `/extension` dashboard.
//!
//! Unlike `/mcp` (which owns a mutable draft/editor), this dashboard is READ-ONLY plus a
//! single destructive action (uninstall): each row is a snapshot of one installed extension,
//! enriched at BUILD time from `config.installed_extensions` + the on-disk `manifest.json` +
//! the live `ExtHostManager` running status (see
//! `crate::app::runtime::commands::extensions::build_extensions_state`). Navigation lives
//! here; the manifest read + running probe live in the command layer (which owns `AppStateRest`).

use super::types::ExtSubMode;

/// One TUI screen an extension declares (`contributes.tui_screens[]`), projected into the
/// detail view as a selectable row. Mirrors the SDK `TuiScreenDef`.
#[derive(Debug, Clone)]
pub struct ExtTuiScreen {
    /// Stable screen id (the `panelId` on every `panel.msg` invoke for this screen).
    pub id: String,
    /// Human-facing row label.
    pub title: String,
}

/// One installed-extension row — a snapshot enriched from the registry entry, the on-disk
/// manifest, and the live running status. Pure render data; never mutated in place (the
/// whole list is rebuilt after an uninstall).
#[derive(Debug, Clone)]
pub struct ExtRow {
    /// Reverse-DNS manifest id (also the `extensions/<id>/` dir + every registry op key).
    pub id: String,
    /// Friendly manifest name (falls back to `id` when the manifest is missing/blank).
    pub name: String,
    /// Manifest version string.
    pub version: String,
    /// Tier wire string: `"free"` | `"paid"`.
    pub tier: String,
    /// Kind wire string: `"daemon"` | `"oneshot"`.
    pub kind: String,
    /// Whether the extension auto-starts at boot (the registry `enabled` flag).
    pub enabled: bool,
    /// Whether a live child is currently running (from `ExtHostManager::is_running` at build).
    pub running: bool,
    /// Manifest description (empty when absent).
    pub description: String,
    /// Grants koma extended to this extension (registry `granted`, wire strings).
    pub granted: Vec<String>,
    /// Contributed tool count (`contributes.tools`).
    pub tools: usize,
    /// Contributed panel count (`contributes.panels`).
    pub panels: usize,
    /// Contributed sub-agent count (`contributes.sub_agents`).
    pub sub_agents: usize,
    /// Contributed model count (`contributes.models`).
    pub models: usize,
    /// TUI screens this extension drives (`contributes.tui_screens`), selectable in Detail.
    pub tui_screens: Vec<ExtTuiScreen>,
    /// Declared workspace_dir the uninstall nuke would delete (named in the confirm), or `None`.
    pub workspace_dir: Option<String>,
}

/// Working state for the in-app `/extension` dashboard.
///
/// `rows` is a snapshot rebuilt from the registry + manifests on open (and after each
/// uninstall); every key is forwarded to the daemon on an attached client, so this only
/// needs to be faithful enough to DRAW + navigate.
#[derive(Debug, Clone)]
pub struct ExtensionsState {
    /// One row per installed extension, in registry order.
    pub rows: Vec<ExtRow>,
    /// Selected index into `rows` (the LIST cursor).
    pub list_sel: usize,
    /// Active sub-mode (Browse / Detail / UninstallConfirm).
    pub sub_mode: ExtSubMode,
    /// Cursor over the selected extension's `tui_screens` (only meaningful in Detail).
    pub screen_sel: usize,
    /// Last in-state error (e.g. an uninstall failure), shown on the detail pane.
    pub error: Option<String>,
}

impl ExtensionsState {
    /// The currently-selected extension row, if any.
    pub fn current(&self) -> Option<&ExtRow> {
        self.rows.get(self.list_sel)
    }

    /// Move the LIST cursor up.
    pub fn list_up(&mut self) {
        self.list_sel = self.list_sel.saturating_sub(1);
    }

    /// Move the LIST cursor down.
    pub fn list_down(&mut self) {
        if self.list_sel + 1 < self.rows.len() {
            self.list_sel += 1;
        }
    }

    /// Enter DETAIL for the selected extension (resetting the tui-screen cursor).
    pub fn enter_detail(&mut self) {
        self.sub_mode = ExtSubMode::Detail;
        self.screen_sel = 0;
    }

    /// Number of tui-screens on the selected extension (0 when none / no selection).
    pub fn current_tui_screens_len(&self) -> usize {
        self.current().map(|r| r.tui_screens.len()).unwrap_or(0)
    }

    /// Move the tui-screen cursor up (Detail).
    pub fn screen_up(&mut self) {
        self.screen_sel = self.screen_sel.saturating_sub(1);
    }

    /// Move the tui-screen cursor down (Detail).
    pub fn screen_down(&mut self) {
        let n = self.current_tui_screens_len();
        if n > 0 && self.screen_sel + 1 < n {
            self.screen_sel += 1;
        }
    }

    /// The `(ext_id, screen_id, screen_title)` of the tui-screen highlighted in Detail, or
    /// `None` when there is no selection / no tui-screen at the cursor.
    pub fn selected_tui_screen(&self) -> Option<(String, String, String)> {
        let row = self.current()?;
        let ts = row.tui_screens.get(self.screen_sel)?;
        Some((row.id.clone(), ts.id.clone(), ts.title.clone()))
    }
}

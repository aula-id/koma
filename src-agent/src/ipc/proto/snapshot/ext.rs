// ─── extension STORE wire types (browse / detail / installed) ────────────────
//
// Serde-safe, camelCase projections of the koma.run store API's
// `ExtensionSummary` / `ExtensionDetail` (see `koma-landing/docs/
// EXTENSION_STORE_API.md`) plus the local `InstalledExtension` registry entry,
// for the GUI Store tab. These cross the daemon<->client socket inside a
// [`crate::ipc::proto::DaemonEvent`] AND are re-embedded verbatim in the GUI
// host's `PushEnvelope` (so the JS store reads these exact camelCase keys).
//
// CRITICAL: [`InstalledExtWire`] carries ONLY display/registry fields — there is
// no token anywhere in the extension registry, so nothing secret can cross. The
// store summary/detail come from the PUBLIC (no-auth) store endpoints.

use serde::{Deserialize, Serialize};

/// One catalogue row — the store API's `ExtensionSummary` (the `GET /extensions`
/// list item), projected to camelCase for the GUI Store grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct StoreItemWire {
    /// Reverse-DNS id, e.g. `"run.koma.gateway"`.
    pub id: String,
    pub name: String,
    pub tagline: String,
    /// `"free"` | `"paid"`.
    pub tier: String,
    /// `"daemon"` | `"oneshot"`.
    pub kind: String,
    pub latest_version: String,
    pub icon_url: String,
    pub categories: Vec<String>,
    pub author: String,
    pub updated_at: String,
}

/// A summary of what an extension CONTRIBUTES (the `contributes` object of the
/// store API's `ExtensionDetail`), collapsed to per-kind COUNTS — enough for the
/// install card's "provides" line without coupling the wire to the full manifest
/// contribution shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct StoreContributesWire {
    pub models: u32,
    pub panels: u32,
    pub tools: u32,
    pub sub_agents: u32,
}

/// The full detail view — the store API's `ExtensionDetail` (`GET
/// /extensions/{id}`), projected to camelCase for the GUI Store detail pane.
/// Carries everything in [`StoreItemWire`] plus the long description, screenshots,
/// the contribution summary + the `requires` grant list (the install card's
/// "wants" line, e.g. `["agents:read"]`), and the available version list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct StoreDetailWire {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub tier: String,
    pub kind: String,
    pub latest_version: String,
    pub icon_url: String,
    pub categories: Vec<String>,
    pub author: String,
    pub updated_at: String,
    /// Markdown long description (`description_md`).
    pub description_md: String,
    pub screenshots: Vec<String>,
    /// Per-kind contribution counts (models / panels / tools / sub-agents).
    pub contributes: StoreContributesWire,
    /// Requested grants, as wire strings (e.g. `"agents:read"`) — the install
    /// card's permission list ("wants a model provider, sub-agent read").
    pub requires: Vec<String>,
    /// Available version strings, newest first.
    pub versions: Vec<String>,
}

/// A serde-safe projection of one locally-[`InstalledExtension`] registry entry
/// ([`crate::model::app_config::InstalledExtension`]) for the GUI Store's
/// "Installed" section. There is no token in the registry, so this is a full
/// projection minus only the on-disk `exec` path (irrelevant to the UI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct InstalledExtWire {
    pub id: String,
    /// Human-readable name from the installed manifest, when available.
    #[serde(default)]
    pub name: String,
    pub version: String,
    /// `"free"` | `"paid"`.
    pub tier: String,
    /// `"daemon"` | `"oneshot"`.
    pub kind: String,
    pub enabled: bool,
    /// Grants koma extended to this extension (echoed manifest `requires`).
    pub granted: Vec<String>,
    /// This extension's `contributes.panels` (read fresh off its installed
    /// `manifest.json` — NOT part of the persisted `InstalledExtension` registry
    /// entry), for the GUI's extension activity-bar icon + tab framing. Empty for
    /// an extension that contributes no panels (or whose manifest couldn't be
    /// read).
    #[serde(default)]
    pub panels: Vec<PanelWire>,
    /// This extension's declared `workspace_dir` (read fresh off its installed
    /// `manifest.json`, trimmed + non-empty), when it declares one — surfaced so the
    /// GUI's uninstall confirm can name the data directory the nuke will delete. `None`
    /// for an extension that declares no workspace dir (or whose manifest couldn't be
    /// read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
}

/// One contributed panel (a `contributes.panels[]` entry of an installed
/// extension's manifest, [`koma_extension::protocol::PanelDef`]) — the GUI's
/// activity-bar icon + tab framing for `koma://extension/<id>/...`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PanelWire {
    pub id: String,
    pub title: String,
    pub icon: String,
}

/// Full detail of a locally-installed extension — the registry entry PLUS the
/// on-disk `manifest.json` contributions (tools, models, panels, sub-agents),
/// projected to camelCase for the GUI's installed-extension detail tab.
/// Reads ONLY the local filesystem (no network); missing/unreadable manifests
/// degrade to empty contribution lists (never fail the whole projection).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledExtensionDetailWire {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub tier: String,
    pub kind: String,
    pub enabled: bool,
    pub granted: Vec<String>,
    /// Manifest-declared requires (converted to wire strings), not runtime/registry secrets.
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub panels: Vec<PanelWire>,
    #[serde(default)]
    pub tools: Vec<InstalledToolWire>,
    #[serde(default)]
    pub models: Vec<InstalledModelWire>,
    #[serde(default)]
    pub sub_agents: Vec<InstalledSubAgentWire>,
    /// Best-effort online store enrichment for this extension. `None` on the
    /// initial local-only response; populated by a second push after the store
    /// API responds. The GUI merges this into the displayed detail without
    /// disturbing the local data ownership of installed version/permissions.
    #[serde(default)]
    pub store_detail: Option<StoreDetailWire>,
    /// This extension's declared `workspace_dir` (trimmed + non-empty) or `None` — the
    /// data directory the uninstall nuke removes, named in the GUI's uninstall confirm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
}

/// One contributed tool from an installed extension's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledToolWire {
    pub name: String,
    pub description: String,
}

/// One contributed model from an installed extension's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelWire {
    pub id: String,
    pub display_name: String,
}

/// One contributed sub-agent from an installed extension's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSubAgentWire {
    pub name: String,
    pub description: String,
}

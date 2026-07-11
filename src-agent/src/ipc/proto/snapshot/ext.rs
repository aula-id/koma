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

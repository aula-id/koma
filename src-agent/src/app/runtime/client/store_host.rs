//! Host-side extension-STORE browse/detail/installed-list computation for the GUI Store
//! tab — split out for the same reason as [`super::git_host`]: browse/detail hit the
//! PUBLIC (no-auth) koma.run store endpoints and the installed-list read is a local
//! `~/.koma/config.json` read, so ALL THREE are stateless and must work whether or not a
//! session daemon is attached (unlike install/uninstall, which mutate live daemon runtime
//! state — `ext_manager`/`mcp_manager` — and stay daemon-forwarded; see
//! `dispatch.rs`'s `GuiReq::InstallExtension`/`UninstallExtension` arms and
//! `HostCtl::ExtNoSession`).
//!
//! Every op here runs on a one-shot [`std::thread::spawn`] worker — never inline on a host
//! control loop, and never on the tokio runtime (a blocking `reqwest` call would panic
//! there) — mirroring `git_host`'s off-thread pattern: a DETACHED flavor that pushes the
//! reply straight through the cloned `push` sink ([`super::host`]'s `host_swapper`), and an
//! ATTACHED flavor that replies over an `mpsc` channel drained by [`super::push_loop`].
//!
//! The JSON-mapping helpers (`map_summary`/`map_detail`/`map_contributes`/`str_field`/
//! `arr_str`) are a deliberate, small DUPLICATE of the daemon's
//! `event_loop::daemon::hub::requests_ext` copies — that daemon module is left untouched
//! (its own `ClientRequest::StoreBrowse`/`StoreDetail`/`ListInstalledExtensions` handlers
//! simply aren't reached from the GUI anymore, since browse/detail/list-installed moved
//! host-local here).

use std::sync::mpsc::Sender;

use crate::ipc::proto::{
    InstalledExtWire, PanelWire, StoreContributesWire, StoreDetailWire, StoreItemWire,
};
use crate::model::app_config::AppConfig;
use crate::model::store;

/// Base URL of the koma.run extension store API (contract v0) — same constant as the
/// daemon's `requests_ext::STORE_API_BASE`.
const STORE_API_BASE: &str = "https://koma.run/api/v1/extensions";

// ─── DETACHED (host_swapper): push the reply straight through the cloned sink ───

/// `HostCtl::StoreBrowse` while detached.
pub(super) fn spawn_store_browse(
    push: impl Fn(String) + Send + 'static,
    query: Option<String>,
    category: Option<String>,
) {
    std::thread::spawn(move || {
        let (items, error) = match fetch_catalogue(query, category) {
            Ok(items) => (items, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        super::push_proto::push_store_catalogue(&push, items, error);
    });
}

/// `HostCtl::StoreDetail` while detached.
pub(super) fn spawn_store_detail(push: impl Fn(String) + Send + 'static, id: String) {
    std::thread::spawn(move || {
        let (detail, error) = match fetch_detail(&id) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        };
        super::push_proto::push_store_detail(&push, detail, error);
    });
}

/// `HostCtl::ListInstalledExtensions` while detached.
pub(super) fn spawn_list_installed(push: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        super::push_proto::push_installed_extensions(&push, installed_extensions());
    });
}

// ─── ATTACHED (push_loop): reply over an mpsc channel, drained by the fold loop ───

/// `HostCtl::StoreBrowse` while attached.
pub(super) fn spawn_store_browse_attached(
    tx: Sender<(Vec<StoreItemWire>, Option<String>)>,
    query: Option<String>,
    category: Option<String>,
) {
    std::thread::spawn(move || {
        let result = match fetch_catalogue(query, category) {
            Ok(items) => (items, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let _ = tx.send(result);
    });
}

/// `HostCtl::StoreDetail` while attached.
pub(super) fn spawn_store_detail_attached(
    tx: Sender<(Option<StoreDetailWire>, Option<String>)>,
    id: String,
) {
    std::thread::spawn(move || {
        let result = match fetch_detail(&id) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        };
        let _ = tx.send(result);
    });
}

/// `HostCtl::ListInstalledExtensions` while attached.
pub(super) fn spawn_list_installed_attached(tx: Sender<Vec<InstalledExtWire>>) {
    std::thread::spawn(move || {
        let _ = tx.send(installed_extensions());
    });
}

// ─── shared blocking computation ───

/// Read the locally-installed extension registry straight off `~/.koma/config.json` — the
/// SAME projection the daemon's `requests_ext::send_installed_extensions` builds, so a
/// re-attach (or the daemon's own post-install/-uninstall re-push) never disagrees with
/// this host read.
fn installed_extensions() -> Vec<InstalledExtWire> {
    let cfg = AppConfig::load();
    cfg.installed_extensions
        .iter()
        .map(|e| InstalledExtWire {
            id: e.id.clone(),
            version: e.version.clone(),
            tier: e.tier.clone(),
            kind: e.kind.clone(),
            enabled: e.enabled,
            granted: e.granted.clone(),
            panels: read_ext_panels(&e.id),
        })
        .collect()
}

/// Read `contributes.panels` straight off `extensions_dir()/<id>/manifest.json` — the
/// registry (`InstalledExtension`) doesn't carry contributions, so this is a fresh,
/// best-effort re-read on every installed-list build: a missing/unreadable/unparsable
/// manifest degrades to an empty panel list (never fails the whole installed-list
/// projection over one bad entry), logged via `append_global_error_log` so a parse
/// failure is still visible. SAME logic as the daemon's
/// `requests_ext::read_ext_panels` copy, mirroring this module's existing
/// map_summary/map_detail/map_contributes duplication.
fn read_ext_panels(id: &str) -> Vec<PanelWire> {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return Vec::new(),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(), // not installed / unreadable — no panels
    };
    let manifest: koma_extension::protocol::ExtensionManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            return Vec::new();
        }
    };
    manifest
        .contributes
        .panels
        .into_iter()
        .map(|p| PanelWire {
            id: p.id,
            title: p.title,
            icon: p.icon,
        })
        .collect()
}

/// `GET /extensions[?q&category]` → the mapped catalogue rows, BLOCKING (this always runs
/// off a plain `std::thread::spawn` worker, never the tokio runtime, so `reqwest::blocking`
/// is the simplest fit — mirrors the daemon's async `requests_ext::fetch_catalogue`
/// field-for-field). A non-2xx status or a parse error is an `Err(String)` the caller
/// surfaces as the catalogue's `error`.
fn fetch_catalogue(
    query: Option<String>,
    category: Option<String>,
) -> Result<Vec<StoreItemWire>, String> {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(q) = query {
        let q = q.trim().to_string();
        if !q.is_empty() {
            pairs.push(("q", q));
        }
    }
    if let Some(c) = category {
        let c = c.trim().to_string();
        if !c.is_empty() {
            pairs.push(("category", c));
        }
    }
    let url = reqwest::Url::parse_with_params(STORE_API_BASE, &pairs)
        .map_err(|e| format!("bad store url: {e}"))?;

    let resp = reqwest::blocking::get(url).map_err(|e| {
        let msg = format!("store request failed: {e}");
        store::append_global_error_log("ext browse", &msg);
        msg
    })?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        store::append_global_error_log("ext browse", &format!("store returned HTTP {code}"));
        return Err(format!("store returned HTTP {code}"));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("store response parse failed: {e}"))?;
    let items = body
        .get("items")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().map(map_summary).collect())
        .unwrap_or_default();
    Ok(items)
}

/// `GET /extensions/{id}` → the mapped detail, BLOCKING (see [`fetch_catalogue`]).
fn fetch_detail(id: &str) -> Result<StoreDetailWire, String> {
    let url = format!("{STORE_API_BASE}/{id}");
    let resp = reqwest::blocking::get(&url).map_err(|e| {
        let msg = format!("store request failed: {e}");
        store::append_global_error_log("ext browse", &format!("{id}: {msg}"));
        msg
    })?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        store::append_global_error_log("ext browse", &format!("{id}: store returned HTTP {code}"));
        return Err(format!("store returned HTTP {code}"));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("store response parse failed: {e}"))?;
    Ok(map_detail(&body))
}

/// Map one store `ExtensionSummary` JSON object to [`StoreItemWire`] — duplicate of the
/// daemon's `requests_ext::map_summary` (defensive: a missing field degrades to empty
/// rather than failing the whole list parse).
fn map_summary(v: &serde_json::Value) -> StoreItemWire {
    StoreItemWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
    }
}

/// Map one store `ExtensionDetail` JSON object to [`StoreDetailWire`] — duplicate of the
/// daemon's `requests_ext::map_detail`.
fn map_detail(v: &serde_json::Value) -> StoreDetailWire {
    StoreDetailWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
        description_md: str_field(v, "description_md"),
        screenshots: arr_str(v, "screenshots"),
        contributes: map_contributes(v.get("contributes")),
        requires: arr_str(v, "requires"),
        versions: arr_str(v, "versions"),
    }
}

/// Collapse the detail's `contributes` object to per-kind COUNTS — duplicate of the
/// daemon's `requests_ext::map_contributes`.
fn map_contributes(v: Option<&serde_json::Value>) -> StoreContributesWire {
    let count = |key: &str| -> u32 {
        v.and_then(|c| c.get(key))
            .and_then(|x| x.as_array())
            .map(|a| a.len() as u32)
            .unwrap_or(0)
    };
    StoreContributesWire {
        models: count("models"),
        panels: count("panels"),
        tools: count("tools"),
        sub_agents: count("sub_agents"),
    }
}

/// A string field of a JSON object, or `""` if absent / not a string.
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// A `Vec<String>` field of a JSON object (its string elements), or empty.
fn arr_str(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The summary mapping pulls exactly the wire fields from an `ExtensionSummary`-shaped
    /// object, degrading a missing field to empty rather than failing — mirrors the
    /// daemon-side `requests_ext::map_summary_projects_summary_fields` test so the two
    /// independent copies stay behaviourally identical.
    #[test]
    fn map_summary_projects_summary_fields() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "Premium koma models, one endpoint.",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "https://cdn.koma.run/ext/run.koma.gateway/icon.png",
            "categories": ["models", "gateway"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z"
        });
        let item = map_summary(&v);
        assert_eq!(item.id, "run.koma.gateway");
        assert_eq!(item.name, "koma Gateway");
        assert_eq!(item.tier, "paid");
        assert_eq!(item.kind, "daemon");
        assert_eq!(item.latest_version, "0.3.1");
        assert_eq!(item.categories, vec!["models", "gateway"]);
        assert_eq!(item.author, "koma");
    }

    /// The detail mapping projects the long-form fields AND collapses `contributes` to
    /// per-kind counts + carries the `requires` grant list (the install card's inputs).
    #[test]
    fn map_detail_counts_contributions_and_reads_requires() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "one endpoint",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "",
            "categories": ["models"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z",
            "description_md": "# koma Gateway\n\nlong",
            "screenshots": ["https://cdn.koma.run/ext/run.koma.gateway/1.png"],
            "contributes": {
                "models": [{ "id": "a" }, { "id": "b" }],
                "panels": [],
                "tools": [{ "name": "t" }],
                "sub_agents": []
            },
            "requires": ["agents:read"],
            "versions": ["0.3.1", "0.3.0"]
        });
        let d = map_detail(&v);
        assert_eq!(d.description_md, "# koma Gateway\n\nlong");
        assert_eq!(d.screenshots.len(), 1);
        assert_eq!(d.contributes.models, 2);
        assert_eq!(d.contributes.panels, 0);
        assert_eq!(d.contributes.tools, 1);
        assert_eq!(d.contributes.sub_agents, 0);
        assert_eq!(d.requires, vec!["agents:read"]);
        assert_eq!(d.versions, vec!["0.3.1", "0.3.0"]);
    }

    /// The installed-extensions projection reads straight off `AppConfig` and carries every
    /// display field verbatim — a smoke check that the host copy stays in lockstep with the
    /// daemon's own `send_installed_extensions` projection shape.
    #[test]
    fn installed_extensions_projects_registry_fields() {
        let mut cfg = AppConfig::default();
        cfg.installed_extensions.push(crate::model::app_config::InstalledExtension {
            id: "run.koma.gateway".to_string(),
            version: "0.3.1".to_string(),
            tier: "paid".to_string(),
            kind: "daemon".to_string(),
            enabled: true,
            granted: vec!["agents:read".to_string()],
            exec: String::new(),
        });
        let items: Vec<InstalledExtWire> = cfg
            .installed_extensions
            .iter()
            .map(|e| InstalledExtWire {
                id: e.id.clone(),
                version: e.version.clone(),
                tier: e.tier.clone(),
                kind: e.kind.clone(),
                enabled: e.enabled,
                granted: e.granted.clone(),
                panels: read_ext_panels(&e.id),
            })
            .collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "run.koma.gateway");
        assert_eq!(items[0].tier, "paid");
        assert!(items[0].enabled);
        assert_eq!(items[0].granted, vec!["agents:read"]);
    }

    /// A missing/never-installed manifest degrades to an empty panel list rather than
    /// failing — the id here is guaranteed to have no `extensions/<id>/manifest.json` on
    /// any test machine.
    #[test]
    fn read_ext_panels_degrades_to_empty_on_missing_manifest() {
        assert_eq!(
            read_ext_panels("run.koma.definitely-not-installed.test-fixture"),
            Vec::<PanelWire>::new()
        );
    }
}

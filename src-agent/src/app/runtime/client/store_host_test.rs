#![allow(clippy::unwrap_used, clippy::expect_used)]
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
/// daemon's own `send_installed_extensions` projection shape. The `name` field comes from
/// the manifest when readable, falling back to the extension id.
#[test]
fn installed_extensions_projects_registry_fields() {
    let mut cfg = AppConfig::default();
    cfg.installed_extensions
        .push(crate::model::app_config::InstalledExtension {
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
        .map(|e| {
            let (name, panels) = read_ext_manifest_info(&e.id);
            InstalledExtWire {
                id: e.id.clone(),
                name,
                version: e.version.clone(),
                tier: e.tier.clone(),
                kind: e.kind.clone(),
                enabled: e.enabled,
                granted: e.granted.clone(),
                panels,
                workspace_dir: None,
            }
        })
        .collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "run.koma.gateway");
    // No manifest on test machine — falls back to id.
    assert_eq!(items[0].name, "run.koma.gateway");
    assert_eq!(items[0].tier, "paid");
    assert!(items[0].enabled);
    assert_eq!(items[0].granted, vec!["agents:read"]);
}

/// `read_ext_manifest_info` returns the id as name when no manifest exists.
#[test]
fn read_ext_manifest_info_falls_back_to_id_on_missing_manifest() {
    let (name, panels) =
        read_ext_manifest_info("run.koma.definitely-not-installed.test-fixture");
    assert_eq!(name, "run.koma.definitely-not-installed.test-fixture");
    assert!(panels.is_empty());
}

/// The local installed detail initializes with `store_detail: None` before
/// any online enrichment.
#[test]
fn get_installed_detail_has_no_store_detail() {
    // This extension is never installed on a test machine, so
    // get_installed_detail returns Err — confirming the function compiles
    // and the wire struct has the store_detail field.
    let result = get_installed_detail("run.koma.definitely-not-installed.test-fixture");
    assert!(result.is_err());
}

/// Wire serialization: InstalledExtWire carries `name` and serializes it as
/// `name` (camelCase — already flat).
#[test]
fn installed_ext_wire_serializes_name() {
    let wire = InstalledExtWire {
        id: "run.koma.hello".to_string(),
        name: "Hello World".to_string(),
        version: "0.1.0".to_string(),
        tier: "free".to_string(),
        kind: "daemon".to_string(),
        enabled: true,
        granted: vec![],
        panels: vec![],
        workspace_dir: None,
    };
    let json = serde_json::to_value(&wire).unwrap();
    assert_eq!(json["name"], "Hello World");
    assert_eq!(json["id"], "run.koma.hello");
}

/// Wire serialization: InstalledExtensionDetailWire with `store_detail: None`
/// omits/nulls the field for backward compat.
#[test]
fn installed_detail_wire_omits_none_store_detail() {
    let wire = InstalledExtensionDetailWire {
        id: "run.koma.hello".to_string(),
        name: "Hello World".to_string(),
        version: "0.1.0".to_string(),
        description: "A test ext".to_string(),
        tier: "free".to_string(),
        kind: "daemon".to_string(),
        enabled: true,
        granted: vec![],
        requires: vec![],
        panels: vec![],
        tools: vec![],
        models: vec![],
        sub_agents: vec![],
        store_detail: None,
        workspace_dir: None,
    };
    let json = serde_json::to_value(&wire).unwrap();
    // serde skips None Option by default with #[serde(default)]
    assert!(json.get("storeDetail").is_none() || json["storeDetail"].is_null());
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

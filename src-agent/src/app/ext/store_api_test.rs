#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// The build host is always one of the v0 platforms (the test binary itself is one of
/// them), so `detect_platform` must resolve to a `Some` in the advertised set.
#[test]
fn detect_platform_is_a_known_v0_token() {
    let plat = detect_platform().expect("build host must be a v0 store platform");
    assert!(
        [
            "linux-x64",
            "linux-arm64",
            "darwin-x64",
            "darwin-arm64",
            "windows-x64"
        ]
        .contains(&plat),
        "unexpected platform token: {plat}"
    );
}

/// The summary mapping pulls exactly the wire fields from an `ExtensionSummary`-shaped
/// object, degrading a missing field to empty rather than failing.
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

/// A 302 integrity body yields `(sha, Some(sig))`; an empty / malformed body yields the
/// unsigned shape `(empty, None)` — the caller's dev-unsigned trigger.
#[test]
fn parse_integrity_json_reads_or_degrades() {
    let (sha, sig) =
        parse_integrity_json(r#"{"sha256":"3b1f","signature":"MEUCIQ==","size":123}"#);
    assert_eq!(sha, "3b1f");
    assert_eq!(sig.as_deref(), Some("MEUCIQ=="));

    let (sha2, sig2) = parse_integrity_json("");
    assert!(sha2.is_empty());
    assert!(sig2.is_none());

    // Present-but-empty signature is treated as unsigned.
    let (_sha3, sig3) = parse_integrity_json(r#"{"sha256":"aa","signature":""}"#);
    assert!(sig3.is_none());
}

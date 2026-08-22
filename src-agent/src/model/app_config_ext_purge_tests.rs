#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::collections::HashSet;

/// A key-backed ext provider owned by `ext_id`.
fn ext_provider(uuid: &str, ext_id: &str) -> ProviderConn {
    ProviderConn {
        uuid: uuid.to_string(),
        name: "gw".to_string(),
        api_type: ApiType::OpenAiCompatible,
        endpoint: "https://gw.test/v1".to_string(),
        api_key: "k".to_string(),
        ext_id: Some(ext_id.to_string()),
    }
}

fn native_provider(uuid: &str) -> ProviderConn {
    ProviderConn {
        uuid: uuid.to_string(),
        name: uuid.to_string(),
        ..Default::default()
    }
}

#[test]
fn cascade_remove_provider_drops_matching_models_and_flags_main() {
    let mut config = AppConfig::default();
    config.providers.push(native_provider("p1"));
    config.providers.push(native_provider("p2"));
    config.models.push(ModelEntry {
        uuid: "m-main".into(),
        provider_uuid: "p1".into(),
        roles: vec![ModelRole::Main],
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m-other".into(),
        provider_uuid: "p2".into(),
        ..Default::default()
    });
    let report = config.cascade_remove_provider("p1");
    assert_eq!(report.models_removed, vec!["m-main".to_string()]);
    assert!(report.main_reset);
    assert!(config.providers.iter().all(|p| p.uuid != "p1"));
    assert!(config.models.iter().all(|m| m.uuid != "m-main"));
    assert!(config.models.iter().any(|m| m.uuid == "m-other"));
    // Missing provider is a no-op.
    let empty = config.cascade_remove_provider("nope");
    assert!(empty.models_removed.is_empty());
    assert!(!empty.main_reset);
}

#[test]
fn cascade_remove_oauth_conn_drops_matching_models() {
    let mut config = AppConfig::default();
    config.oauth_conns.push(OAuthConn {
        uuid: "oauth-1".into(),
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m-oauth".into(),
        provider_uuid: "oauth-1".into(),
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m-keep".into(),
        provider_uuid: "other".into(),
        ..Default::default()
    });
    let report = config.cascade_remove_oauth_conn("oauth-1");
    assert_eq!(report.models_removed, vec!["m-oauth".to_string()]);
    assert!(!report.main_reset);
    assert!(config.oauth_conns.is_empty());
    assert!(config.models.iter().all(|m| m.uuid != "m-oauth"));
    assert!(config.models.iter().any(|m| m.uuid == "m-keep"));
}

#[test]
fn cascade_remove_models_drops_by_uuid_and_flags_main() {
    let mut config = AppConfig::default();
    config.models.push(ModelEntry {
        uuid: "m1".into(),
        roles: vec![ModelRole::Main],
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m2".into(),
        ..Default::default()
    });
    let mut dead = HashSet::new();
    dead.insert("m1".into());
    dead.insert("missing".into());
    let report = config.cascade_remove_models(&dead);
    assert_eq!(report.models_removed, vec!["m1".to_string()]);
    assert!(report.main_reset);
    assert!(config.models.iter().all(|m| m.uuid != "m1"));
    assert!(config.models.iter().any(|m| m.uuid == "m2"));
}

#[test]
fn remove_provider_by_uuid_cascades_models() {
    let mut config = AppConfig::default();
    config.providers.push(native_provider("p1"));
    config.models.push(ModelEntry {
        uuid: "m1".into(),
        provider_uuid: "p1".into(),
        ..Default::default()
    });
    config.remove_provider_by_uuid("p1");
    assert!(config.providers.is_empty());
    assert!(config.models.is_empty(), "thin wrapper must cascade");
}

/// `purge_extension` removes the extension's providers + oauth conns + orphaned models +
/// preferred record, leaves EVERY other owner's entries untouched, and reports `main_reset`
/// only when a removed model held the global Main role.
#[test]
fn purge_removes_ext_entries_and_reports_main_reset() {
    let mut config = AppConfig::default();
    // ext A: one key-backed provider + one oauth conn, two models (one holds Main).
    config.providers.push(ext_provider("prov-a", "ext.a"));
    config.oauth_conns.push(OAuthConn {
        uuid: "conn-a".to_string(),
        provider: OAuthProvider::Extension,
        ext_id: Some("ext.a".to_string()),
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m-a-main".to_string(),
        model_id: "big".to_string(),
        provider_uuid: "prov-a".to_string(),
        roles: vec![ModelRole::Main],
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m-a-conn".to_string(),
        model_id: "small".to_string(),
        provider_uuid: "conn-a".to_string(),
        ..Default::default()
    });
    config
        .ext_preferred_models
        .insert("ext.a".to_string(), "m-a-main".to_string());
    // ext B + a native provider/model that must SURVIVE the purge of A.
    config.providers.push(ext_provider("prov-b", "ext.b"));
    config.providers.push(ProviderConn {
        uuid: "prov-native".to_string(),
        name: "native".to_string(),
        ..Default::default()
    });
    config.models.push(ModelEntry {
        uuid: "m-native".to_string(),
        provider_uuid: "prov-native".to_string(),
        ..Default::default()
    });
    config
        .ext_preferred_models
        .insert("ext.b".to_string(), "m-b".to_string());

    let report = config.purge_extension("ext.a");
    assert_eq!(report.providers_removed, 1);
    assert_eq!(report.conns_removed, 1);
    assert_eq!(
        report.models_removed, 2,
        "both of A's models (provider + conn backed) are swept"
    );
    assert!(
        report.main_reset,
        "a removed model held the global Main role"
    );

    // A is gone; B + native survive.
    assert!(config.providers.iter().all(|p| p.uuid != "prov-a"));
    assert!(config.oauth_conns.is_empty());
    assert!(config
        .models
        .iter()
        .all(|m| m.provider_uuid != "prov-a" && m.provider_uuid != "conn-a"));
    assert!(
        config.providers.iter().any(|p| p.uuid == "prov-b"),
        "another extension is untouched"
    );
    assert!(
        config.providers.iter().any(|p| p.uuid == "prov-native"),
        "a native provider is untouched"
    );
    assert!(
        config.models.iter().any(|m| m.uuid == "m-native"),
        "a native model is untouched"
    );
    assert_eq!(
        config.ext_preferred_models.get("ext.a"),
        None,
        "A's preferred record is cleared"
    );
    assert_eq!(
        config.ext_preferred_models.get("ext.b").map(String::as_str),
        Some("m-b"),
        "another extension's preferred record is untouched"
    );
}

/// Purging an extension whose models hold NO runtime role leaves `main_reset` false.
#[test]
fn purge_without_main_holder_does_not_flag_reset() {
    let mut config = AppConfig::default();
    config.providers.push(ext_provider("prov-a", "ext.a"));
    config.models.push(ModelEntry {
        uuid: "m1".to_string(),
        provider_uuid: "prov-a".to_string(),
        ..Default::default()
    });
    let report = config.purge_extension("ext.a");
    assert_eq!(report.providers_removed, 1);
    assert_eq!(report.models_removed, 1);
    assert!(!report.main_reset, "no removed model held Main");
}

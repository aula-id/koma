#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::model::app_config::{AppConfig, ModelEntry, ModelRole, ProviderConn};

use super::*;

/// Build a minimal AppConfig with one provider and one global model entry.
fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-1".to_string(),
        name: "Test Provider".to_string(),
        endpoint: "https://api.test.com".to_string(),
        api_key: "test-key".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-1".to_string(),
        name: "test-model".to_string(),
        model_id: "test/model-v1".to_string(),
        provider_uuid: "prov-1".to_string(),
        route: None,
        roles: vec![ModelRole::Main],
        role: None,
        source_uuid: None,
    });
    config
}

#[test]
fn entry_label_basic() {
    let config = test_config();
    let entry = &config.models[0];
    let label = entry_label(&config, entry);
    assert_eq!(label, "test-model — test/model-v1 @ Test Provider");
}

#[test]
fn entry_label_unknown_provider() {
    let config = test_config();
    let mut entry = config.models[0].clone();
    entry.provider_uuid = "prov-does-not-exist".to_string();
    let label = entry_label(&config, &entry);
    assert_eq!(label, "test-model — test/model-v1 @ ?");
}

#[test]
fn entry_label_oauth_provider() {
    let mut config = test_config();
    // Remove the regular provider so the oauth fallback path is tested.
    config.providers.clear();
    config.oauth_conns.push(crate::model::app_config::OAuthConn {
        uuid: "prov-1".to_string(),
        name: "My OAuth".to_string(),
        provider: crate::model::app_config::OAuthProvider::Codex,
        access_token: "tok".to_string(),
        ..crate::model::app_config::OAuthConn::default()
    });
    let label = entry_label(&config, &config.models[0]);
    assert_eq!(label, "test-model — test/model-v1 @ My OAuth");
}

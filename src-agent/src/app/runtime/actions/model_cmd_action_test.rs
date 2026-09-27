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
    config
        .oauth_conns
        .push(crate::model::app_config::OAuthConn {
            uuid: "prov-1".to_string(),
            name: "My OAuth".to_string(),
            provider: crate::model::app_config::OAuthProvider::Codex,
            access_token: "tok".to_string(),
            ..crate::model::app_config::OAuthConn::default()
        });
    let label = entry_label(&config, &config.models[0]);
    assert_eq!(label, "test-model — test/model-v1 @ My OAuth");
}

#[test]
fn role_swap_and_inherit_preserve_secondary_session_role() {
    let path = std::env::temp_dir().join(format!("koma-model-contract-{}", uuid::Uuid::new_v4()));
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    state.rest.config = test_config();
    let settings = crate::model::settings::Settings {
        session_models: vec![ModelEntry {
            uuid: "local".into(),
            roles: vec![ModelRole::Main, ModelRole::Safeguard],
            ..Default::default()
        }],
        ..Default::default()
    };
    state.rest.fg_mut().session = Some(crate::model::session::Session::new(
        "test".into(),
        path.clone(),
        "pwd".into(),
        settings,
        crate::model::conversation::Conversation::from_messages(vec![]),
    ));
    handle_model_role_swap(ModelRole::Main, Some("model-1".into()), &mut state).unwrap();
    handle_model_role_swap(ModelRole::Main, None, &mut state).unwrap();
    let models = &state
        .rest
        .fg()
        .session
        .as_ref()
        .unwrap()
        .settings
        .session_models;
    assert_eq!(models[0].roles, vec![ModelRole::Safeguard]);
    assert!(!models
        .iter()
        .any(|m| m.effective_roles().contains(&ModelRole::Main)));
    std::fs::remove_dir_all(path).unwrap();
}

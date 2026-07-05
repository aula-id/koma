use super::*;
use crate::model::app_config::{ModelEntry, ProviderConn};

/// Build a minimal config with one Main model + one Planner model, each on its
/// own provider connection, so `resolve_role` resolves both independently
/// (Planner must never inherit Main's route the way Compactor/Awareness do).
fn config_with(main_model: &str, main_endpoint: &str, planner_model: &str, planner_endpoint: &str) -> AppConfig {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-main".to_string(),
        name: "main-provider".to_string(),
        endpoint: main_endpoint.to_string(),
        api_key: "key-main".to_string(),
        ..ProviderConn::default()
    });
    config.providers.push(ProviderConn {
        uuid: "prov-planner".to_string(),
        name: "planner-provider".to_string(),
        endpoint: planner_endpoint.to_string(),
        api_key: "key-planner".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-main".to_string(),
        name: "Main".to_string(),
        model_id: main_model.to_string(),
        provider_uuid: "prov-main".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-planner".to_string(),
        name: "Planner".to_string(),
        model_id: planner_model.to_string(),
        provider_uuid: "prov-planner".to_string(),
        roles: vec![ModelRole::Planner],
        ..ModelEntry::default()
    });
    config
}

#[test]
fn non_plan_mode_always_uses_main_even_with_planner_assigned() {
    let config = config_with("main/model", "https://main.example", "planner/model", "https://planner.example");
    let settings = Settings::default();

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Auto).unwrap();
    assert_eq!(resolved.model_id, "main/model");
    assert_eq!(resolved.endpoint, "https://main.example");
}

#[test]
fn plan_mode_with_distinct_planner_uses_planner() {
    let config = config_with("main/model", "https://main.example", "planner/model", "https://planner.example");
    let settings = Settings::default();

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Plan).unwrap();
    assert_eq!(resolved.model_id, "planner/model");
    assert_eq!(resolved.endpoint, "https://planner.example");
}

#[test]
fn plan_mode_with_no_planner_assigned_stays_on_main() {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-main".to_string(),
        name: "main-provider".to_string(),
        endpoint: "https://main.example".to_string(),
        api_key: "key-main".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-main".to_string(),
        name: "Main".to_string(),
        model_id: "main/model".to_string(),
        provider_uuid: "prov-main".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Plan).unwrap();
    assert_eq!(resolved.model_id, "main/model");
}

#[test]
fn plan_mode_with_planner_same_route_as_main_keeps_main_resolved() {
    // Planner assigned to the SAME model_id/endpoint/route as Main (e.g. the
    // user pinned the same model to both roles): the caller should get Main's
    // `Resolved` unchanged, not a structurally-identical Planner copy — this is
    // the prompt-cache-continuity guarantee.
    let config = config_with("shared/model", "https://shared.example", "shared/model", "https://shared.example");
    let settings = Settings::default();

    let main_only = resolve_role(&config, &settings, ModelRole::Main).unwrap();
    let turn = resolve_turn_model(&config, &settings, AgentMode::Plan).unwrap();

    assert_eq!(turn.model_id, main_only.model_id);
    assert_eq!(turn.endpoint, main_only.endpoint);
    assert_eq!(turn.route, main_only.route);
}

#[test]
fn planner_role_has_no_legacy_fallback() {
    // An unassigned Planner must resolve to `None` from `resolve_role` directly
    // (no legacy fallback), even though `resolve_turn_model` papers over that
    // with Main.
    let config = AppConfig::default();
    let settings = Settings::default();

    assert!(resolve_role(&config, &settings, ModelRole::Planner).is_none());
}

use crate::model::app_config::{OAuthConn, OAuthProvider};

fn oauth_model_entry(provider_uuid: &str) -> ModelEntry {
    ModelEntry {
        uuid: "model-uuid".to_string(),
        name: "test".to_string(),
        model_id: "test-model".to_string(),
        provider_uuid: provider_uuid.to_string(),
        route: None,
        roles: vec![ModelRole::Main],
        role: None,
    }
}

#[test]
fn resolve_role_falls_back_to_codex_oauth_conn() {
    let conn = OAuthConn {
        uuid: "codex-uuid".to_string(),
        provider: OAuthProvider::Codex,
        access_token: "codex-token".to_string(),
        account_id: "acct-123".to_string(),
        ..Default::default()
    };
    let mut config = AppConfig::default();
    config.oauth_conns.push(conn);
    config.models.push(oauth_model_entry("codex-uuid"));
    let settings = Settings::default();

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(resolved.endpoint, crate::service::oauth::registry::meta(OAuthProvider::Codex).chat_endpoint);
    assert_eq!(resolved.api_key, "codex-token");
    assert_eq!(resolved.api_type, ApiType::Codex);
    assert_eq!(resolved.account_id, "acct-123");
    assert_eq!(resolved.oauth_uuid, "codex-uuid");
}

#[test]
fn resolve_role_falls_back_to_kilocode_oauth_conn() {
    let conn = OAuthConn {
        uuid: "kilo-uuid".to_string(),
        provider: OAuthProvider::Kilocode,
        access_token: "kilo-token".to_string(),
        org_id: "org-456".to_string(),
        ..Default::default()
    };
    let mut config = AppConfig::default();
    config.oauth_conns.push(conn);
    config.models.push(oauth_model_entry("kilo-uuid"));
    let settings = Settings::default();

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(resolved.endpoint, crate::service::oauth::registry::meta(OAuthProvider::Kilocode).chat_endpoint);
    assert_eq!(resolved.api_key, "kilo-token");
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(resolved.account_id, "org-456");
    assert_eq!(resolved.oauth_uuid, "kilo-uuid");
}

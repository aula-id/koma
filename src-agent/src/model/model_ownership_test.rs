use super::*;

fn entry(uuid: &str, roles: Vec<ModelRole>) -> ModelEntry {
    ModelEntry {
        uuid: uuid.into(),
        roles,
        ..ModelEntry::default()
    }
}

#[test]
fn assignment_and_inherit_preserve_other_roles_and_entries() {
    let mut models = vec![entry("old", vec![ModelRole::Main, ModelRole::Safeguard])];
    let source = entry("catalogue", vec![ModelRole::Planner]);
    assign_role(&mut models, ModelRole::Main, Some(&source));
    assert_eq!(models[0].roles, vec![ModelRole::Safeguard]);
    assert_eq!(models[1].roles, vec![ModelRole::Main]);
    assert_eq!(models[1].source_uuid.as_deref(), Some("catalogue"));
    assign_role(&mut models, ModelRole::Main, None);
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].roles, vec![ModelRole::Safeguard]);
    assert!(models[1].roles.is_empty());
}

#[test]
fn legacy_roles_and_first_owner_survive_repeated_roundtrips() {
    let mut config: AppConfig = serde_json::from_value(serde_json::json!({"models": [
        {"uuid": "first", "role": "main"},
        {"uuid": "second", "roles": ["main", "awareness"], "role": "safeguard"}
    ]}))
    .unwrap();
    for _ in 0..3 {
        let json = serde_json::to_value(&config).unwrap();
        assert!(json["models"][0].get("role").is_none());
        config = serde_json::from_value(json).unwrap();
        assert_eq!(config.models[0].roles, vec![ModelRole::Main]);
        assert_eq!(config.models[1].roles, vec![ModelRole::Awareness]);
    }
}

#[test]
fn stripping_last_current_role_does_not_resurrect_legacy() {
    let mut model = entry("both", vec![ModelRole::Main]);
    model.role = Some(ModelRole::Safeguard);
    strip_role(&mut model, ModelRole::Main);
    assert!(model.effective_roles().is_empty());
}

#[test]
fn session_legacy_roles_are_serialized_before_legacy_field_disappears() {
    let mut settings = crate::model::settings::Settings::default();
    let mut model = entry("legacy", vec![]);
    model.role = Some(ModelRole::Compactor);
    settings.session_models.push(model);
    let loaded: crate::model::settings::Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(loaded.session_models[0].roles, vec![ModelRole::Compactor]);
}

#[test]
fn apple_setup_fills_empty_roles_without_stealing_existing_ownership() {
    let mut config = AppConfig::default();
    config
        .models
        .push(entry("custom", vec![ModelRole::Main, ModelRole::Safeguard]));
    crate::service::koma_free::ensure_koma_free_config(&mut config);
    crate::service::koma_free::ensure_koma_free_config(&mut config);
    assert_eq!(config.models.len(), 2);
    assert_eq!(
        config.models[0].roles,
        vec![ModelRole::Main, ModelRole::Safeguard]
    );
    assert_eq!(config.models[1].roles.len(), 3);
}

#[test]
fn fresh_apple_and_old_multi_role_assignments_keep_all_five_roles() {
    let mut config = AppConfig::default();
    crate::service::koma_free::ensure_koma_free_config(&mut config);
    assert_eq!(config.models[0].roles.len(), 5);
    let loaded: AppConfig = serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
    assert_eq!(loaded.models[0].roles, config.models[0].roles);
}

#[test]
fn standalone_legacy_entry_serializes_effective_roles() {
    let mut model = entry("legacy", vec![]);
    model.role = Some(ModelRole::Main);
    let serialized = serde_json::to_value(&model).unwrap();
    assert_eq!(serialized["roles"], serde_json::json!(["main"]));
    assert!(serialized.get("role").is_none());
}

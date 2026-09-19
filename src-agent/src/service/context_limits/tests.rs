use super::*;
fn model(id: &str, window: u64) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        context_length: Some(window),
        ..Default::default()
    }
}
fn limits(name: &str, models: &[CatalogModel]) -> ContextLimits {
    resolve(name, "https://example.invalid/v1", "", 0, models, &[])
}
#[test]
fn canonical_and_human_names_match_without_erasing_variants() {
    let models = vec![
        model("anthropic/claude-3.5-sonnet", 200_000),
        model("openai/gpt-4o-mini", 128_000),
    ];
    for name in [
        "Claude 3-5 Sonnet",
        "Anthropic Claude Sonnet 3.5",
        "anthropic/claude-3.5-sonnet",
    ] {
        assert_eq!(
            limits(name, &models).matched_model.as_deref(),
            Some("anthropic/claude-3.5-sonnet")
        );
    }
    for name in [
        "claude-3.6-sonnet",
        "claude-3.5-opus",
        "claude-3.5-sonnet-20241022",
        "gpt-4o",
        "gpt-4o-nano",
    ] {
        assert!(limits(name, &models).matched_model.is_none(), "{name}");
    }
    assert_eq!(
        limits("gpt-4o-mini", &models).matched_model.as_deref(),
        Some("openai/gpt-4o-mini")
    );
}
#[test]
fn million_window_uses_300k_operating_limit_and_smaller_overrides() {
    let models = vec![model("vendor/model-1", 1_000_000)];
    let got = limits("vendor/model-1", &models);
    assert_eq!(got.effective_window, 300_000);
    assert_eq!(got.effective_window * 60 / 100, 180_000);
    assert_eq!(got.effective_window * 75 / 100, 225_000);
    assert_eq!(
        resolve("unknown", "", "vendor/model-1", 96_000, &models, &[]).effective_window,
        96_000
    );
}
#[test]
fn fuzzy_and_ambiguous_matches_cannot_raise_context() {
    let models = vec![model("anthropic/claude-4-sonnet", 1_000_000)];
    let got = limits("claudee-4-sonnet", &models);
    assert_eq!(got.match_method, "fuzzy_estimate");
    assert_eq!(got.effective_window, FALLBACK_WINDOW);
    let ambiguous = vec![
        model("vendor-a/model-1", 1_000_000),
        model("vendor-b/model-1", 32_000),
    ];
    let got = limits("model-1", &ambiguous);
    assert_eq!(got.match_method, "ambiguous");
    assert_eq!(got.effective_window, 32_000);
    assert!(got.matched_model.is_none());
}
#[test]
fn actual_endpoint_vendor_and_limit_constrain_catalogue() {
    let models = vec![
        model("openai/gpt-4o", 1_000_000),
        model("other/gpt-4o", 64_000),
    ];
    let native: ModelInfo =
        serde_json::from_value(serde_json::json!({"id":"gpt-4o", "context_length":64_000}))
            .unwrap();
    let got = resolve(
        "gpt-4o",
        "https://chatgpt.com/backend-api/codex",
        "",
        0,
        &models,
        &[native],
    );
    assert_eq!(got.matched_model.as_deref(), Some("openai/gpt-4o"));
    assert_eq!(got.effective_window, 64_000);
    assert!(limits("gpt-4o", &models).matched_model.is_none());
}
#[test]
fn output_budget_respects_provider_and_full_request_room() {
    let mut model = model("vendor/model-1", 1_000_000);
    model.top_provider.max_completion_tokens = Some(8192);
    let got = limits("vendor/model-1", &[model]);
    assert_eq!(got.desired_output(0), 8192);
    assert_eq!(got.output_tokens(0, 280_000).unwrap(), 8192);
    assert!(got.output_tokens(0, 298_000).is_err());
    assert_eq!(got.output_tokens(1, 298_000).unwrap(), 1);
}
#[test]
fn empty_catalogue_and_legacy_settings_have_stable_defaults() {
    assert_eq!(limits("unknown", &[]).effective_window, FALLBACK_WINDOW);
    let settings: crate::model::settings::Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.context_window_limit, 0);
    assert!(settings.context_model_alias.is_empty());
}

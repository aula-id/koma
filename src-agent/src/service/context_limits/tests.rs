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
    let unknown = limits("unknown", &[]);
    assert_eq!(unknown.effective_window, FALLBACK_WINDOW);
    assert_eq!(unknown.desired_output(0), 128_000);
    let settings: crate::model::settings::Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.context_window_limit, 0);
    assert!(settings.context_model_alias.is_empty());
}

#[test]
fn missing_null_and_zero_context_use_128k_reply_fallback_on_every_endpoint() {
    for endpoint in [
        "https://openrouter.ai/api/v1",
        "https://api.x.ai/v1",
        "https://api.anthropic.com",
        "https://chatgpt.com/backend-api/codex",
        "https://example.invalid/v1",
    ] {
        for public in [
            Vec::new(),
            vec![serde_json::from_value(serde_json::json!({"id":"vendor/model-1","context_length":null,"top_provider":null})).unwrap()],
            vec![serde_json::from_value(serde_json::json!({"id":"vendor/model-1","context_length":0,"top_provider":{"context_length":0}})).unwrap()],
        ] {
            let got = resolve("vendor/model-1", endpoint, "", 0, &public, &[]);
            assert_eq!(got.effective_window, 128_000);
            assert_eq!(got.auto_output_tokens, 128_000);
            assert_eq!(got.catalogue_source, "fallback");
            assert_eq!(got.output_tokens(0, 80_000).unwrap(), 46_976);
            assert_eq!(got.output_tokens(8192, 80_000).unwrap(), 8192);
            assert_eq!(got.output_tokens(512_000, 80_000).unwrap(), 46_976);
        }
    }
}

#[test]
fn missing_context_still_respects_output_metadata_and_smaller_native_limits() {
    let public = CatalogModel {
        id: "vendor/model-1".into(),
        top_provider: TopProvider {
            max_completion_tokens: Some(8192),
            ..Default::default()
        },
        ..Default::default()
    };
    let native: ModelInfo =
        serde_json::from_value(serde_json::json!({"id":"vendor/model-1","context_length":64000}))
            .unwrap();
    // Both exact and normalized identity matches preserve output metadata.
    for query in ["vendor/model-1", "Model 1"] {
        let got = resolve(query, "", "", 0, &[public.clone()], &[native.clone()]);
        assert_eq!(got.effective_window, 64_000);
        assert_eq!(got.catalogue_source, "active_provider");
        assert_eq!(got.auto_output_tokens, 128_000);
        assert_eq!(got.output_tokens(0, 50_000).unwrap(), 8192);
        assert_eq!(got.output_tokens(4000, 50_000).unwrap(), 4000);
    }
}

#[test]
fn missing_public_context_does_not_make_an_uncertain_native_window_trusted() {
    let public = CatalogModel {
        id: "vendor/modeel-1".into(),
        ..Default::default()
    };
    let native: ModelInfo = serde_json::from_value(serde_json::json!({
        "id":"vendor/model-1", "context_length":1_000_000,
    }))
    .unwrap();
    let got = resolve("vendor/modeel-1", "", "", 0, &[public], &[native]);
    assert_eq!(got.match_method, "exact");
    assert_eq!(got.route_window, Some(1_000_000));
    assert_eq!(got.effective_window, 128_000);
}

#[test]
fn missing_context_does_not_hide_identity_ambiguity() {
    let models = [
        model("vendor-a/model-1", 1_000_000),
        CatalogModel {
            id: "vendor-b/model-1".into(),
            ..Default::default()
        },
    ];
    let got = limits("model-1", &models);
    assert_eq!(got.match_method, "ambiguous");
    assert!(got.matched_model.is_none());
    assert_eq!(got.effective_window, 128_000);
}

#[test]
fn catalogue_lookup_ignores_zero_and_keeps_the_smaller_positive_limit() {
    for (nominal, provider, expected) in [
        (None, None, None),
        (Some(0), Some(0), None),
        (Some(64_000), Some(0), Some(64_000)),
        (Some(0), Some(32_000), Some(32_000)),
        (Some(64_000), Some(128_000), Some(64_000)),
        (Some(128_000), Some(64_000), Some(64_000)),
    ] {
        let native: ModelInfo = serde_json::from_value(serde_json::json!({
            "id":"vendor/model-1", "context_length":nominal,
            "top_provider":{"context_length":provider},
        }))
        .unwrap();
        assert_eq!(
            crate::service::openrouter::context_length_for(&[native], "vendor/model-1"),
            expected
        );
    }
}

#[test]
fn nullable_provider_metadata_keeps_nominal_window_and_canonical_ids_match() {
    let mut model: CatalogModel = serde_json::from_value(serde_json::json!({
        "id":"vendor/model-1", "canonical_slug":"vendor/model-1-20260901",
        "context_length":64000, "top_provider":null,
    }))
    .unwrap();
    assert_eq!(
        limits("vendor/model-1-20260901", &[model.clone()]).effective_window,
        64000
    );
    model.top_provider.context_length = Some(32000);
    assert_eq!(
        limits("vendor/model-1", &[model.clone()]).effective_window,
        32000
    );
    model.top_provider.context_length = Some(128000);
    assert_eq!(limits("vendor/model-1", &[model]).effective_window, 64000);
}

#[test]
fn small_models_can_use_a_proportional_reply_reserve() {
    let got = limits("vendor/small", &[model("vendor/small", 8000)]);
    assert_eq!(got.reserved_output(0), 1600);
    assert_eq!(got.output_tokens(0, 5000).unwrap(), 1976);
    assert!(got.output_tokens(0, 6500).is_err());
}

#[test]
fn known_native_model_is_fallback_when_public_catalogue_is_unavailable() {
    let native: ModelInfo = serde_json::from_value(serde_json::json!({
        "id":"local-model", "context_length":1_000_000,
    }))
    .unwrap();
    let got = resolve("local-model", "", "", 0, &[], &[native]);
    assert_eq!(got.catalogue_source, "active_provider");
    assert_eq!(got.effective_window, 300_000);
}

#[test]
fn custom_reply_limit_wins_and_zero_uses_128k_on_every_endpoint() {
    for endpoint in [
        "https://openrouter.ai/api/v1",
        "https://api.x.ai/v1",
        "https://api.anthropic.com",
        "https://chatgpt.com/backend-api/codex",
        "https://example.invalid/v1",
    ] {
        let got = resolve(
            "vendor/model-1",
            endpoint,
            "",
            0,
            &[model("vendor/model-1", 1_000_000)],
            &[],
        );
        assert_eq!(got.output_tokens(0, 50_000).unwrap(), 128_000);
        assert_eq!(got.output_tokens(8192, 50_000).unwrap(), 8192);
        assert_eq!(got.output_tokens(200_000, 50_000).unwrap(), 200_000);
        assert_eq!(got.output_tokens(400_000, 50_000).unwrap(), 248_976);
        assert_eq!(got.output_tokens(0, 225_000).unwrap(), 73_976);
        assert!(got.output_tokens(0, 298_000).is_err());
    }
}

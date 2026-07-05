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

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Auto, false).unwrap();
    assert_eq!(resolved.model_id, "main/model");
    assert_eq!(resolved.endpoint, "https://main.example");
}

#[test]
fn plan_mode_with_distinct_planner_uses_planner() {
    let config = config_with("main/model", "https://main.example", "planner/model", "https://planner.example");
    let settings = Settings::default();

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Plan, false).unwrap();
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

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Plan, false).unwrap();
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
    let turn = resolve_turn_model(&config, &settings, AgentMode::Plan, false).unwrap();

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

#[test]
fn free_mode_forces_main_to_koma_free() {
    // A normal keyed provider + Main model configured; `free_mode = true` must
    // still short-circuit Main onto the keyless koma-free route.
    let config = config_with("main/model", "https://main.example", "planner/model", "https://planner.example");
    let settings = Settings::default();

    let resolved = resolve_role_free(&config, &settings, ModelRole::Main, true)
        .expect("free-mode Main must resolve");
    assert_eq!(resolved.api_type, ApiType::KomaFree);
    assert_eq!(resolved.endpoint, crate::service::koma_free::KOMA_FREE_ENDPOINT);
    assert_eq!(resolved.model_id, crate::service::koma_free::KOMA_FREE_MODEL);
    assert_eq!(resolved.install_id, config.install_id);
}

#[test]
fn free_mode_forces_compactor_and_awareness() {
    let config = config_with("main/model", "https://main.example", "planner/model", "https://planner.example");
    let settings = Settings::default();

    let compactor = resolve_role_free(&config, &settings, ModelRole::Compactor, true)
        .expect("free-mode Compactor must resolve");
    assert_eq!(compactor.api_type, ApiType::KomaFree);
    assert_eq!(compactor.endpoint, crate::service::koma_free::KOMA_FREE_ENDPOINT);
    assert_eq!(compactor.model_id, crate::service::koma_free::KOMA_FREE_MODEL);

    let awareness = resolve_role_free(&config, &settings, ModelRole::Awareness, true)
        .expect("free-mode Awareness must resolve");
    assert_eq!(awareness.api_type, ApiType::KomaFree);
    assert_eq!(awareness.endpoint, crate::service::koma_free::KOMA_FREE_ENDPOINT);
    assert_eq!(awareness.model_id, crate::service::koma_free::KOMA_FREE_MODEL);
}

#[test]
fn free_mode_leaves_planner_and_safeguard_alone() {
    // Planner and Safeguard never match the free-mode short-circuit set — with
    // no config assigned to either, they behave exactly like plain
    // `resolve_role` with free_mode false: Safeguard fail-closed `None`,
    // Planner `None` (no legacy fallback).
    let config = AppConfig::default();
    let settings = Settings::default();

    assert!(resolve_role_free(&config, &settings, ModelRole::Safeguard, true).is_none());
    assert!(resolve_role_free(&config, &settings, ModelRole::Planner, true).is_none());
}

#[test]
fn free_mode_off_is_identical() {
    let config = config_with("main/model", "https://main.example", "planner/model", "https://planner.example");
    let settings = Settings::default();

    let free_off = resolve_role_free(&config, &settings, ModelRole::Main, false)
        .expect("Main must resolve");
    let plain = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(free_off.model_id, plain.model_id);
    assert_eq!(free_off.endpoint, plain.endpoint);
    assert_eq!(free_off.api_key, plain.api_key);
    assert_eq!(free_off.api_type, plain.api_type);
    assert_eq!(free_off.route, plain.route);
    assert_eq!(free_off.effort, plain.effort);
    assert_eq!(free_off.account_id, plain.account_id);
    assert_eq!(free_off.oauth_uuid, plain.oauth_uuid);
    assert_eq!(free_off.install_id, plain.install_id);
}

#[test]
fn main_no_key_falls_back_to_koma_free() {
    // Fresh / unconfigured: empty global config + no legacy api_key. The Main FALLBACK
    // must resolve to the keyless koma-free tier, NOT a dead `gpt-4o-mini`@OpenRouter
    // route. (A non-koma `model` isolates the no-key branch from the koma-default one.)
    let config = AppConfig::default();
    let settings = Settings {
        api_key: String::new(),
        model: "openai/gpt-4o-mini".to_string(),
        ..Default::default()
    };
    // The koma-free fallback now fires ONLY for a truly unconfigured install; this is
    // that case (empty catalogue + no legacy key).
    assert!(config.is_unconfigured(&settings));

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(resolved.api_type, ApiType::KomaFree);
    assert_eq!(resolved.endpoint, crate::service::koma_free::KOMA_FREE_ENDPOINT);
    assert_eq!(resolved.model_id, crate::service::koma_free::KOMA_FREE_MODEL);
    assert_eq!(resolved.install_id, config.install_id);
    assert!(resolved.api_key.is_empty());
}

#[test]
fn main_koma_default_with_legacy_key_falls_to_legacy_not_koma_free() {
    // BEHAVIOUR CHANGE: the koma-free fallback now gates on `is_unconfigured`, NOT on
    // `settings.model == KOMA_FREE_MODEL`. A user WITH a legacy api_key is CONFIGURED, so
    // even though `settings.model` is the koma/apple DEFAULT_MODEL, Main is NOT forced onto
    // koma-free — it falls to the legacy settings route. (The old guard fired koma-free
    // here; because DEFAULT_MODEL == koma/apple it was permanently armed and silently
    // overrode every keyed user's model.)
    let config = AppConfig::default();
    let settings = Settings {
        api_key: "sk-or-legacy".to_string(),
        model: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        ..Default::default()
    };
    assert!(!config.is_unconfigured(&settings));

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_ne!(resolved.api_type, ApiType::KomaFree);
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(resolved.endpoint, crate::config::DEFAULT_BASE_URL);
    assert_eq!(resolved.api_key, "sk-or-legacy");
}

#[test]
fn main_with_legacy_key_and_real_model_still_uses_legacy_main() {
    // Legacy keyed user with their OWN explicit (non-koma) model: UNCHANGED — the old
    // settings-fields route (their key + model @ DEFAULT_BASE_URL, OpenAI-compatible wire).
    let config = AppConfig::default();
    let settings = Settings {
        api_key: "sk-or-legacy".to_string(),
        model: "openai/gpt-4o".to_string(),
        ..Default::default()
    };

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(resolved.endpoint, crate::config::DEFAULT_BASE_URL);
    assert_eq!(resolved.model_id, "openai/gpt-4o");
    assert_eq!(resolved.api_key, "sk-or-legacy");
}

#[test]
fn reassigned_main_on_real_provider_wins_over_koma_free_entry() {
    // After onboarding, config has a KomaFree provider + a koma/apple Main entry. The user
    // then adds a real (keyed) provider + model and assigns Main to it GLOBALLY; the
    // /settings role-steal removes Main from the koma-free entry (same global scope),
    // leaving it role-less. resolve_role(Main) must return the REAL model — not koma/apple —
    // even though the koma-free entry is still listed FIRST and settings.model is still the
    // koma/apple default. (This is the "configured user assigns Main to a real model → step
    // 2 wins, koma-free never fires" guarantee.)
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-koma".to_string(),
        name: "koma free".to_string(),
        api_type: ApiType::KomaFree,
        endpoint: crate::service::koma_free::KOMA_FREE_ENDPOINT.to_string(),
        api_key: String::new(),
    });
    config.providers.push(ProviderConn {
        uuid: "prov-real".to_string(),
        name: "real".to_string(),
        endpoint: "https://real.example".to_string(),
        api_key: "key-real".to_string(),
        ..ProviderConn::default()
    });
    // koma-free entry FIRST, but Main was stolen away by the reassignment (roles empty now).
    config.models.push(ModelEntry {
        uuid: "model-koma".to_string(),
        name: "koma free".to_string(),
        model_id: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        provider_uuid: "prov-koma".to_string(),
        roles: vec![],
        ..ModelEntry::default()
    });
    // The reassigned Main on the real keyed provider.
    config.models.push(ModelEntry {
        uuid: "model-real".to_string(),
        name: "My Main".to_string(),
        model_id: "vendor/real-model".to_string(),
        provider_uuid: "prov-real".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    let settings = Settings {
        model: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        ..Default::default()
    };

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(resolved.model_id, "vendor/real-model");
    assert_eq!(resolved.endpoint, "https://real.example");
    assert_eq!(resolved.api_key, "key-real");
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_ne!(resolved.model_id, crate::service::koma_free::KOMA_FREE_MODEL);
}

#[test]
fn configured_dangling_main_does_not_force_koma_free() {
    // A CONFIGURED install (real keyed provider + a Main model) whose Main entry points at a
    // DANGLING provider_uuid, with settings.model at the koma/apple default. The OLD guard
    // fired koma-free here (settings.model == KOMA_FREE_MODEL); the fix gates on
    // `is_unconfigured` (false — providers/models are present), so Main falls through to the
    // legacy settings route rather than being silently shadowed by koma-free.
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-real".to_string(),
        name: "real".to_string(),
        endpoint: "https://real.example".to_string(),
        api_key: "key-real".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-main".to_string(),
        name: "Main".to_string(),
        model_id: "vendor/real-model".to_string(),
        provider_uuid: "prov-missing".to_string(), // DANGLING — no such provider.
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    let settings = Settings {
        model: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        ..Default::default()
    };
    assert!(!config.is_unconfigured(&settings));

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    // Falls to legacy_main (settings.model @ DEFAULT_BASE_URL) — NOT the koma-free tier.
    assert_ne!(resolved.api_type, ApiType::KomaFree);
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(resolved.endpoint, crate::config::DEFAULT_BASE_URL);
}

#[test]
fn session_reassigned_main_wins_over_leftover_global_koma_free_main() {
    // Duplicate-Main hazard guard: a LOCAL (session) reassignment CANNOT strip the GLOBAL
    // koma-free entry's Main role (the /settings steal is scope-matched: `other.session_only
    // == draft.session_only`), so BOTH entries hold Main — the koma-free one GLOBAL, the new
    // one SESSION. resolve_role checks `session_models` FIRST, so the real session Main wins
    // and the leftover global koma-free Main never shadows it (which would otherwise force
    // koma/apple via from_entry's KomaFree branch).
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-koma".to_string(),
        name: "koma free".to_string(),
        api_type: ApiType::KomaFree,
        endpoint: crate::service::koma_free::KOMA_FREE_ENDPOINT.to_string(),
        api_key: String::new(),
    });
    config.providers.push(ProviderConn {
        uuid: "prov-real".to_string(),
        name: "real".to_string(),
        endpoint: "https://real.example".to_string(),
        api_key: "key-real".to_string(),
        ..ProviderConn::default()
    });
    // GLOBAL koma-free Main entry — still holds Main (a session reassignment can't strip it).
    config.models.push(ModelEntry {
        uuid: "model-koma".to_string(),
        name: "koma free".to_string(),
        model_id: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        provider_uuid: "prov-koma".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    // SESSION override: Main on the real provider (checked before config.models).
    let settings = Settings {
        model: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        session_models: vec![ModelEntry {
            uuid: "model-sess".to_string(),
            name: "Session Main".to_string(),
            model_id: "vendor/session-model".to_string(),
            provider_uuid: "prov-real".to_string(),
            roles: vec![ModelRole::Main],
            ..ModelEntry::default()
        }],
        ..Default::default()
    };

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
    assert_eq!(resolved.model_id, "vendor/session-model");
    assert_eq!(resolved.endpoint, "https://real.example");
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::app_config::{ModelEntry, ProviderConn};

/// Build a minimal config with one Main model + one Planner model, each on its
/// own provider connection, so `resolve_role` resolves both independently
/// (Planner must never inherit Main's route the way Compactor/Awareness do).
fn config_with(
    main_model: &str,
    main_endpoint: &str,
    planner_model: &str,
    planner_endpoint: &str,
) -> AppConfig {
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
    let config = config_with(
        "main/model",
        "https://main.example",
        "planner/model",
        "https://planner.example",
    );
    let settings = Settings::default();

    let resolved = resolve_turn_model(&config, &settings, AgentMode::Auto).unwrap();
    assert_eq!(resolved.model_id, "main/model");
    assert_eq!(resolved.endpoint, "https://main.example");
}

#[test]
fn plan_mode_with_distinct_planner_uses_planner() {
    let config = config_with(
        "main/model",
        "https://main.example",
        "planner/model",
        "https://planner.example",
    );
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
    let config = config_with(
        "shared/model",
        "https://shared.example",
        "shared/model",
        "https://shared.example",
    );
    let settings = Settings::default();

    let main_only = resolve_role(&config, &settings, ModelRole::Main).unwrap();
    let turn = resolve_turn_model(&config, &settings, AgentMode::Plan).unwrap();

    assert_eq!(turn.model_id, main_only.model_id);
    assert_eq!(turn.endpoint, main_only.endpoint);
    assert_eq!(turn.route, main_only.route);
}

#[test]
fn unassigned_planner_inherits_main() {
    let config = AppConfig::default();
    let settings = Settings::default();
    assert!(same_route(
        &resolve_role_dispatch(&config, &settings, ModelRole::Planner).unwrap(),
        &resolve_role_dispatch(&config, &settings, ModelRole::Main).unwrap()
    ));
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
        source_uuid: None,
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
    assert_eq!(
        resolved.endpoint,
        crate::service::oauth::registry::meta(OAuthProvider::Codex).chat_endpoint
    );
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
    assert_eq!(
        resolved.endpoint,
        crate::service::oauth::registry::meta(OAuthProvider::Kilocode).chat_endpoint
    );
    assert_eq!(resolved.api_key, "kilo-token");
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(resolved.account_id, "org-456");
    assert_eq!(resolved.oauth_uuid, "kilo-uuid");
}

#[test]
fn main_koma_default_with_legacy_key_falls_to_legacy_not_koma_free() {
    // A user WITH a legacy api_key falls to the legacy settings route: even with
    // `settings.model` set to koma/apple, Main resolves to the legacy key + model
    // @ DEFAULT_BASE_URL (OpenAI-compatible wire), never the koma-free tier.
    let config = AppConfig::default();
    let settings = Settings {
        api_key: "sk-or-legacy".to_string(),
        model: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        ..Default::default()
    };

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
        ext_id: None,
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
    assert_ne!(
        resolved.model_id,
        crate::service::koma_free::KOMA_FREE_MODEL
    );
}

#[test]
fn configured_dangling_main_does_not_force_koma_free() {
    // A configured install (real keyed provider + a Main model) whose Main entry points at a
    // DANGLING provider_uuid, with settings.model set to koma/apple. Main can't resolve the
    // assigned entry (dangling provider), so it falls through to the legacy settings route
    // (OpenAI-compatible @ DEFAULT_BASE_URL) rather than the koma-free tier.
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

    assert!(resolve_role_dispatch(&config, &settings, ModelRole::Main).is_none());
    assert_eq!(
        role_resolution(&config, &settings, ModelRole::Main)
            .reason
            .as_deref(),
        Some("provider_missing")
    );
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
        ext_id: None,
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

// ---------------------------------------------------------------------------
// Wave 4: find_model_entry_by_slug + resolve_agent step 1c (slug reference) +
// spawn-override resolution.
// ---------------------------------------------------------------------------

#[test]
fn find_model_entry_by_slug_matches_by_model_id_name_uuid_case_insensitive() {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-1".to_string(),
        name: "prov".to_string(),
        endpoint: "https://example.com".to_string(),
        api_key: "key".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-uuid-1".to_string(),
        name: "My Model".to_string(),
        model_id: "vendor/model-a".to_string(),
        provider_uuid: "prov-1".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let by_id = find_model_entry_by_slug(&config, &settings, "VENDOR/Model-A", None)
        .expect("matches by model_id, case-insensitive");
    assert_eq!(by_id.uuid, "model-uuid-1");

    let by_name = find_model_entry_by_slug(&config, &settings, "my model", None)
        .expect("matches by name, case-insensitive");
    assert_eq!(by_name.uuid, "model-uuid-1");

    let by_uuid = find_model_entry_by_slug(&config, &settings, "MODEL-UUID-1", None)
        .expect("matches by uuid, case-insensitive");
    assert_eq!(by_uuid.uuid, "model-uuid-1");
}

#[test]
fn find_model_entry_by_slug_session_models_win_over_global() {
    let mut config = AppConfig::default();
    config.models.push(ModelEntry {
        uuid: "global-uuid".to_string(),
        name: "Global".to_string(),
        model_id: "shared/slug".to_string(),
        provider_uuid: "prov-global".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings {
        session_models: vec![ModelEntry {
            uuid: "session-uuid".to_string(),
            name: "Session".to_string(),
            model_id: "shared/slug".to_string(),
            provider_uuid: "prov-session".to_string(),
            ..ModelEntry::default()
        }],
        ..Default::default()
    };

    let hit = find_model_entry_by_slug(&config, &settings, "shared/slug", None).expect("matches");
    assert_eq!(
        hit.uuid, "session-uuid",
        "session_models must win over config.models on the same slug"
    );
}

#[test]
fn find_model_entry_by_slug_miss_returns_none() {
    let config = AppConfig::default();
    let settings = Settings::default();
    assert!(find_model_entry_by_slug(&config, &settings, "nonexistent/slug", None).is_none());
}

#[test]
fn find_model_entry_by_slug_preferred_provider_wins_over_earlier_general_match() {
    let mut config = AppConfig::default();
    // Two entries sharing the same slug on DIFFERENT providers; the earlier
    // (insertion-order) general-scan match is "other", but a preferred set
    // steers the lookup to "wanted" instead — the seam a later wave uses to
    // prefer an extension's OWN registered provider.
    config.models.push(ModelEntry {
        uuid: "other-uuid".to_string(),
        name: "Other".to_string(),
        model_id: "shared/slug".to_string(),
        provider_uuid: "prov-other".to_string(),
        ..ModelEntry::default()
    });
    config.models.push(ModelEntry {
        uuid: "wanted-uuid".to_string(),
        name: "Wanted".to_string(),
        model_id: "shared/slug".to_string(),
        provider_uuid: "prov-wanted".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let mut preferred = std::collections::HashSet::new();
    preferred.insert("prov-wanted".to_string());

    let hit = find_model_entry_by_slug(&config, &settings, "shared/slug", Some(&preferred))
        .expect("matches");
    assert_eq!(
        hit.uuid, "wanted-uuid",
        "a preferred provider_uuid must win over the earlier general match"
    );

    // Without the preference, the FIRST general match (insertion order) wins.
    let unpreferred =
        find_model_entry_by_slug(&config, &settings, "shared/slug", None).expect("matches");
    assert_eq!(unpreferred.uuid, "other-uuid");
}

#[test]
fn resolve_agent_step_1c_resolves_slug_reference_with_no_provider_uuid() {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-1".to_string(),
        name: "prov".to_string(),
        endpoint: "https://slug.example".to_string(),
        api_key: "slug-key".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-uuid-1".to_string(),
        name: "Slug Model".to_string(),
        model_id: "vendor/slug-model".to_string(),
        provider_uuid: "prov-1".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let agent = AgentDef {
        model: Some("vendor/slug-model".to_string()),
        ..AgentDef::default()
    };

    let resolved = resolve_agent(&config, &settings, &agent).expect("resolves via slug");
    assert_eq!(resolved.model_id, "vendor/slug-model");
    assert_eq!(resolved.endpoint, "https://slug.example");
    assert_eq!(resolved.api_key, "slug-key");
}

#[test]
fn resolve_agent_slug_miss_falls_to_main_and_agent_model_resolves_is_false() {
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

    let agent = AgentDef {
        model: Some("nonexistent/slug".to_string()),
        ..AgentDef::default()
    };

    let resolved = resolve_agent(&config, &settings, &agent).expect("falls to Main");
    assert_eq!(
        resolved.model_id, "main/model",
        "unresolved slug falls to Main"
    );

    assert!(
        agent_declares_model(&agent),
        "agent declares a model (even though it won't resolve)"
    );
    assert!(
        !agent_model_resolves(&config, &settings, &agent),
        "an unresolvable slug must report false so the caller's toast fires"
    );
}

/// Mirrors [`crate::app::subagent::spawn::spawn_subagent`]'s override-application
/// (clone the def, replace `model`, clear `model_uuid`/`provider_uuid`) so the
/// resolution behavior an override produces is tested at the `resolve_agent`
/// level without standing up the full spawn/state plumbing.
#[test]
fn spawn_override_model_replaces_agent_model_at_resolution() {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-a".to_string(),
        name: "prov-a".to_string(),
        endpoint: "https://a.example".to_string(),
        api_key: "key-a".to_string(),
        ..ProviderConn::default()
    });
    config.providers.push(ProviderConn {
        uuid: "prov-b".to_string(),
        name: "prov-b".to_string(),
        endpoint: "https://b.example".to_string(),
        api_key: "key-b".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-a".to_string(),
        name: "Model A".to_string(),
        model_id: "vendor/model-a".to_string(),
        provider_uuid: "prov-a".to_string(),
        ..ModelEntry::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-b".to_string(),
        name: "Model B".to_string(),
        model_id: "vendor/model-b".to_string(),
        provider_uuid: "prov-b".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let agent = AgentDef {
        model: Some("vendor/model-a".to_string()),
        ..AgentDef::default()
    };

    let mut overridden = agent.clone();
    overridden.model = Some("vendor/model-b".to_string());
    overridden.model_uuid = None;
    overridden.provider_uuid = None;

    let resolved =
        resolve_agent(&config, &settings, &overridden).expect("resolves via override slug");
    assert_eq!(
        resolved.model_id, "vendor/model-b",
        "override model wins over the agent's own"
    );
    assert_eq!(resolved.endpoint, "https://b.example");
}

#[test]
fn spawn_override_effort_only_leaves_model_untouched() {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-a".to_string(),
        name: "prov-a".to_string(),
        endpoint: "https://a.example".to_string(),
        api_key: "key-a".to_string(),
        ..ProviderConn::default()
    });
    config.models.push(ModelEntry {
        uuid: "model-a".to_string(),
        name: "Model A".to_string(),
        model_id: "vendor/model-a".to_string(),
        provider_uuid: "prov-a".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let agent = AgentDef {
        model: Some("vendor/model-a".to_string()),
        ..AgentDef::default()
    };

    // Only `effort` overridden — model left as the agent's own.
    let mut overridden = agent.clone();
    overridden.effort = Some("max".to_string());

    let resolved = resolve_agent(&config, &settings, &overridden).expect("resolves");
    assert_eq!(
        resolved.model_id, "vendor/model-a",
        "model untouched by an effort-only override"
    );
    assert_eq!(resolved.effort, "max", "effort replaced by the override");
}

/// Mirrors `spawn_task_with_id`'s mismatch-warning check-clone: an override
/// slug that names nothing registered must fall to Main AND make the
/// `agent_declares_model && !agent_model_resolves` warning predicate true.
#[test]
fn spawn_override_garbage_slug_falls_to_main_and_warns() {
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

    // Agent declares no model of its own; only the (garbage) override supplies one.
    let agent = AgentDef::default();
    let mut check_agent = agent.clone();
    check_agent.model = Some("totally/bogus-slug".to_string());
    check_agent.model_uuid = None;
    check_agent.provider_uuid = None;

    let resolved = resolve_agent(&config, &settings, &check_agent).expect("falls to Main");
    assert_eq!(resolved.model_id, "main/model");

    assert!(
        agent_declares_model(&check_agent),
        "override slug counts as a declared model"
    );
    assert!(
        !agent_model_resolves(&config, &settings, &check_agent),
        "a garbage override slug must fail to resolve so the mismatch warning fires"
    );
}

// ---------------------------------------------------------------------------
// Wave 12: data-driven extension-backed provider resolution + ext-scoped
// deterministic sub-agent model binding.
// ---------------------------------------------------------------------------

/// An ext-backed [`OAuthConn`] carrying the W12 model-provider meta (`ext_id` +
/// `chat_endpoint` + `api_type`), so a `ModelEntry` pointing at it resolves as a real
/// provider.
fn ext_model_conn(uuid: &str, ext_id: &str, endpoint: &str, api_type: &str) -> OAuthConn {
    OAuthConn {
        uuid: uuid.to_string(),
        provider: OAuthProvider::Extension,
        access_token: "ext-bearer".to_string(),
        ext_id: Some(ext_id.to_string()),
        provider_id: Some("prov".to_string()),
        chat_endpoint: Some(endpoint.to_string()),
        api_type: Some(api_type.to_string()),
        ..Default::default()
    }
}

#[test]
fn ext_conn_with_meta_resolves_data_driven_openai() {
    // A ModelEntry served by an ext conn whose manifest declared an OpenAI-compatible chat
    // endpoint resolves to that endpoint + bearer, wire type OpenAiCompatible, threading the
    // conn uuid as oauth_uuid for the send-time refresh hook.
    let mut config = AppConfig::default();
    config.oauth_conns.push(ext_model_conn(
        "ext-conn",
        "my.ext",
        "https://api.ext.test/v1",
        "openai",
    ));
    config.models.push(ModelEntry {
        uuid: "ext-model".to_string(),
        name: "Ext Model".to_string(),
        model_id: "ext/model".to_string(),
        provider_uuid: "ext-conn".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("ext conn resolves");
    assert_eq!(resolved.endpoint, "https://api.ext.test/v1");
    assert_eq!(resolved.api_key, "ext-bearer");
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(
        resolved.account_id, "",
        "ext conns carry no account/org header"
    );
    assert_eq!(
        resolved.oauth_uuid, "ext-conn",
        "conn uuid threads through for refresh"
    );
    // The Conn projection carries the same identity to the call boundary.
    let conn = resolved.conn();
    assert_eq!(conn.endpoint, "https://api.ext.test/v1");
    assert_eq!(conn.oauth_uuid, "ext-conn");
    assert!(matches!(conn.api_type, ApiType::OpenAiCompatible));
}

#[test]
fn ext_conn_with_meta_resolves_data_driven_anthropic() {
    // The "anthropic" wire maps to AnthropicCompatible.
    let mut config = AppConfig::default();
    config.oauth_conns.push(ext_model_conn(
        "ext-conn",
        "my.ext",
        "https://api.ext.test",
        "anthropic",
    ));
    config.models.push(ModelEntry {
        uuid: "ext-model".to_string(),
        name: "Ext Model".to_string(),
        model_id: "ext/model".to_string(),
        provider_uuid: "ext-conn".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    let resolved = from_entry(&config, &settings, &config.models[0], ModelRole::Main)
        .expect("ext conn resolves");
    assert_eq!(resolved.endpoint, "https://api.ext.test");
    assert_eq!(resolved.api_type, ApiType::AnthropicCompatible);
    assert_eq!(resolved.oauth_uuid, "ext-conn");
}

#[test]
fn ext_conn_without_meta_is_not_a_model_provider() {
    // An ext conn with NO chat_endpoint/api_type (account-login-only) is inert: from_entry
    // returns None (the W11 "not a model provider yet" behavior), so a referencing entry is
    // treated as dangling and resolve_role falls to the legacy fallback rather than routing a
    // broken empty-endpoint ext route.
    let mut config = AppConfig::default();
    config.oauth_conns.push(OAuthConn {
        uuid: "ext-login-only".to_string(),
        provider: OAuthProvider::Extension,
        access_token: "ext-bearer".to_string(),
        ext_id: Some("my.ext".to_string()),
        provider_id: Some("prov".to_string()),
        // No chat_endpoint / api_type → account-login-only.
        ..Default::default()
    });
    let entry = ModelEntry {
        uuid: "ext-model".to_string(),
        name: "Ext Model".to_string(),
        model_id: "ext/model".to_string(),
        provider_uuid: "ext-login-only".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    };
    config.models.push(entry.clone());
    let settings = Settings::default();

    // Direct: from_entry refuses the meta-less ext conn.
    assert!(
        from_entry(&config, &settings, &entry, ModelRole::Main).is_none(),
        "a meta-less ext conn is not a model provider (W11 inert stance preserved)"
    );
    assert!(resolve_role_dispatch(&config, &settings, ModelRole::Main).is_none());
    assert_eq!(
        role_resolution(&config, &settings, ModelRole::Main)
            .reason
            .as_deref(),
        Some("unsupported_route")
    );
}

#[test]
fn ext_agent_binds_to_its_own_model_over_same_named_global() {
    // Deterministic ext-scoped binding: a GLOBAL entry and an EXT-OWNED entry share the slug
    // "fast" (by name). An AgentDef authored by the extension (ext_id set) binds to the
    // extension's OWN entry (served by its oauth conn) FIRST — uuid-deterministic, so the
    // same-named global entry (inserted FIRST) can't hijack it. A non-ext agent gets the
    // general first-match (the global). An ext agent whose extension owns no matching
    // provider falls through to the general pass.
    let mut config = AppConfig::default();
    // The extension's connected model-provider conn.
    config.oauth_conns.push(ext_model_conn(
        "ext-conn",
        "my.ext",
        "https://api.ext.test/v1",
        "openai",
    ));
    // A real user provider for the global entry.
    config.providers.push(ProviderConn {
        uuid: "prov-x".to_string(),
        name: "X".to_string(),
        endpoint: "https://x.example".to_string(),
        api_key: "x-key".to_string(),
        ..ProviderConn::default()
    });
    // GLOBAL "fast" FIRST (so it is the first general match), then the EXT-owned "fast".
    config.models.push(ModelEntry {
        uuid: "global-fast".to_string(),
        name: "fast".to_string(),
        model_id: "global/fast-model".to_string(),
        provider_uuid: "prov-x".to_string(),
        ..ModelEntry::default()
    });
    config.models.push(ModelEntry {
        uuid: "ext-fast".to_string(),
        name: "fast".to_string(),
        model_id: "ext/fast-model".to_string(),
        provider_uuid: "ext-conn".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();

    // Ext-authored agent → binds to the EXT entry (its own provider), NOT the earlier global.
    let ext_agent = AgentDef {
        ext_id: Some("my.ext".to_string()),
        model: Some("fast".to_string()),
        ..AgentDef::default()
    };
    let resolved = resolve_agent(&config, &settings, &ext_agent).expect("resolves");
    assert_eq!(
        resolved.model_id, "ext/fast-model",
        "ext agent binds to its OWN 'fast'"
    );
    assert_eq!(resolved.endpoint, "https://api.ext.test/v1");
    assert_eq!(resolved.oauth_uuid, "ext-conn");
    assert!(
        agent_model_resolves(&config, &settings, &ext_agent),
        "the ext binding resolves (no spurious mismatch warning)"
    );

    // Non-ext agent → general first-match (the global "fast").
    let plain_agent = AgentDef {
        model: Some("fast".to_string()),
        ..AgentDef::default()
    };
    let plain = resolve_agent(&config, &settings, &plain_agent).expect("resolves");
    assert_eq!(
        plain.model_id, "global/fast-model",
        "a non-ext agent takes the general match"
    );
    assert_eq!(plain.endpoint, "https://x.example");

    // Ext agent whose extension owns NO conn → empty preferred set → general pass (the global).
    let orphan_agent = AgentDef {
        ext_id: Some("other.ext".to_string()),
        model: Some("fast".to_string()),
        ..AgentDef::default()
    };
    let orphan = resolve_agent(&config, &settings, &orphan_agent).expect("resolves");
    assert_eq!(
        orphan.model_id, "global/fast-model",
        "preferred-set present but no match falls to the general pass"
    );
}

// ---------------------------------------------------------------------------
// main_fallback_reason — free-tier auto-route diagnosis
// ---------------------------------------------------------------------------

#[test]
fn main_fallback_unconfigured_when_no_main_and_no_usable_legacy() {
    // Empty catalogue + empty legacy key → dispatch will auto-route to koma/apple.
    let config = AppConfig::default();
    let settings = Settings::default(); // model = DEFAULT_MODEL, api_key empty
    assert_eq!(
        main_fallback_reason(&config, &settings),
        Some(MainFallback::Unconfigured)
    );
    // And dispatch itself lands on koma/apple.
    let d = resolve_role_dispatch(&config, &settings, ModelRole::Main).expect("dispatch");
    assert_eq!(d.model_id, crate::service::koma_free::KOMA_FREE_MODEL);
    assert_eq!(d.api_type, ApiType::KomaFree);
}

#[test]
fn main_fallback_none_when_user_explicitly_chose_koma_free() {
    // A deliberate koma-free Main is usable — no fallback warning.
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "prov-koma".to_string(),
        name: "koma free".to_string(),
        api_type: ApiType::KomaFree,
        endpoint: crate::service::koma_free::KOMA_FREE_ENDPOINT.to_string(),
        api_key: String::new(),
        ext_id: None,
    });
    config.models.push(ModelEntry {
        uuid: "model-koma".to_string(),
        name: "koma free".to_string(),
        model_id: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        provider_uuid: "prov-koma".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    let settings = Settings::default();
    assert_eq!(main_fallback_reason(&config, &settings), None);
}

#[test]
fn main_fallback_provider_removed_when_main_provider_dangling() {
    let mut config = AppConfig::default();
    config.models.push(ModelEntry {
        uuid: "model-main".to_string(),
        name: "Main".to_string(),
        model_id: "vendor/x".to_string(),
        provider_uuid: "gone".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    // Empty legacy key so resolve_role is unusable → dispatch substitutes.
    let settings = Settings::default();
    assert_eq!(
        main_fallback_reason(&config, &settings),
        Some(MainFallback::ProviderRemoved)
    );
}

#[test]
fn live_session_reference_follows_source_but_keeps_own_role() {
    let mut config = config_with(
        "old",
        "https://main.example",
        "planner",
        "https://planner.example",
    );
    let mut settings = Settings::default();
    crate::model::app_config::assign_role(
        &mut settings.session_models,
        ModelRole::Safeguard,
        Some(&config.models[0]),
    );
    config.models[0].model_id = "edited".into();
    config.models[0].provider_uuid = "prov-planner".into();
    config.models[0].roles.clear();
    let r = resolve_role_dispatch(&config, &settings, ModelRole::Safeguard).unwrap();
    assert_eq!(r.model_id, "edited");
    assert_eq!(r.endpoint, "https://planner.example");
    assert_eq!(settings.session_models[0].roles, vec![ModelRole::Safeguard]);
    config.models.remove(0);
    let report = role_resolution(&config, &settings, ModelRole::Safeguard);
    assert_eq!(report.reason.as_deref(), Some("reference_deleted"));
    assert!(report.effective_model.is_none());
}

#[test]
fn connection_identity_includes_account_api_provider_and_pin() {
    let cfg = config_with(
        "same",
        "https://same.example",
        "same",
        "https://same.example",
    );
    let r = resolve_role(&cfg, &Settings::default(), ModelRole::Main).unwrap();
    for variant in 0..4 {
        let mut other = r.clone();
        match variant {
            0 => other.account_id = "other-account".into(),
            1 => other.api_type = ApiType::AnthropicCompatible,
            2 => other.provider_uuid = "other-connection".into(),
            _ => other.route = Some("other-upstream".into()),
        }
        assert!(!same_route(&r, &other));
    }
    let mut rotated = r.clone();
    rotated.api_key = "rotated-secret".into();
    assert!(same_route(&r, &rotated));
}

#[test]
fn broken_session_owner_never_silently_uses_global_or_default() {
    let config = config_with(
        "main",
        "https://main.example",
        "planner",
        "https://planner.example",
    );
    let mut settings = Settings::default();
    settings.session_models.push(ModelEntry {
        roles: vec![ModelRole::Main],
        provider_uuid: "missing".into(),
        ..Default::default()
    });
    assert!(resolve_role_dispatch(&config, &settings, ModelRole::Main).is_none());
    let report = role_resolution(&config, &settings, ModelRole::Main);
    assert_eq!(report.origin, "session");
    assert_eq!(report.reason.as_deref(), Some("provider_missing"));
}

#[test]
fn dedicated_legacy_awareness_precedes_inherited_main() {
    let config = AppConfig::default();
    let settings = Settings {
        api_key: "legacy-key".into(),
        awareness_inherit: false,
        awareness_model: "legacy-aware".into(),
        awareness_provider: "upstream".into(),
        ..Default::default()
    };
    let route = resolve_role_dispatch(&config, &settings, ModelRole::Awareness).unwrap();
    assert_eq!(route.model_id, "legacy-aware");
    assert_eq!(route.provider(), "upstream");
}

fn commandcode_oauth_config(model_id: &str, commandcode_chat: Option<&str>) -> AppConfig {
    let conn = OAuthConn {
        uuid: "cc-uuid".to_string(),
        provider: OAuthProvider::CommandCode,
        access_token: "cc-token".to_string(),
        commandcode_chat: commandcode_chat.map(str::to_string),
        ..Default::default()
    };
    let mut config = AppConfig::default();
    config.oauth_conns.push(conn);
    config.models.push(ModelEntry {
        uuid: "model-uuid".to_string(),
        name: "test".to_string(),
        model_id: model_id.to_string(),
        provider_uuid: "cc-uuid".to_string(),
        route: None,
        roles: vec![ModelRole::Main],
        role: None,
        source_uuid: None,
    });
    config
}

#[test]
fn commandcode_claude_model_uses_anthropic_messages() {
    let config = commandcode_oauth_config("claude-haiku-5-5", None);
    let resolved = resolve_role(&config, &Settings::default(), ModelRole::Main).unwrap();
    assert_eq!(resolved.api_type, ApiType::AnthropicCompatible);
    assert_eq!(
        resolved.endpoint,
        crate::service::oauth::registry::COMMANDCODE_API_BASE
    );
}

#[test]
fn commandcode_gpt_model_stays_openai() {
    let config = commandcode_oauth_config("gpt-6-sol", None);
    let resolved = resolve_role(&config, &Settings::default(), ModelRole::Main).unwrap();
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(
        resolved.endpoint,
        crate::service::oauth::registry::COMMANDCODE_API_BASE
    );
}

#[test]
fn commandcode_ndjson_pref_wins_over_claude_model() {
    let config = commandcode_oauth_config("claude-haiku-5-5", Some("ndjson"));
    let resolved = resolve_role(&config, &Settings::default(), ModelRole::Main).unwrap();
    assert_eq!(resolved.api_type, ApiType::CommandCode);
    assert_eq!(
        resolved.endpoint,
        crate::service::oauth::registry::COMMANDCODE_CHAT_BASE
    );
}

#[test]
fn kilocode_claude_model_stays_openai() {
    let conn = OAuthConn {
        uuid: "kilo-uuid".to_string(),
        provider: OAuthProvider::Kilocode,
        access_token: "kilo-token".to_string(),
        org_id: "org-456".to_string(),
        ..Default::default()
    };
    let mut config = AppConfig::default();
    config.oauth_conns.push(conn);
    config.models.push(ModelEntry {
        uuid: "model-uuid".to_string(),
        name: "test".to_string(),
        model_id: "anthropic/claude-haiku-5-5".to_string(),
        provider_uuid: "kilo-uuid".to_string(),
        route: None,
        roles: vec![ModelRole::Main],
        role: None,
        source_uuid: None,
    });
    let resolved = resolve_role(&config, &Settings::default(), ModelRole::Main).unwrap();
    assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
    assert_eq!(
        resolved.endpoint,
        crate::service::oauth::registry::meta(OAuthProvider::Kilocode).chat_endpoint
    );
}

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
    assert_eq!(hit.uuid, "session-uuid", "session_models must win over config.models on the same slug");
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
    assert_eq!(hit.uuid, "wanted-uuid", "a preferred provider_uuid must win over the earlier general match");

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
    assert_eq!(resolved.model_id, "main/model", "unresolved slug falls to Main");

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
    assert_eq!(resolved.model_id, "vendor/model-b", "override model wins over the agent's own");
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
    assert_eq!(resolved.model_id, "vendor/model-a", "model untouched by an effort-only override");
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

    assert!(agent_declares_model(&check_agent), "override slug counts as a declared model");
    assert!(
        !agent_model_resolves(&config, &settings, &check_agent),
        "a garbage override slug must fail to resolve so the mismatch warning fires"
    );
}

//! Pure catalogue write for SDK `configure`.
//!
//! No daemon or disk access. Callers save, and the live path re-pins Main from
//! [`Applied::main_uuid`]. Provider `api_type` is set here because
//! [`AppConfig::upsert_provider`] forces OpenAI-compatible on create and will
//! not change the wire type on edit.

use crate::model::app_config::{
    new_uuid, upsert_model_entry, ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn,
};
use serde_json::{Map, Value};

const MAX_PROVIDERS: usize = 32;
const MAX_NAME_LEN: usize = 200;
const MAX_ENDPOINT_LEN: usize = 2048;
const MAX_KEY_LEN: usize = 4096;
const MAX_MODEL_LEN: usize = 200;

#[derive(Debug)]
pub(crate) struct Applied {
    pub reply: Value,
    /// Global catalogue uuid of `models.main` when this call assigned it.
    pub main_uuid: Option<String>,
}

pub(crate) fn has_main(config: &AppConfig) -> bool {
    config
        .models
        .iter()
        .any(|model| model.effective_roles().contains(&ModelRole::Main))
}

/// Apply `params` to `config`. `require_main` is set by the unbound caller when
/// the catalogue has no Main yet. A live session passes `false`.
pub(crate) fn apply(
    config: &mut AppConfig,
    params: &Value,
    require_main: bool,
) -> Result<Applied, String> {
    let parsed = parse(params, require_main)?;
    commit(config, &parsed)?;
    let reply = describe(config, &parsed)?;
    let main_uuid = reply
        .get("models")
        .and_then(|models| models.get("main"))
        .and_then(|main| main.get("uuid"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(Applied { reply, main_uuid })
}

/// Idempotent lookup of the reply for a call that has already been applied.
/// Does not mutate `config`.
pub(crate) fn reply(config: &AppConfig, params: &Value) -> Result<Value, String> {
    describe(config, &parse(params, false)?)
}

struct ProviderIn {
    name: String,
    endpoint: String,
    api_type: ApiType,
    /// `None` keeps the stored key on update. A new provider must have `Some`.
    key: Option<String>,
}

struct Assignment {
    role: ModelRole,
    provider: String,
    model_id: String,
    route: Option<String>,
}

struct Parsed {
    providers: Vec<ProviderIn>,
    models: Vec<Assignment>,
}

fn parse(params: &Value, require_main: bool) -> Result<Parsed, String> {
    let obj = params
        .as_object()
        .ok_or_else(|| "params must be an object".to_string())?;
    let providers = match obj.get("providers") {
        None => Vec::new(),
        Some(Value::Array(items)) => parse_providers(items)?,
        Some(_) => return Err("providers must be an array".to_string()),
    };
    let models = match obj.get("models") {
        None => Vec::new(),
        Some(Value::Object(entries)) => parse_models(entries)?,
        Some(_) => return Err("models must be an object".to_string()),
    };
    if providers.is_empty() && models.is_empty() {
        return Err("providers or models required".to_string());
    }
    if require_main && !models.iter().any(|item| item.role == ModelRole::Main) {
        return Err("models.main is required".to_string());
    }
    Ok(Parsed { providers, models })
}

fn parse_providers(items: &[Value]) -> Result<Vec<ProviderIn>, String> {
    if items.len() > MAX_PROVIDERS {
        return Err("too many providers".to_string());
    }
    let mut out = Vec::with_capacity(items.len());
    let mut seen = Vec::with_capacity(items.len());
    for item in items {
        let obj = item
            .as_object()
            .ok_or_else(|| "provider must be an object".to_string())?;
        let name = required_str(obj, "name")?.trim();
        if name.is_empty() {
            return Err("provider name is empty".to_string());
        }
        if name.len() > MAX_NAME_LEN {
            return Err("provider name is too long".to_string());
        }
        if seen.iter().any(|existing: &String| existing == name) {
            return Err(format!("duplicate provider name '{name}'"));
        }
        let endpoint = required_str(obj, "endpoint")?.trim();
        if endpoint.len() > MAX_ENDPOINT_LEN {
            return Err("endpoint is too long".to_string());
        }
        if !is_http_url(endpoint) {
            return Err("endpoint must be an http(s) URL".to_string());
        }
        let api_type = match required_str(obj, "api_type")?.trim() {
            "openai" => ApiType::OpenAiCompatible,
            "anthropic" => ApiType::AnthropicCompatible,
            _ => return Err("api_type must be 'openai' or 'anthropic'".to_string()),
        };
        let key = match obj.get("api_key") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err("api_key must be non-empty".to_string());
                }
                if trimmed.len() > MAX_KEY_LEN {
                    return Err("api_key is too long".to_string());
                }
                Some(trimmed.to_string())
            }
            Some(_) => return Err("api_key must be a string".to_string()),
        };
        seen.push(name.to_string());
        out.push(ProviderIn {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            api_type,
            key,
        });
    }
    Ok(out)
}

fn parse_models(entries: &Map<String, Value>) -> Result<Vec<Assignment>, String> {
    let mut out = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let role = parse_role(key).ok_or_else(|| format!("unknown model role '{key}'"))?;
        let obj = value
            .as_object()
            .ok_or_else(|| "model assignment must be an object".to_string())?;
        let provider = required_str(obj, "provider")?.trim();
        if provider.is_empty() || provider.len() > MAX_NAME_LEN {
            return Err("model provider is invalid".to_string());
        }
        let model_id = required_str(obj, "model_id")?.trim();
        if model_id.is_empty() {
            return Err("model_id is empty".to_string());
        }
        if model_id.len() > MAX_MODEL_LEN {
            return Err("model_id is too long".to_string());
        }
        let route = match obj.get("route") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => {
                if value.len() > MAX_MODEL_LEN {
                    return Err("route is too long".to_string());
                }
                ModelEntry::normalize_route(Some(value.clone()))
            }
            Some(_) => return Err("route must be a string".to_string()),
        };
        out.push(Assignment {
            role,
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            route,
        });
    }
    Ok(out)
}

fn commit(config: &mut AppConfig, parsed: &Parsed) -> Result<(), String> {
    for provider in &parsed.providers {
        if owned_by_extension(config, &provider.name) {
            return Err(format!(
                "provider '{}' is owned by an extension",
                provider.name
            ));
        }
        if user_provider(config, &provider.name).is_none() && provider.key.is_none() {
            return Err(format!("provider '{}' requires an api_key", provider.name));
        }
    }
    for assignment in &parsed.models {
        let in_call = parsed
            .providers
            .iter()
            .any(|provider| provider.name == assignment.provider);
        if owned_by_extension(config, &assignment.provider) && !in_call {
            return Err(format!(
                "provider '{}' is owned by an extension",
                assignment.provider
            ));
        }
        if !in_call && user_provider(config, &assignment.provider).is_none() {
            return Err(format!("unknown provider '{}'", assignment.provider));
        }
    }

    for provider in &parsed.providers {
        if let Some(slot) = config
            .providers
            .iter_mut()
            .find(|entry| entry.ext_id.is_none() && entry.name == provider.name)
        {
            slot.endpoint = provider.endpoint.clone();
            slot.api_type = provider.api_type;
            if let Some(key) = &provider.key {
                slot.api_key = key.clone();
            }
        } else {
            let key = provider
                .key
                .clone()
                .ok_or_else(|| format!("provider '{}' requires an api_key", provider.name))?;
            config.providers.push(ProviderConn {
                uuid: new_uuid(),
                name: provider.name.clone(),
                api_type: provider.api_type,
                endpoint: provider.endpoint.clone(),
                api_key: key,
                ext_id: None,
            });
        }
    }

    let claimed: Vec<ModelRole> = parsed.models.iter().map(|item| item.role).collect();
    let mut groups: Vec<Group> = Vec::new();
    for assignment in &parsed.models {
        let provider_uuid = user_provider(config, &assignment.provider)
            .map(|provider| provider.uuid.clone())
            .ok_or_else(|| format!("unknown provider '{}'", assignment.provider))?;
        if let Some(group) = groups.iter_mut().find(|group| {
            group.provider_uuid == provider_uuid
                && group.model_id == assignment.model_id
                && group.route == assignment.route
        }) {
            group.roles.push(assignment.role);
        } else {
            groups.push(Group {
                provider_uuid,
                model_id: assignment.model_id.clone(),
                route: assignment.route.clone(),
                roles: vec![assignment.role],
            });
        }
    }

    // Role math uses the pre-image. A later upsert steals claimed roles off
    // entries this call does not replace, and those entries keep the rest.
    let original = config.models.clone();
    for group in groups {
        let existing = original.iter().find(|model| {
            model.provider_uuid == group.provider_uuid
                && model.model_id == group.model_id
                && ModelEntry::normalize_route(model.route.clone()) == group.route
        });
        let mut roles: Vec<ModelRole> = existing
            .map(ModelEntry::effective_roles)
            .unwrap_or_default()
            .into_iter()
            .filter(|role| !claimed.contains(role))
            .collect();
        for role in &group.roles {
            if !roles.contains(role) {
                roles.push(*role);
            }
        }
        roles.sort_by_key(|role| role_rank(*role));
        upsert_model_entry(
            &mut config.models,
            ModelEntry {
                uuid: existing
                    .map(|model| model.uuid.clone())
                    .unwrap_or_else(new_uuid),
                name: existing
                    .map(|model| {
                        if model.name.is_empty() {
                            group.model_id.clone()
                        } else {
                            model.name.clone()
                        }
                    })
                    .unwrap_or_else(|| group.model_id.clone()),
                model_id: group.model_id.clone(),
                provider_uuid: group.provider_uuid.clone(),
                route: group.route.clone(),
                roles,
                role: None,
                source_uuid: existing.and_then(|model| model.source_uuid.clone()),
            },
        );
    }
    Ok(())
}

struct Group {
    provider_uuid: String,
    model_id: String,
    route: Option<String>,
    roles: Vec<ModelRole>,
}

fn describe(config: &AppConfig, parsed: &Parsed) -> Result<Value, String> {
    let mut providers = Vec::with_capacity(parsed.providers.len());
    for provider in &parsed.providers {
        let saved = user_provider(config, &provider.name)
            .ok_or_else(|| format!("provider '{}' missing after configure", provider.name))?;
        providers.push(serde_json::json!({
            "name": provider.name,
            "uuid": saved.uuid,
        }));
    }
    let mut models = Map::new();
    for assignment in &parsed.models {
        let provider = user_provider(config, &assignment.provider)
            .ok_or_else(|| format!("provider '{}' missing after configure", assignment.provider))?;
        let model = config
            .models
            .iter()
            .find(|model| {
                model.provider_uuid == provider.uuid
                    && model.model_id == assignment.model_id
                    && ModelEntry::normalize_route(model.route.clone()) == assignment.route
                    && model.effective_roles().contains(&assignment.role)
            })
            .ok_or_else(|| {
                format!(
                    "model for role '{}' missing after configure",
                    role_name(assignment.role)
                )
            })?;
        models.insert(
            role_name(assignment.role).to_string(),
            serde_json::json!({
                "uuid": model.uuid,
                "model_id": model.model_id,
                "provider": provider.name,
            }),
        );
    }
    Ok(serde_json::json!({
        "providers": providers,
        "models": models,
    }))
}

fn user_provider<'a>(config: &'a AppConfig, name: &str) -> Option<&'a ProviderConn> {
    config
        .providers
        .iter()
        .find(|provider| provider.ext_id.is_none() && provider.name == name)
}

fn owned_by_extension(config: &AppConfig, name: &str) -> bool {
    config
        .providers
        .iter()
        .any(|provider| provider.ext_id.is_some() && provider.name == name)
}

fn required_str<'a>(obj: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    obj.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} must be a string"))
}

fn parse_role(name: &str) -> Option<ModelRole> {
    Some(match name {
        "main" => ModelRole::Main,
        "awareness" => ModelRole::Awareness,
        "safeguard" => ModelRole::Safeguard,
        "compactor" => ModelRole::Compactor,
        "planner" => ModelRole::Planner,
        "cyber-red" | "cyber_red" => ModelRole::CyberRed,
        "cyber-blue" | "cyber_blue" => ModelRole::CyberBlue,
        "cyber-research" | "cyber_research" | "cyber-researcher" | "cyber_researcher" => {
            ModelRole::CyberResearch
        }
        _ => return None,
    })
}

fn role_name(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
        ModelRole::Planner => "planner",
        ModelRole::CyberRed => "cyber-red",
        ModelRole::CyberBlue => "cyber-blue",
        ModelRole::CyberResearch => "cyber-researcher",
    }
}

fn role_rank(role: ModelRole) -> u8 {
    match role {
        ModelRole::Main => 0,
        ModelRole::Awareness => 1,
        ModelRole::Safeguard => 2,
        ModelRole::Compactor => 3,
        ModelRole::Planner => 4,
        // Cyber last: these are the most specific, so a generic assignment is
        // the weakest and must not win a ranking tie.
        ModelRole::CyberRed => 5,
        ModelRole::CyberBlue => 6,
        ModelRole::CyberResearch => 7,
    }
}

fn is_http_url(endpoint: &str) -> bool {
    match url::Url::parse(endpoint) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https"),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::app_config::assign_role;
    use serde_json::json;

    const SECRET: &str = "sk-test-secret-value";

    fn provider(name: &str, endpoint: &str, api_type: &str, key: Option<&str>) -> Value {
        let mut value = json!({
            "name": name,
            "endpoint": endpoint,
            "api_type": api_type,
        });
        if let Some(key) = key {
            value["api_key"] = json!(key);
        }
        value
    }

    fn two_providers(main_key: Option<&str>) -> Value {
        let mut openai = provider(
            "openai",
            "https://api.openai.com/v1",
            "openai",
            main_key.or(Some(SECRET)),
        );
        if main_key.is_none() && openai.get("api_key").is_some() {
            openai.as_object_mut().unwrap().remove("api_key");
        }
        json!({
            "providers": [
                openai,
                provider("anthropic", "https://api.anthropic.com", "anthropic", Some("sk-ant-test")),
            ],
            "models": {
                "main": {"provider": "openai", "model_id": "gpt-4.1"},
                "awareness": {"provider": "openai", "model_id": "gpt-4.1-mini"},
                "safeguard": {"provider": "anthropic", "model_id": "claude-sonnet-4-5"},
                "compactor": {"provider": "openai", "model_id": "gpt-4.1-mini"},
                "planner": {"provider": "anthropic", "model_id": "claude-sonnet-4-5"},
            }
        })
    }

    fn assert_unchanged(before: &AppConfig, after: &AppConfig) {
        assert_eq!(before.providers, after.providers);
        assert_eq!(before.models, after.models);
    }

    #[test]
    fn two_providers_share_entries_per_model_and_hide_keys() {
        let mut config = AppConfig::default();
        let applied = apply(&mut config, &two_providers(Some(SECRET)), true).unwrap();
        let body = applied.reply.to_string();
        assert!(!body.contains(SECRET));
        assert!(!body.contains("api_key"));
        assert_eq!(config.providers.len(), 2);
        assert_eq!(config.providers[0].api_type, ApiType::OpenAiCompatible);
        assert_eq!(config.providers[1].api_type, ApiType::AnthropicCompatible);
        assert!(config
            .providers
            .iter()
            .all(|provider| provider.ext_id.is_none()));
        let main = config
            .models
            .iter()
            .find(|model| model.effective_roles().contains(&ModelRole::Main))
            .unwrap();
        assert_eq!(main.model_id, "gpt-4.1");
        assert!(!main.effective_roles().contains(&ModelRole::Compactor));
        let shared = config
            .models
            .iter()
            .find(|model| model.model_id == "gpt-4.1-mini")
            .unwrap();
        assert!(shared.effective_roles().contains(&ModelRole::Awareness));
        assert!(shared.effective_roles().contains(&ModelRole::Compactor));
        let safeguard = config
            .models
            .iter()
            .find(|model| model.effective_roles().contains(&ModelRole::Safeguard))
            .unwrap();
        assert_eq!(safeguard.model_id, "claude-sonnet-4-5");
        assert!(safeguard.effective_roles().contains(&ModelRole::Planner));
        assert_eq!(applied.main_uuid.as_deref(), Some(main.uuid.as_str()));
        assert_eq!(
            reply(&config, &two_providers(Some(SECRET))).unwrap(),
            applied.reply
        );
    }

    #[test]
    fn same_name_update_keeps_uuid_and_omitted_key() {
        let mut config = AppConfig::default();
        apply(&mut config, &two_providers(Some(SECRET)), true).unwrap();
        let uuid = config.providers[0].uuid.clone();
        let update = json!({
            "providers": [{
                "name": "openai",
                "endpoint": "https://proxy.example/v1",
                "api_type": "anthropic",
            }]
        });
        let applied = apply(&mut config, &update, false).unwrap();
        assert_eq!(config.providers[0].uuid, uuid);
        assert_eq!(config.providers[0].api_key, SECRET);
        assert_eq!(config.providers[0].endpoint, "https://proxy.example/v1");
        assert_eq!(config.providers[0].api_type, ApiType::AnthropicCompatible);
        assert_eq!(applied.reply["providers"][0]["uuid"], uuid);
        assert!(applied.main_uuid.is_none());
        assert!(has_main(&config));
    }

    #[test]
    fn null_key_keeps_stored_key() {
        let mut config = AppConfig::default();
        apply(&mut config, &two_providers(Some(SECRET)), true).unwrap();
        let update = json!({
            "providers": [{
                "name": "openai",
                "endpoint": "https://api.openai.com/v1",
                "api_type": "openai",
                "api_key": null,
            }]
        });
        apply(&mut config, &update, false).unwrap();
        assert_eq!(config.providers[0].api_key, SECRET);
    }

    #[test]
    fn moving_awareness_leaves_unmentioned_planner() {
        let mut config = AppConfig::default();
        apply(
            &mut config,
            &json!({
                "providers": [provider("openai", "https://api.openai.com/v1", "openai", Some(SECRET))],
                "models": {
                    "main": {"provider": "openai", "model_id": "gpt-4.1"},
                    "planner": {"provider": "openai", "model_id": "gpt-4.1"},
                    "awareness": {"provider": "openai", "model_id": "gpt-4.1"},
                }
            }),
            true,
        )
        .unwrap();
        let moved = json!({
            "models": {
                "awareness": {"provider": "openai", "model_id": "gpt-4.1-mini"}
            }
        });
        apply(&mut config, &moved, false).unwrap();
        let old = config
            .models
            .iter()
            .find(|model| model.model_id == "gpt-4.1")
            .unwrap();
        assert!(old.effective_roles().contains(&ModelRole::Planner));
        assert!(old.effective_roles().contains(&ModelRole::Main));
        assert!(!old.effective_roles().contains(&ModelRole::Awareness));
        let awareness = config
            .models
            .iter()
            .find(|model| model.model_id == "gpt-4.1-mini")
            .unwrap();
        assert_eq!(awareness.effective_roles(), vec![ModelRole::Awareness]);
    }

    #[test]
    fn provider_without_a_role_is_saved() {
        let mut config = AppConfig::default();
        apply(
            &mut config,
            &json!({
                "providers": [
                    provider("openai", "https://api.openai.com/v1", "openai", Some(SECRET)),
                    provider("spare", "https://spare.example/v1", "openai", Some("sk-spare")),
                ],
                "models": {"main": {"provider": "openai", "model_id": "gpt-4.1"}}
            }),
            true,
        )
        .unwrap();
        assert!(config
            .providers
            .iter()
            .any(|provider| provider.name == "spare"));
        assert!(config
            .models
            .iter()
            .all(|model| model.provider_uuid != config.providers[1].uuid));
    }

    #[test]
    fn rejections_do_not_mutate_or_echo_the_key() {
        let mut config = AppConfig::default();
        apply(&mut config, &two_providers(Some(SECRET)), true).unwrap();
        let before = config.clone();
        let cases = [
            json!({"providers": [
                provider("dup", "https://dup.example/v1", "openai", Some(SECRET)),
                provider("dup", "https://dup.example/v1", "openai", Some(SECRET)),
            ]}),
            json!({"providers": [
                provider("openai", "https://api.openai.com/v1", "openai", Some("   ")),
            ]}),
            json!({"providers": [provider("new", "ftp://files.example", "openai", Some(SECRET))]}),
            json!({"providers": [provider("brand", "https://brand.example/v1", "openai", None)]}),
            json!({"models": {"main": {"provider": "missing", "model_id": "gpt-4.1"}}}),
            json!({"models": {"main": {"provider": "openai", "model_id": "  "}}}),
            json!({}),
            json!({"models": {"sidekick": {"provider": "openai", "model_id": "gpt-4.1"}}}),
        ];
        for params in cases {
            let err = apply(&mut config, &params, false).unwrap_err();
            assert!(!err.contains(SECRET), "{err}");
            assert!(!err.contains("sk-ant-test"), "{err}");
            assert_unchanged(&before, &config);
        }
        let mut empty = AppConfig::default();
        let err = apply(
            &mut empty,
            &json!({"providers": [provider("openai", "https://api.openai.com/v1", "openai", Some(SECRET))]}),
            true,
        )
        .unwrap_err();
        assert_eq!(err, "models.main is required");
        assert!(empty.providers.is_empty());
        assert!(!err.contains(SECRET));
    }

    #[test]
    fn extension_owned_name_is_rejected() {
        let mut config = AppConfig::default();
        config.providers.push(ProviderConn {
            uuid: "ext-1".into(),
            name: "openai".into(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "https://ext.example".into(),
            api_key: "ext-key".into(),
            ext_id: Some("demo".into()),
        });
        let err = apply(&mut config, &two_providers(Some(SECRET)), true).unwrap_err();
        assert!(err.contains("owned by an extension"));
        assert!(!err.contains(SECRET));
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].api_key, "ext-key");
    }

    #[test]
    fn live_main_pin_replaces_session_main_and_keeps_planner() {
        let mut config = AppConfig::default();
        let first = apply(&mut config, &two_providers(Some(SECRET)), true).unwrap();
        let old_main = first.main_uuid.unwrap();
        let global_main = config
            .models
            .iter()
            .find(|model| model.uuid == old_main)
            .unwrap()
            .clone();
        let mut session_models = vec![
            ModelEntry {
                uuid: "local-main".into(),
                roles: vec![ModelRole::Main],
                source_uuid: Some(old_main.clone()),
                ..global_main.clone()
            },
            ModelEntry {
                uuid: "local-planner".into(),
                name: "planner".into(),
                model_id: "claude-sonnet-4-5".into(),
                provider_uuid: config.providers[1].uuid.clone(),
                roles: vec![ModelRole::Planner],
                source_uuid: Some("global-planner".into()),
                ..ModelEntry::default()
            },
        ];
        let swapped = apply(
            &mut config,
            &json!({"models": {"main": {"provider": "anthropic", "model_id": "claude-sonnet-4-5"}}}),
            false,
        )
        .unwrap();
        let new_main = swapped.main_uuid.expect("main uuid");
        assert_ne!(new_main, old_main);
        let global = config
            .models
            .iter()
            .find(|model| model.uuid == new_main)
            .unwrap()
            .clone();
        assign_role(&mut session_models, ModelRole::Main, Some(&global));
        let planner = session_models
            .iter()
            .find(|model| model.uuid == "local-planner")
            .unwrap();
        assert_eq!(planner.roles, vec![ModelRole::Planner]);
        let pinned: Vec<_> = session_models
            .iter()
            .filter(|model| model.effective_roles().contains(&ModelRole::Main))
            .collect();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].source_uuid.as_deref(), Some(new_main.as_str()));
        let old_global = config
            .models
            .iter()
            .find(|model| model.model_id == "gpt-4.1")
            .unwrap();
        assert!(!old_global.effective_roles().contains(&ModelRole::Main));
    }
}

//! UN-ATTACHED GUI catalogue builders: the model/route live-fetches and the
//! /agents + OAuth dashboard readers the [`super::host`] swapper serves when
//! no daemon is attached (onboarding / the empty-state start screen, where
//! there is no in-memory `AppConfig` to answer from — these load
//! `~/.koma/config.json` straight off disk instead). Split out of
//! [`super::host`] for file size — PURE code motion, no behaviour change.

/// UN-ATTACHED GUI model-picker fetch (a [`super::HostCtl::ListModels`] serviced by the
/// swapper): load the GLOBAL config and resolve the provider by uuid. A `config.providers`
/// entry gets a live `GET {endpoint}/models`, falling back to the curated `catalogue_overlay`
/// if that comes back empty; a `config.oauth_conns` entry (Codex/Claude/xAI) has no live
/// fetch worth making and resolves straight to the overlay via
/// `registry::meta(conn.provider).chat_endpoint`. Returns an EMPTY list on an unknown
/// provider OR any fetch error — the caller ALWAYS pushes a reply, so the React picker's
/// spinner clears. Mirrors the daemon's attached-path `ClientRequest::ListModels` handler
/// (`hub::requests_read`), but sources the provider from disk since the swapper holds no
/// in-memory `AppConfig`.
pub(super) async fn fetch_models_for_provider(provider: &str) -> Vec<String> {
    let cfg = crate::model::app_config::AppConfig::load();
    if let Some(p) = cfg.providers.iter().find(|p| p.uuid == provider) {
        let c = crate::app::runtime::session_mgmt::build_client();
        let conn = crate::service::openrouter::Conn {
            endpoint: &p.endpoint,
            api_key: &p.api_key,
            api_type: crate::model::app_config::ApiType::OpenAiCompatible,
            account_id: "",
            oauth_uuid: "",
            install_id: "",
        };
        let mut models = c
            .list_models(conn)
            .await
            .map(|v| v.into_iter().map(|m| m.id).collect::<Vec<_>>())
            .unwrap_or_default();
        if models.is_empty() {
            models = crate::service::catalogue_overlay::models_for(&p.endpoint)
                .into_iter()
                .map(|m| m.id)
                .collect();
        }
        return models;
    }
    if let Some(conn) = cfg.oauth_conns.iter().find(|c| c.uuid == provider) {
        let endpoint = crate::service::oauth::registry::meta(conn.provider).chat_endpoint;
        return crate::service::catalogue_overlay::models_for(endpoint)
            .into_iter()
            .map(|m| m.id)
            .collect();
    }
    Vec::new()
}

/// UN-ATTACHED GUI route-picker fetch (a [`super::HostCtl::ListRoutes`] serviced by the
/// swapper): load the GLOBAL config, resolve the provider by uuid, GATE on it being an OpenRouter-
/// style routable endpoint (the model-endpoints API is OpenRouter-specific — a non-OpenRouter
/// provider gets an immediate EMPTY list with no network call), then `GET
/// {endpoint}/models/{model_id}/endpoints`, flattening each route to the wire subset.
/// Returns EMPTY on an unknown/non-OpenRouter provider OR any fetch error (the caller always
/// pushes a reply → the form falls back to "Auto"). Mirrors the daemon's attached-path
/// `ClientRequest::ListRoutes` handler, including its OpenRouter gate.
pub(super) async fn fetch_routes_for_provider(
    provider: &str,
    model_id: &str,
) -> Vec<crate::ipc::proto::ModelEndpointWire> {
    let cfg = crate::model::app_config::AppConfig::load();
    let Some(p) = cfg.providers.iter().find(|p| p.uuid == provider) else {
        return Vec::new();
    };
    // OpenRouter-only gate, mirroring the daemon path: the endpoints API is OpenRouter-
    // specific, so a non-OpenRouter provider yields an empty route list (form → "Auto").
    if !(p.api_type.is_routable() && p.endpoint.to_lowercase().contains("openrouter")) {
        return Vec::new();
    }
    let c = crate::app::runtime::session_mgmt::build_client();
    let conn = crate::service::openrouter::Conn {
        endpoint: &p.endpoint,
        api_key: &p.api_key,
        api_type: crate::model::app_config::ApiType::OpenAiCompatible,
        account_id: "",
        oauth_uuid: "",
        install_id: "",
    };
    c.list_model_endpoints(conn, model_id)
        .await
        .map(|eps| {
            eps.into_iter()
                .map(|ep| crate::ipc::proto::ModelEndpointWire {
                    name: ep.name,
                    provider_name: ep.provider_name,
                    price_prompt: ep.pricing.as_ref().and_then(|pr| pr.prompt.clone()),
                    price_completion: ep.pricing.as_ref().and_then(|pr| pr.completion.clone()),
                    uptime_last_30m: ep.uptime_last_30m,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Build the UN-ATTACHED GUI /agents reply (a [`super::HostCtl::GetAgents`] serviced by the
/// swapper / start-screen): the built-in + global agent roster (`load_registry(None)` — no
/// session overlay) plus the GLOBAL model / provider catalogue off the loaded config.
/// Mirrors the daemon's attached `send_agents_values` builder but with NO session (no
/// `session_models`, no session agents), so the dashboard populates identically on the
/// start screen. ALWAYS returns a value (empty vecs at worst), so the caller always pushes
/// a reply and the loading state clears. `pub(super)` so the sibling `host_config` swapper
/// mutation path can reuse it to re-push after an un-attached agent create / delete.
pub(super) fn build_host_agents_values() -> (
    Vec<crate::ipc::proto::AgentEntry>,
    Vec<crate::ipc::proto::CatalogueModelSnapshot>,
    Vec<crate::ipc::proto::CatalogueProviderSnapshot>,
) {
    use crate::model::agent_def::{load_registry, AgentSource};
    let registry = load_registry(None);
    let agents = registry
        .list(false)
        .into_iter()
        .map(|ag| crate::ipc::proto::AgentEntry {
            name: ag.name.clone(),
            description: ag.description.clone(),
            conditions: ag.conditions.clone(),
            source: match ag.source {
                AgentSource::Session => "session",
                AgentSource::Global => "global",
                AgentSource::Builtin => "builtin",
                AgentSource::Extension => "extension",
            }
            .to_string(),
            model_uuid: ag.model_uuid.clone(),
            model: ag.model.clone(),
            tools: ag.tools.clone(),
            prompt: ag.prompt.clone(),
        })
        .collect();
    let cfg = crate::model::app_config::AppConfig::load();
    let catalogue_models = cfg
        .models
        .iter()
        .map(|e| crate::ipc::proto::CatalogueModelSnapshot {
            uuid: e.uuid.clone(),
            name: e.name.clone(),
            model_id: e.model_id.clone(),
            provider_uuid: e.provider_uuid.clone(),
        })
        .collect();
    let catalogue_providers = cfg
        .providers
        .iter()
        .map(|p| crate::ipc::proto::CatalogueProviderSnapshot {
            uuid: p.uuid.clone(),
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
        })
        .collect();
    (agents, catalogue_models, catalogue_providers)
}

/// Build the UN-ATTACHED GUI OAuth reply (a [`super::HostCtl::GetOAuthState`] serviced by the
/// swapper / start-screen): the persisted OAuth connections (TOKENLESS wire projection off
/// `~/.koma/config.json`) + the data-driven provider catalogue. Mirrors the daemon's
/// attached `send_oauth_state` builder but sources the connections from disk (the swapper
/// holds no in-memory `AppConfig`), so the OAuth screen populates identically on the start
/// screen. NEVER serializes a token — the wire type ([`crate::ipc::proto::OAuthConnWire`])
/// has no token field. `pub(super)` so the sibling swapper delete arm can reuse it.
pub(super) fn build_host_oauth_state() -> (
    Vec<crate::ipc::proto::OAuthConnWire>,
    Vec<crate::ipc::proto::OAuthProviderWire>,
) {
    let cfg = crate::model::app_config::AppConfig::load();
    let conns = cfg
        .oauth_conns
        .iter()
        .map(|c| crate::ipc::proto::OAuthConnWire {
            uuid: c.uuid.clone(),
            name: c.name.clone(),
            provider: c.provider.wire_id().to_string(),
            email: c.email.clone(),
            plan: c.plan.clone(),
            account_id: c.account_id.clone(),
        })
        .collect();
    let providers = crate::service::oauth::registry::oauth_providers()
        .into_iter()
        .map(|(id, label, kind)| crate::ipc::proto::OAuthProviderWire {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
        })
        .collect();
    (conns, providers)
}

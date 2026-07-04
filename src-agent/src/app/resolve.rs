//! Per-role route resolution: the single chokepoint that turns a [`ModelRole`]
//! into one concrete [`Resolved`] route (model id + endpoint + key + wire type +
//! optional upstream pin + effort).
//!
//! The runtime has four model-driven roles — Main (interactive chat), Awareness
//! (project-doc summary), Safeguard (the safety classifier), and Compactor
//! (`/compact`, which rides Main today). Each is assigned a model via the global
//! catalogue (`config.models`) or a per-session override (`settings.session_models`);
//! that model points at a provider connection (`config.providers`) by uuid, which
//! carries the endpoint + key + wire type. [`resolve_role`] walks that chain and
//! produces the route the call site hands to the client.
//!
//! ## Resolution order
//!
//! 1. Find the model assigned to `role`: session overrides first
//!    (`settings.session_models`), then the global catalogue (`config.models`).
//! 2. Resolve that model's provider by `provider_uuid` against `config.providers`.
//!    A hit produces the [`Resolved`] route directly.
//! 3. If no model is assigned, OR the assigned model's `provider_uuid` does not
//!    resolve to a known provider, fall through to the per-role LEGACY fallback —
//!    the old per-field `settings.*` behaviour, so an empty/old config never
//!    bricks chat (Main/Compactor/Awareness) and the safeguard fails CLOSED.
//!
//! ## Fallback table (when step 2 finds no provider)
//!
//! | role      | fallback                                                            |
//! |-----------|---------------------------------------------------------------------|
//! | Main      | legacy `settings.model` / `api_key` @ `DEFAULT_BASE_URL`            |
//! | Compactor | resolve Main (compactor rides Main; no config slot of its own)      |
//! | Awareness | inherit Main (same route as the Main role)                          |
//! | Safeguard | legacy `classifier_model` if set; else `None` (FAIL-CLOSED)        |
//!
//! ## Foot-gun (do not regress)
//!
//! An Awareness model that is explicitly assigned (found in step 1 and whose provider
//! resolves in step 2) ALWAYS wins — explicit assignment is the only way to give
//! Awareness its own model. When nothing is assigned, Awareness inherits Main so the
//! call works on any provider the user has actually configured, not just OpenRouter.

use crate::config::DEFAULT_BASE_URL;
use crate::model::agent_def::AgentDef;
use crate::model::app_config::{ApiType, AppConfig, ModelEntry, ModelRole, OAuthProvider};
use crate::model::settings::Settings;
use crate::service::openrouter::Conn;

/// One fully-resolved route for a runtime role: everything a client call needs to
/// reach the right model on the right provider, with no further config lookups.
///
/// `route` is the OpenRouter upstream-provider pin (`None` = automatic routing).
/// `effort` carries the reasoning-effort token and is only ever non-empty for the
/// Main role (the only role that exposes effort today); every other role resolves
/// it to `String::new()`.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model_id: String,
    pub endpoint: String,
    pub api_key: String,
    // The provider's wire type. Consumed at the call boundary via
    // [`Resolved::is_routable`]: only `OpenAiCompatible` dispatches; an
    // `AnthropicCompatible` route fails/skips (native Anthropic is deferred — see
    // the `model/app_config` `ApiType` docs). The client itself never branches on
    // this; the gate lives ONE level up, here at resolution.
    pub api_type: ApiType,
    pub route: Option<String>,
    // Consumed by the MAIN streaming path (passed as the `effort` param of
    // `stream_complete`). Non-empty only for the Main role; every other role
    // resolves it to "".
    pub effort: String,
    /// Provider-specific identity carried onto the [`Conn`]: for a Codex OAuth
    /// route the ChatGPT `chatgpt-account-id`; for a Kilo OAuth route the
    /// organization id; "" for a static-key route. Filled by the OAuth
    /// resolution path (a later wave); every non-OAuth construction site sets "".
    pub account_id: String,
    /// The [`OAuthConn`](crate::model::app_config::OAuthConn) uuid this route
    /// authenticates through, threaded onto the [`Conn`] so the send-time
    /// `fresh_key` hook can refresh a near-expiry token. "" = static key (no
    /// refresh). Filled by the OAuth resolution path (a later wave).
    pub oauth_uuid: String,
}

impl Resolved {
    /// Borrow this route's `endpoint` + `api_key` (+ wire type + OAuth identity)
    /// as a [`Conn`] for a client call (the connection part of the route —
    /// "where + auth + how").
    pub fn conn(&self) -> Conn<'_> {
        Conn {
            endpoint: &self.endpoint,
            api_key: &self.api_key,
            api_type: self.api_type,
            account_id: &self.account_id,
            oauth_uuid: &self.oauth_uuid,
        }
    }

    /// The OpenRouter upstream-provider pin as a slug (`""` = automatic routing),
    /// the form the client's `provider_routing_for` expects.
    pub fn provider(&self) -> &str {
        self.route.as_deref().unwrap_or("")
    }

    /// Whether this route can actually be dispatched against the OpenAI-compatible
    /// client. `false` for an `AnthropicCompatible` provider (native Anthropic is
    /// deferred — see [`ApiType`]). The call boundary checks this BEFORE dispatch:
    /// the interactive Main path emits a [`crate::service::StreamEvent::Error`] and
    /// does not POST; secondary roles (awareness / shortsend fold+router /
    /// safeguard) skip the call gracefully (no summary / no recall / fail-closed).
    pub fn is_routable(&self) -> bool {
        self.api_type.is_routable()
    }
}

/// Find a registered [`ModelEntry`] by `uuid`, checking `settings.session_models`
/// first (per-session overrides win), then the global `config.models`. Returns
/// `None` when no entry with that uuid exists in either catalogue.
fn find_model_entry<'a>(
    config: &'a AppConfig,
    settings: &'a Settings,
    uuid: &str,
) -> Option<&'a ModelEntry> {
    settings
        .session_models
        .iter()
        .find(|e| e.uuid == uuid)
        .or_else(|| config.models.iter().find(|e| e.uuid == uuid))
}

/// Build a [`Resolved`] from an assigned [`ModelEntry`] by resolving its
/// `provider_uuid`, first against `config.providers` (a static-key provider) and
/// then — on a miss — against `config.oauth_conns` (an OAuth-backed connection:
/// Codex / Kilo Code). Returns `None` only when the `provider_uuid` matches
/// NEITHER catalogue (a dangling reference), which the caller treats the same as
/// "no assignment" and falls through to the legacy fallback.
///
/// A static-key provider carries no OAuth identity (`account_id`/`oauth_uuid`
/// empty). An OAuth-backed connection resolves its endpoint from
/// [`registry::meta`](crate::service::oauth::registry::meta), its bearer from the
/// conn's `access_token`, and threads the conn's `uuid` (for the send-time
/// `fresh_key` refresh) plus the provider-specific identity (`account_id` for
/// Codex, `org_id` for Kilo) onto the route.
///
/// `effort` is taken from `settings.effort` only for the Main role; every other
/// role gets an empty effort.
fn from_entry(config: &AppConfig, settings: &Settings, entry: &ModelEntry, role: ModelRole) -> Option<Resolved> {
    let effort = if role == ModelRole::Main {
        settings.effort.clone()
    } else {
        String::new()
    };
    if let Some(provider) = config.providers.iter().find(|p| p.uuid == entry.provider_uuid) {
        return Some(Resolved {
            model_id: entry.model_id.clone(),
            endpoint: provider.endpoint.clone(),
            api_key: provider.api_key.clone(),
            api_type: provider.api_type,
            route: entry.route.clone(),
            effort,
            // Static-key provider route: no OAuth identity.
            account_id: String::new(),
            oauth_uuid: String::new(),
        });
    }
    // Fall back to an OAuth-backed connection (Codex / Kilo Code).
    let conn = config.oauth_conns.iter().find(|c| c.uuid == entry.provider_uuid)?;
    let meta = crate::service::oauth::registry::meta(conn.provider);
    let api_type = match conn.provider {
        OAuthProvider::Codex => ApiType::Codex,
        OAuthProvider::Kilocode => ApiType::OpenAiCompatible,
    };
    let account_id = match conn.provider {
        OAuthProvider::Codex => conn.account_id.clone(),
        OAuthProvider::Kilocode => conn.org_id.clone(),
    };
    Some(Resolved {
        model_id: entry.model_id.clone(),
        endpoint: meta.chat_endpoint.to_string(),
        api_key: conn.access_token.clone(),
        api_type,
        route: entry.route.clone(),
        effort,
        account_id,
        oauth_uuid: conn.uuid.clone(),
    })
}

/// The universal Main soft-fallback: a [`Resolved`] built from the OLD per-field
/// `settings` (api_key + model + provider + effort @ [`DEFAULT_BASE_URL`], the
/// OpenAI-compatible wire). Keeps chat alive when `config` is empty/old or an
/// assigned Main provider is missing, exactly preserving today's behaviour.
fn legacy_main(settings: &Settings) -> Resolved {
    Resolved {
        model_id: settings.model.clone(),
        endpoint: DEFAULT_BASE_URL.to_string(),
        api_key: settings.api_key.clone(),
        api_type: ApiType::OpenAiCompatible,
        route: None,
        effort: settings.effort.clone(),
        account_id: String::new(),
        oauth_uuid: String::new(),
    }
}

/// Per-role fallback used when no model is assigned to `role`, or the assigned
/// model's provider is dangling. Only handles Main (soft-fallback to legacy
/// settings fields) and Safeguard (fail-closed). Compactor and Awareness inherit
/// the fully-resolved Main route instead — that redirect is handled in
/// [`resolve_role`] before this function is called, so neither role reaches here.
fn legacy_fallback(settings: &Settings, role: ModelRole) -> Option<Resolved> {
    match role {
        // Chat is the product — Main must never go dark.
        ModelRole::Main => Some(legacy_main(settings)),
        // Compactor and Awareness are redirected to resolve_role(Main) before
        // reaching here; these arms are unreachable in practice but kept for
        // exhaustiveness.
        ModelRole::Compactor | ModelRole::Awareness => Some(legacy_main(settings)),
        ModelRole::Safeguard => {
            // FAIL-CLOSED: only the legacy classifier model rescues it; an empty
            // field yields `None`, which the harness caller degrades to a human
            // prompt (TAC) / advisory toast (PC) rather than silently allowing.
            if settings.classifier_model.is_empty() {
                None
            } else {
                Some(Resolved {
                    model_id: settings.classifier_model.clone(),
                    endpoint: DEFAULT_BASE_URL.to_string(),
                    api_key: settings.api_key.clone(),
                    api_type: ApiType::OpenAiCompatible,
                    route: None,
                    effort: String::new(),
                    account_id: String::new(),
                    oauth_uuid: String::new(),
                })
            }
        }
    }
}

/// Resolve the concrete route for `role`.
///
/// Session overrides (`settings.session_models`) win over the global catalogue
/// (`config.models`); the chosen model's provider is resolved by uuid against
/// `config.providers`. A successful resolution returns the assigned route. When no
/// model is assigned, or the assigned model's provider is dangling, the per-role
/// legacy fallback applies (see [`legacy_fallback`]). Returns `None` only for an
/// unresolved Safeguard (fail-closed); every other role always resolves to
/// `Some`.
pub fn resolve_role(config: &AppConfig, settings: &Settings, role: ModelRole) -> Option<Resolved> {
    // 1. Pick the assigned model: per-session overrides first, then the global
    //    catalogue. A model may hold several roles, so match on whether its
    //    effective role set CONTAINS `role` (this also folds the legacy
    //    single-role field in via `effective_roles`).
    let assigned = settings
        .session_models
        .iter()
        .find(|e| e.effective_roles().contains(&role))
        .or_else(|| config.models.iter().find(|e| e.effective_roles().contains(&role)));

    // 2. If a model is assigned AND its provider resolves, that route wins —
    //    including an explicitly-assigned Awareness model (explicit assignment is
    //    the only way to give Awareness its own dedicated model).
    if let Some(entry) = assigned {
        if let Some(resolved) = from_entry(config, settings, entry, role) {
            return Some(resolved);
        }
        // Assigned but the provider_uuid is dangling → fall through.
    }

    // 3. Compactor and Awareness have no config slot of their own — both inherit
    //    the FULLY-RESOLVED Main route (which honours config.models Main + its
    //    provider connection's real endpoint/key). This must happen here, where
    //    `config` is in scope, NOT inside `legacy_fallback`, which only has
    //    `settings` and would wrongly hard-code DEFAULT_BASE_URL (OpenRouter).
    //    No infinite recursion: Main never resolves to Compactor or Awareness.
    if matches!(role, ModelRole::Compactor | ModelRole::Awareness) {
        return resolve_role(config, settings, ModelRole::Main);
    }

    // 4. No assignment, or a dangling provider → per-role legacy fallback.
    legacy_fallback(settings, role)
}

/// Resolve the concrete route for a sub-agent ([`AgentDef`]).
///
/// A sub-agent carries its OWN model + provider on the definition, independent of
/// the runtime role catalogue:
///
/// 1. If the agent names a `model` AND its `provider_uuid` resolves to a known
///    provider connection, dispatch against THAT provider (endpoint + key + wire
///    type), pinning the agent's legacy `provider` routing slug as the upstream
///    route. This is the explicit-assignment path and always wins.
/// 2. Otherwise — the agent has no model, or its `provider_uuid` is absent /
///    dangling — inherit the fully-resolved Main route so the sub-agent runs on
///    whatever provider the user has actually configured (never silently dark).
///
/// In BOTH cases the agent's own reasoning `effort` overrides the route's effort
/// when set (an agent declares its own thinking budget); an unset effort keeps the
/// inherited one. Returns `None` only when the agent has no usable model AND Main
/// itself can't resolve — practically never, since Main has a legacy soft-fallback.
///
/// True when `agent` declares its own model (model_uuid or legacy model field)
/// that is non-empty. Does NOT test whether the model actually resolves; use
/// [`agent_model_resolves`] for that. This is the "did the agent author set a
/// model at all" predicate used to decide whether a fallback-to-Main is
/// surprising (declared but unresolvable) or expected (no model declared).
pub fn agent_declares_model(agent: &AgentDef) -> bool {
    agent.model_uuid.as_deref().is_some_and(|u| !u.trim().is_empty())
        || agent.model.as_deref().is_some_and(|m| !m.trim().is_empty())
}

/// True when `agent`'s declared model resolves to a concrete (non-Main-fallback)
/// route. Returns false both when the agent declares NO model and when it declares
/// one that can't be resolved (deleted entry / dangling provider / stale
/// session_models). The caller pairs this with [`agent_declares_model`]: warn when
/// the agent declared a model but this returns false.
pub fn agent_model_resolves(config: &AppConfig, settings: &Settings, agent: &AgentDef) -> bool {
    // 1. Registered model uuid → resolvable entry+provider.
    if let Some(uuid) = agent.model_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
        if let Some(entry) = find_model_entry(config, settings, uuid) {
            if from_entry(config, settings, entry, ModelRole::Main).is_some() {
                return true;
            }
        }
        // uuid unresolved (deleted entry / dangling provider) → fall through to legacy.
    }
    // 2. Legacy explicit model + resolvable provider connection.
    if let Some(model_id) = agent.model.as_deref().filter(|m| !m.trim().is_empty()) {
        let _ = model_id;
        if let Some(uuid) = agent.provider_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
            if config.providers.iter().any(|p| p.uuid == uuid) {
                return true;
            }
        }
    }
    false
}

/// Currently only called by the (Stage-1 inert) sub-agent spawn path, so it is
/// unreferenced from the binary until that path is wired in — hence the allow.
#[allow(dead_code)]
pub fn resolve_agent(config: &AppConfig, settings: &Settings, agent: &AgentDef) -> Option<Resolved> {
    // The agent's declared effort, applied on top of whichever route we land on.
    let agent_effort = agent.effort.clone();
    let with_effort = |mut r: Resolved| -> Resolved {
        if let Some(e) = &agent_effort {
            r.effort = e.clone();
        }
        r
    };

    // 1a. Registered model uuid → look up the ModelEntry and resolve via from_entry.
    //     A uuid that no longer exists (deleted entry) falls through gracefully.
    if let Some(uuid) = agent.model_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
        if let Some(entry) = find_model_entry(config, settings, uuid) {
            if let Some(resolved) = from_entry(config, settings, entry, ModelRole::Main) {
                return Some(with_effort(resolved));
            }
            // Entry found but its provider is dangling → fall through to legacy.
        }
        // uuid present but no matching entry (deleted) → fall through.
    }

    // 1b. Legacy explicit model + resolvable provider connection → dispatch there.
    if let Some(model_id) = agent.model.as_deref().filter(|m| !m.trim().is_empty()) {
        if let Some(uuid) = agent.provider_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
            if let Some(provider) = config.providers.iter().find(|p| p.uuid == uuid) {
                return Some(with_effort(Resolved {
                    model_id: model_id.to_string(),
                    endpoint: provider.endpoint.clone(),
                    api_key: provider.api_key.clone(),
                    api_type: provider.api_type,
                    // The legacy free-text `provider` field is an OpenRouter
                    // upstream-routing slug (None = automatic routing).
                    route: agent.provider.clone(),
                    effort: String::new(),
                    account_id: String::new(),
                    oauth_uuid: String::new(),
                }));
            }
        }
        // A named model whose provider is absent/dangling falls through to Main —
        // better to run on the configured Main provider than to go dark.
    }

    // 2. No usable model/provider → inherit the Main route.
    resolve_role(config, settings, ModelRole::Main).map(with_effort)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::app_config::{OAuthConn, OAuthProvider};

    fn model_entry(provider_uuid: &str) -> ModelEntry {
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
        config.models.push(model_entry("codex-uuid"));
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
        config.models.push(model_entry("kilo-uuid"));
        let settings = Settings::default();

        let resolved = resolve_role(&config, &settings, ModelRole::Main).expect("Main must resolve");
        assert_eq!(resolved.endpoint, crate::service::oauth::registry::meta(OAuthProvider::Kilocode).chat_endpoint);
        assert_eq!(resolved.api_key, "kilo-token");
        assert_eq!(resolved.api_type, ApiType::OpenAiCompatible);
        assert_eq!(resolved.account_id, "org-456");
        assert_eq!(resolved.oauth_uuid, "kilo-uuid");
    }
}

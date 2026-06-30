//! First-run credential wizard: SaveCreds action + endpoint-to-provider-name helper.

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::config::{
    DEFAULT_AWARENESS_MODEL, DEFAULT_AWARENESS_PROVIDER, DEFAULT_CLASSIFIER_MODEL,
    DEFAULT_CLASSIFIER_PROVIDER, DEFAULT_MODEL,
};
use crate::model::app_config::{ApiType, ModelEntry, ModelRole, ProviderConn};
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

/// Derive a human-readable provider name from a base-URL `endpoint`, used to
/// label the [`ProviderConn`] the first-run wizard writes.
///
/// - An OpenRouter URL (case-insensitive) → `"OpenRouter"`.
/// - Otherwise the URL host (scheme, userinfo, port, and path stripped), e.g.
///   `https://api.example.com/v1` → `"api.example.com"`.
/// - Anything we can't parse a host out of → `"Provider"`.
pub(super) fn provider_name_from_endpoint(endpoint: &str) -> String {
    if endpoint.to_lowercase().contains("openrouter") {
        return "OpenRouter".to_string();
    }
    // Strip the scheme (`https://`, `http://`, or any `scheme://`).
    let after_scheme = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    // Drop any userinfo (`user:pass@host`), then cut at the first path/port/query
    // delimiter to isolate the host.
    let authority = after_scheme.rsplit_once('@').map(|(_, h)| h).unwrap_or(after_scheme);
    let host = authority
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    if host.is_empty() {
        "Provider".to_string()
    } else {
        host.to_string()
    }
}

/// Handle `Action::SaveCreds`: persist the first-run wizard's entered
/// credentials, seed the global provider/model catalogue, build a keyless
/// client, and transition to Chat.
pub(super) fn handle_save_creds(
    endpoint: String,
    api_key: String,
    model: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let endpoint = if endpoint.is_empty() {
        crate::config::DEFAULT_BASE_URL.to_string()
    } else {
        endpoint
    };
    let model = if model.is_empty() {
        DEFAULT_MODEL.to_string()
    } else {
        model
    };
    // Lazy creation: first-run path has no session yet. Create it now,
    // then apply the entered credentials.
    if state.rest.fg().session.is_none() {
        match store::create_session() {
            Ok(s) => state.rest.fg_mut().session = Some(s),
            Err(e) => {
                state.rest.fg_mut().status = format!("error: {e}");
                return Ok(());
            }
        }
    }
    // Back-compat + startup none-gate + legacy resolver fallback: mirror
    // the entered creds onto the session settings. `provider` (OpenRouter
    // routing slug) stays EMPTY — the wizard pins routing via the config
    // ProviderConn/ModelEntry below, not the legacy slug.
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        sess.settings.api_key = api_key.clone();
        sess.settings.model = model.clone();
        sess.settings.provider = String::new();
        let _ = sess.save();
    }
    // Provider-agnostic config write (first-run): build a real
    // ProviderConn from the ENTERED endpoint (NOT hardcoded OpenRouter)
    // plus a Main-role ModelEntry, and persist config.json. Only seed when
    // the catalogue is empty (first-run norm) so a re-entry over an
    // existing config doesn't duplicate entries — the session-settings
    // mirror above already covers the legacy fallback in that case.
    if state.rest.config.providers.is_empty() && state.rest.config.models.is_empty() {
        let provider_uuid = uuid::Uuid::new_v4().to_string();
        state.rest.config.providers.push(ProviderConn {
            uuid: provider_uuid.clone(),
            name: provider_name_from_endpoint(&endpoint),
            api_type: ApiType::OpenAiCompatible,
            endpoint: endpoint.clone(),
            api_key: api_key.clone(),
        });
        state.rest.config.models.push(ModelEntry {
            uuid: uuid::Uuid::new_v4().to_string(),
            name: "main".to_string(),
            model_id: model.clone(),
            provider_uuid: provider_uuid.clone(),
            // Omnisearch routing is a later pass; no upstream pin here.
            route: None,
            // First-run only ever assigns Main (unchanged behavior); the
            // legacy single-role field is left None so it isn't written.
            roles: vec![ModelRole::Main],
            role: None,
        });
        // OpenRouter first-run: auto-register the cheap groq-pinned
        // Awareness and Safeguard model entries so the harness works
        // out-of-the-box without manual configuration.
        if endpoint.to_lowercase().contains("openrouter") {
            state.rest.config.models.push(ModelEntry {
                uuid: uuid::Uuid::new_v4().to_string(),
                name: "awareness".to_string(),
                model_id: DEFAULT_AWARENESS_MODEL.into(),
                provider_uuid: provider_uuid.clone(),
                route: Some(DEFAULT_AWARENESS_PROVIDER.into()),
                roles: vec![ModelRole::Awareness],
                role: None,
            });
            state.rest.config.models.push(ModelEntry {
                uuid: uuid::Uuid::new_v4().to_string(),
                name: "safeguard".to_string(),
                model_id: DEFAULT_CLASSIFIER_MODEL.into(),
                provider_uuid,
                route: Some(DEFAULT_CLASSIFIER_PROVIDER.into()),
                roles: vec![ModelRole::Safeguard],
                role: None,
            });
        }
        if let Err(e) = state.rest.config.save() {
            state.rest.fg_mut().status = format!("config save failed: {e}");
        }
    }
    // Routing slug is empty for the wizard path (config drives routing).
    state.rest.remember_creds(&api_key, &model, "");
    // KEYLESS client → no creds baked in; just (re)build for a fresh
    // plan_word at this session boundary. Resolve gates whether there's a
    // usable Main route (non-empty key) so we don't pin a no-creds client.
    *client = state.rest.fg().session.as_ref().and_then(|sess| {
        crate::app::resolve::resolve_role(
            &state.rest.config,
            &sess.settings,
            crate::model::app_config::ModelRole::Main,
        )
        .filter(|r| !r.api_key.is_empty())
        .map(|_| build_client())
    });
    // Seed THIS (foreground) session's own counters from its log (new or
    // picker-prefilled session being confirmed).
    if let Some(p) = state.rest.fg().session.as_ref().map(|s| s.path.clone()) {
        let fg = state.rest.foreground;
        state.rest.load_token_totals(fg, &p);
    }
    state.rest.prev_session = None; // committed; discard fallback
    // Creds confirmed — a /new-spawned session is no longer pending, so a later
    // unrelated KeyInput cancel must not pop it (the spawn is now committed).
    state.rest.spawn_pending = false;
    state.rest.reset_scroll();
    // Land in Chat first, THEN warm: `warm_session` is non-blocking and may
    // upgrade the mode to `Mode::Loading` (animated splash) when it has warm
    // work to spawn, so it must run LAST to get the final word. With no warm
    // work it leaves the mode as the Chat we just set.
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    // Warm the confirmed session: reindex its workspace + (async) fetch the
    // catalogue and awareness summary so it's primed like a cold boot.
    super::super::warm_session(state, client, handle);
    Ok(())
}

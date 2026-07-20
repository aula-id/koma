//! Transient state for the guided PROVIDER onboarding wizard (`Mode::OnboardProvider`).
//!
//! Entered from the first-run chooser's "provider" row (`Action::OnboardProvider`).
//! Chains two steps end-to-end so a brand-new install can go from zero to chatting
//! through a signed-in provider account:
//!
//!   1. `Login`       — pick + sign in to an OAuth provider. Reuses the SAME
//!      [`OAuthFlowState`] machine + background login runners the `/settings` OAuth
//!      submenu drives; the wizard just binds them to this state instead.
//!   2. `ModelSelect` — omnisearch the signed-in provider's model catalogue and pick
//!      the model to bind as the GLOBAL Main model.
//!
//! On login success the `service_global` OAuth drain (or the synchronous paste path)
//! advances `step` to `ModelSelect` and stamps `new_conn_uuid` / `provider`. Selecting
//! a model emits [`Action::OnboardProviderSaveModel`](crate::controller::input::Action),
//! which writes a Main-role `ModelEntry` bound to the new connection and drops into
//! Chat (see `actions::onboard`).

use crate::app::mode::settings::{filter_models, OAuthFlowState};
use crate::dto::openrouter::ModelInfo;
use crate::model::app_config::OAuthProvider;
use crate::service::oauth::registry;

/// Which step of the wizard is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardProviderStep {
    /// Provider pick + OAuth login (drives `oauth_flow`).
    Login,
    /// Model omnisearch over the signed-in provider's catalogue.
    ModelSelect,
}

/// In-progress state of the guided provider onboarding wizard.
///
/// The model-result list is NOT stored here: it is recomputed from the on-demand
/// catalogue (`candidate_model_ids`) at render/keystroke time, exactly like the
/// `/settings` model omnisearch — the thin client renders it off the globally
/// projected `models_cache`, so no separate results projection is needed.
#[derive(Debug, Clone)]
pub struct OnboardProviderState {
    /// Which step is active.
    pub step: OnboardProviderStep,
    /// Reused OAuth connect-flow state machine (picker / wait / paste / failed).
    pub oauth_flow: OAuthFlowState,
    /// The just-created connection's uuid (set on login success); the saved model
    /// entry binds to it via `provider_uuid`.
    pub new_conn_uuid: String,
    /// Which provider was signed in (set on login success); drives the catalogue
    /// source in `ModelSelect` (Codex static list vs a network fetch).
    pub provider: Option<OAuthProvider>,
    /// Model omnisearch query.
    pub query: String,
    /// Highlighted row in the filtered model list.
    pub result_sel: usize,
}

impl OnboardProviderState {
    /// Fresh wizard at the Login step, provider picker open on the first row.
    pub fn new() -> Self {
        Self {
            step: OnboardProviderStep::Login,
            oauth_flow: OAuthFlowState::Pick(0),
            new_conn_uuid: String::new(),
            provider: None,
            query: String::new(),
            result_sel: 0,
        }
    }

    // --- Login step: provider picker nav (mirrors SettingsState::oauth_pick_*) ---

    /// Move the picker cursor up (no-op off `Pick`; clamps at 0).
    pub fn pick_up(&mut self) {
        if let OAuthFlowState::Pick(c) = &mut self.oauth_flow {
            *c = c.saturating_sub(1);
        }
    }

    /// Move the picker cursor down (no-op off `Pick`; clamps at the last option, 9).
    pub fn pick_down(&mut self) {
        if let OAuthFlowState::Pick(c) = &mut self.oauth_flow {
            *c = (*c + 1).min(9);
        }
    }

    // --- Login step: paste-token text field ---

    /// Append `c` to the paste-token draft (no-op off `CodexPaste`).
    pub fn paste_push_char(&mut self, c: char) {
        if let OAuthFlowState::CodexPaste { input, .. } = &mut self.oauth_flow {
            input.push(c);
        }
    }

    /// Delete the last char of the paste-token draft (no-op off `CodexPaste`).
    pub fn paste_backspace(&mut self) {
        if let OAuthFlowState::CodexPaste { input, .. } = &mut self.oauth_flow {
            input.pop();
        }
    }

    // --- ModelSelect step: result nav + query edit ---

    /// Move the result highlight up (clamps at 0).
    pub fn result_up(&mut self) {
        self.result_sel = self.result_sel.saturating_sub(1);
    }

    /// Move the result highlight down (clamps at `max`).
    pub fn result_down(&mut self, max: usize) {
        if self.result_sel < max {
            self.result_sel += 1;
        }
    }

    /// Append `c` to the omnisearch query and reset the result highlight.
    pub fn push_query_char(&mut self, c: char) {
        self.query.push(c);
        self.result_sel = 0;
    }

    /// Delete the last query char and reset the result highlight.
    pub fn backspace_query(&mut self) {
        self.query.pop();
        self.result_sel = 0;
    }
}

/// Compute the filtered candidate model ids for the `ModelSelect` step.
///
/// A network provider's on-demand `models_cache` counts ONLY when it was fetched for
/// THAT provider's own catalogue endpoint (else the cache is stale / for another
/// provider). Whenever it doesn't match — Codex/Claude have no network catalogue at
/// all, and a network provider (e.g. xAI) whose cache hasn't landed yet also lands
/// here — fall back to the curated [`catalogue_overlay`](crate::service::catalogue_overlay)
/// for the provider's chat endpoint, served through the SAME [`filter_models`]
/// omnisearch as a fetched catalogue (mirrors the `/settings` model modal's Codex
/// substitution, generalized to every OAuth provider the overlay covers).
///
/// Returns OWNED ids (no borrow held) so callers can freely mutate state afterwards.
/// Shared by the input handler (selection) and the view (render list) so they never
/// diverge on what the highlighted row means.
pub fn candidate_model_ids(
    provider: Option<OAuthProvider>,
    query: &str,
    models_cache: &[ModelInfo],
    models_cache_endpoint: Option<&str>,
) -> Vec<String> {
    // A network provider's live cache only counts when it was fetched for THAT
    // provider's own catalogue endpoint.
    let cache_matches = provider
        .map(|p| registry::meta(p).catalogue_endpoint)
        .filter(|ep| !ep.is_empty())
        .map(|ep| models_cache_endpoint == Some(ep))
        .unwrap_or(false);
    // Overlay fallback: no matching live cache — resolve the curated catalogue for
    // this provider's chat endpoint (empty if the provider has none / is unknown).
    let overlay_cache = if cache_matches {
        Vec::new()
    } else {
        provider
            .map(crate::service::catalogue_overlay::models_for_provider)
            .unwrap_or_default()
    };
    let cache: &[ModelInfo] = if cache_matches { models_cache } else { &overlay_cache };
    filter_models(cache, query)
        .into_iter()
        .map(|i| cache[i].id.clone())
        .collect()
}

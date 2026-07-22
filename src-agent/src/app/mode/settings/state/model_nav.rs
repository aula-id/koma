//! Model modal navigation, provider queries, omnisearch, and endpoints helpers
//! for [`SettingsState`].

use super::super::ModelField;
use super::SettingsState;
use crate::model::app_config::ApiType;

/// The model modal's selected provider slot, resolved from `provider_idx`:
/// providers first, then OAuth drafts (offset by `providers.len()`) — the
/// provider-cycle merge.
enum SelectedProvider<'a> {
    Provider(&'a super::super::ProviderDraft),
    OAuth(&'a super::super::OAuthDraft),
}

impl SettingsState {
    /// Resolve the model modal's `provider_idx` into the merged provider cycle.
    /// `None` when no modal is open or the index is out of range of BOTH lists.
    fn mm_selected_provider(&self) -> Option<SelectedProvider<'_>> {
        let idx = self.model_modal.as_ref()?.provider_idx;
        let n = self.providers.len();
        if idx < n {
            self.providers.get(idx).map(SelectedProvider::Provider)
        } else {
            self.oauth_drafts.get(idx - n).map(SelectedProvider::OAuth)
        }
    }

    /// `true` when the modal's selected provider is an OpenRouter endpoint
    /// (its [`ProviderDraft::endpoint`], lowercased, contains `"openrouter"`).
    /// `false` when no modal is open or the provider index is out of range.
    ///
    /// This now gates ONLY the Route field (the upstream-pin list is an
    /// OpenRouter-only feature). The MODEL field's omnisearch is gated by
    /// [`Self::mm_provider_omnisearchable`] instead — the catalogue is just
    /// `GET {endpoint}/models`, available for ANY non-empty endpoint.
    pub fn mm_provider_is_openrouter(&self) -> bool {
        self.model_modal
            .as_ref()
            .and_then(|m| self.providers.get(m.provider_idx))
            .map(|p| p.endpoint.to_lowercase().contains("openrouter"))
            .unwrap_or(false)
    }

    /// `true` when the Model field should be a live omnisearch: a real provider
    /// with a non-empty endpoint, OR any OAuth draft (Kilo has a live catalogue;
    /// Codex serves the static `CODEX_MODELS` list — both get the omnisearch UI).
    pub fn mm_provider_omnisearchable(&self) -> bool {
        match self.mm_selected_provider() {
            Some(SelectedProvider::Provider(p)) => !p.endpoint.trim().is_empty(),
            Some(SelectedProvider::OAuth(_)) => true,
            None => false,
        }
    }

    /// The edited provider's `(endpoint, api_key)` for the on-demand catalogue
    /// fetch, or `None` when no modal is open. For an OAuth-backed provider the
    /// endpoint is its CATALOGUE endpoint (`registry::meta(..).catalogue_endpoint`,
    /// empty for Codex — no network catalogue, see the static-list mechanism below)
    /// and the key is the OAuth draft's snapshotted access token.
    pub fn mm_provider_conn(&self) -> Option<(String, String)> {
        match self.mm_selected_provider()? {
            SelectedProvider::Provider(p) => Some((p.endpoint.clone(), p.api_key.clone())),
            SelectedProvider::OAuth(d) => {
                let ep = crate::service::oauth::registry::meta(d.provider)
                    .catalogue_endpoint
                    .to_string();
                Some((ep, d.key.clone()))
            }
        }
    }

    /// `true` when the modal's selected provider can serve the per-model
    /// provider-endpoints GET: it must be `OpenAiCompatible` (the endpoints
    /// catalogue is an OpenRouter/OpenAI-shaped API — an Anthropic-typed provider
    /// has no equivalent) AND an OpenRouter endpoint (the GET is OpenRouter-only).
    /// The runtime checks this before firing `list_model_endpoints` so a non-
    /// OpenRouter or Anthropic provider never triggers a doomed request — the modal
    /// is resolved to an empty endpoints list instead. `false` when no modal is
    /// open or the provider index is out of range.
    pub fn mm_provider_has_endpoints_api(&self) -> bool {
        self.model_modal
            .as_ref()
            .and_then(|m| self.providers.get(m.provider_idx))
            .map(|p| p.api_type.is_routable() && p.endpoint.to_lowercase().contains("openrouter"))
            .unwrap_or(false)
    }

    /// The OAuth uuid backing the model modal's selected provider, or `""` when it
    /// is a plain (non-OAuth) provider / no modal is open. Threaded onto the
    /// catalogue-fetch `Conn` so `fresh_key` can refresh a near-expiry token.
    pub fn mm_provider_oauth_uuid(&self) -> String {
        match self.mm_selected_provider() {
            Some(SelectedProvider::OAuth(d)) => d.uuid.clone(),
            _ => String::new(),
        }
    }

    /// `true` when the model modal's selected provider is the Codex OAuth draft —
    /// gates the static-list short-circuit (no network catalogue for Codex).
    pub fn mm_selected_is_codex(&self) -> bool {
        matches!(
            self.mm_selected_provider(),
            Some(SelectedProvider::OAuth(d)) if d.provider == crate::model::app_config::OAuthProvider::Codex
        )
    }

    /// `true` when the model modal's selected provider is an OAuth draft whose
    /// `registry::meta(provider).catalogue_endpoint` is empty (no network
    /// catalogue) and it is NOT the Codex case (Codex has its own dedicated
    /// static-list mechanism — [`Self::mm_selected_is_codex`] /
    /// `codex_static_catalogue`). Gates the curated
    /// `catalogue_overlay::models_for_provider` short-circuit for the OTHER
    /// empty-catalogue-endpoint OAuth providers (currently ClaudeAI, KomaRun,
    /// Extension): same idea as Codex's static list — an instant local
    /// candidate list, no network fetch, no "searching models…" spinner.
    pub fn mm_selected_is_static_overlay(&self) -> bool {
        match self.mm_selected_provider() {
            Some(SelectedProvider::OAuth(d)) => {
                d.provider != crate::model::app_config::OAuthProvider::Codex
                    && crate::service::oauth::registry::meta(d.provider)
                        .catalogue_endpoint
                        .is_empty()
            }
            _ => false,
        }
    }

    /// The curated overlay catalogue for the model modal's currently selected
    /// provider, when [`Self::mm_selected_is_static_overlay`] is true; an empty
    /// vec otherwise. Mirrors `codex_static_catalogue()`'s role for Codex, but
    /// sourced from the shared `models.json` curated table
    /// (`catalogue_overlay::models_for_provider`) instead of a hardcoded list.
    ///
    /// An empty result here (the overlay has no entries for this provider) is a
    /// legitimate "no matches" state, NOT "still fetching" — callers must treat
    /// a static-overlay provider's cache as always resolved (matched), same as
    /// they do for Codex.
    pub fn mm_static_overlay_catalogue(&self) -> Vec<crate::dto::openrouter::ModelInfo> {
        match self.mm_selected_provider() {
            Some(SelectedProvider::OAuth(d)) if self.mm_selected_is_static_overlay() => {
                crate::service::catalogue_overlay::models_for_provider(d.provider)
            }
            _ => Vec::new(),
        }
    }

    /// Display label for provider index `idx` as stored on a `ModelDraft`/`ModelEntry`
    /// (`provider_idx`, independent of any open modal): a real provider's name (em-
    /// dash if blank), or an OAuth draft's label when `idx` is beyond the providers
    /// list. `None` when `idx` resolves to neither (dangling / stale index).
    pub fn provider_label_at(&self, idx: usize) -> Option<&str> {
        let n = self.providers.len();
        if idx < n {
            self.providers
                .get(idx)
                .map(|p| p.name.as_str())
                .filter(|s| !s.is_empty())
        } else {
            self.oauth_drafts.get(idx - n).map(|d| d.label.as_str())
        }
    }

    /// Label for a model draft's provider binding. Prefers the authoritative
    /// `provider_uuid` (so an OAuth model whose load-time index fell back to 0
    /// still shows the OAuth label, not `providers[0]` / koma free), then the
    /// positional index, then an em-dash.
    pub fn provider_label_for_draft(&self, m: &crate::app::mode::settings::ModelDraft) -> String {
        if !m.provider_uuid.is_empty() {
            if let Some(p) = self.providers.iter().find(|p| p.uuid == m.provider_uuid) {
                if !p.name.is_empty() {
                    return p.name.clone();
                }
            }
            if let Some(d) = self.oauth_drafts.iter().find(|d| d.uuid == m.provider_uuid) {
                return d.label.clone();
            }
        }
        self.provider_label_at(m.provider_idx)
            .unwrap_or("\u{2014}")
            .to_string()
    }

    /// Resolve a positional provider index (merged providers-then-oauth cycle) to
    /// the underlying provider / OAuth-connection uuid. `None` when `idx` is out
    /// of range. Used when committing a model-modal edit so the draft's
    /// authoritative `provider_uuid` stays in sync with the navigated index.
    pub fn provider_uuid_at(&self, idx: usize) -> Option<String> {
        let n = self.providers.len();
        if idx < n {
            self.providers.get(idx).map(|p| p.uuid.clone())
        } else {
            self.oauth_drafts.get(idx - n).map(|d| d.uuid.clone())
        }
    }

    /// Display label for the model modal's CURRENTLY selected provider (real name,
    /// em-dash placeholder, or OAuth draft label). `None` when no modal is open.
    pub fn mm_provider_label(&self) -> Option<String> {
        match self.mm_selected_provider()? {
            SelectedProvider::Provider(p) => Some(if p.name.is_empty() {
                "\u{2014}".to_string()
            } else {
                p.name.clone()
            }),
            SelectedProvider::OAuth(d) => Some(d.label.clone()),
        }
    }

    /// The fields the model modal exposes right now, in navigation order.
    ///
    /// Always `Name, Provider, Model, …, Save, Cancel`. A `Route` field is
    /// inserted when the provider is OpenRouter AND a model is selected (so the
    /// user can pin an upstream provider or leave it on Auto); a `Role` field is
    /// inserted in EDIT mode. The modal's `field` index addresses into this vec.
    ///
    /// The save scope (global / session-local) is determined by which add button
    /// opened the modal (`session_only` on `ModelModal`), so there is only one
    /// `Save` button — no `SaveSession`.
    pub fn model_modal_fields(&self) -> Vec<ModelField> {
        let mut v = vec![ModelField::Name, ModelField::Provider, ModelField::Model];
        if let Some(m) = &self.model_modal {
            if self.mm_provider_is_openrouter() && !m.model_id.is_empty() {
                v.push(ModelField::Route);
            }
            v.push(ModelField::Role);
        }
        v.push(ModelField::Save);
        v.push(ModelField::Cancel);
        v
    }

    /// The [`ModelField`] currently focused in the model modal, or `None` when
    /// no modal is open (or `field` somehow points past the computed list).
    pub fn mm_current_field(&self) -> Option<ModelField> {
        let m = self.model_modal.as_ref()?;
        self.model_modal_fields().get(m.field).copied()
    }

    /// The number of Route options (Auto + one per fetched endpoint). `1` when
    /// no endpoints are loaded (just the Auto entry).
    pub fn mm_route_option_count(&self) -> usize {
        1 + self
            .model_modal
            .as_ref()
            .and_then(|m| m.endpoints.as_ref())
            .map(|e| e.len())
            .unwrap_or(0)
    }

    /// Move focus up one field (clamps at 0).
    pub fn mm_up(&mut self) {
        if let Some(m) = self.model_modal.as_mut() {
            m.field = m.field.saturating_sub(1);
        }
    }

    /// Move focus down one field (clamps at the last computed field).
    pub fn mm_down(&mut self) {
        let max = self.model_modal_fields().len().saturating_sub(1);
        if let Some(m) = self.model_modal.as_mut() {
            m.field = (m.field + 1).min(max);
        }
    }

    /// `true` when provider slot `idx` is a REAL provider (not an OAuth draft)
    /// whose `api_type` is `KomaFree`. Used to skip koma-free slots during
    /// Provider-field navigation ONLY — the underlying `providers` vec and its
    /// index space are never touched, so `provider_idx → provider_uuid` saving
    /// (see `actions/settings.rs::to_entry`) stays correct.
    fn is_koma_free_at(&self, idx: usize) -> bool {
        idx < self.providers.len() && self.providers[idx].api_type == ApiType::KomaFree
    }

    /// Move left in the model modal, dispatching on the focused field:
    /// - Provider → cycle provider backward (wrapping, resets search), skipping
    ///   any koma-free provider slot (nav-only — see [`Self::is_koma_free_at`]),
    ///   then re-clamp `field` since the Route field may appear/disappear.
    /// - Save/Cancel → step left within the button group, clamping at Save.
    /// - everything else (Name/Model/Route) → no-op.
    ///
    /// The Role field is NOT handled here: Enter on it opens the Role checkbox
    /// picker overlay ([`Self::open_role_picker`]); ←→ do nothing on that field.
    pub fn mm_left(&mut self) {
        let n = self.providers.len() + self.oauth_drafts.len();
        match self.mm_current_field() {
            Some(ModelField::Provider) => {
                if n > 0 {
                    // Compute the landing index against `&self` first (borrow
                    // of self.model_modal must not overlap with the
                    // is_koma_free_at(&self) calls below).
                    let start = self.model_modal.as_ref().map_or(0, |m| m.provider_idx);
                    let mut idx = (start + n - 1) % n;
                    // Keep stepping backward past koma-free slots. Cap at
                    // `n` iterations so an all-koma-free config can't hang;
                    // if that cap is hit, every slot is koma-free — leave
                    // the index at its pre-call value instead.
                    let mut steps = 0;
                    while self.is_koma_free_at(idx) {
                        steps += 1;
                        if steps >= n {
                            idx = start;
                            break;
                        }
                        idx = (idx + n - 1) % n;
                    }
                    if let Some(m) = self.model_modal.as_mut() {
                        m.provider_idx = idx;
                        m.query.clear();
                        m.result_sel = 0;
                    }
                }
                self.mm_clamp_field();
            }
            // Button group: Save → Cancel; Left from Cancel steps back to Save.
            Some(ModelField::Cancel) => {
                self.mm_focus_field(ModelField::Save);
            }
            Some(ModelField::Save) => {
                // Already at the leftmost button — no-op.
            }
            _ => {}
        }
    }

    /// Move right in the model modal, dispatching on the focused field:
    /// - Provider → cycle provider forward (wrapping, resets search), skipping
    ///   any koma-free provider slot (nav-only — see [`Self::is_koma_free_at`]),
    ///   then re-clamp `field` since the Route field may appear/disappear.
    /// - Save/Cancel → step right within the button group, clamping at Cancel.
    /// - everything else (Name/Model/Route) → no-op.
    ///
    /// The Role field is NOT handled here — see [`Self::mm_left`].
    pub fn mm_right(&mut self) {
        let n = self.providers.len() + self.oauth_drafts.len();
        match self.mm_current_field() {
            Some(ModelField::Provider) => {
                if n > 0 {
                    // Compute the landing index against `&self` first (borrow
                    // of self.model_modal must not overlap with the
                    // is_koma_free_at(&self) calls below).
                    let start = self.model_modal.as_ref().map_or(0, |m| m.provider_idx);
                    let mut idx = (start + 1) % n;
                    // Keep stepping forward past koma-free slots. Same cap +
                    // degenerate-fallback as mm_left, mirrored direction.
                    let mut steps = 0;
                    while self.is_koma_free_at(idx) {
                        steps += 1;
                        if steps >= n {
                            idx = start;
                            break;
                        }
                        idx = (idx + 1) % n;
                    }
                    if let Some(m) = self.model_modal.as_mut() {
                        m.provider_idx = idx;
                        m.query.clear();
                        m.result_sel = 0;
                    }
                }
                self.mm_clamp_field();
            }
            // Button group: Save → Cancel; Right from Save steps forward to Cancel.
            Some(ModelField::Save) => {
                self.mm_focus_field(ModelField::Cancel);
            }
            Some(ModelField::Cancel) => {
                // Already at the rightmost button — no-op.
            }
            _ => {}
        }
    }

    /// Point `field` at `target` if it exists in the current computed field list.
    fn mm_focus_field(&mut self, target: ModelField) {
        if let Some(pos) = self.model_modal_fields().iter().position(|f| *f == target) {
            if let Some(m) = self.model_modal.as_mut() {
                m.field = pos;
            }
        }
    }

    /// Clamp `field` to the current computed field list (used after the Route
    /// field appears/disappears from a provider change).
    fn mm_clamp_field(&mut self) {
        let max = self.model_modal_fields().len().saturating_sub(1);
        if let Some(m) = self.model_modal.as_mut() {
            if m.field > max {
                m.field = max;
            }
        }
    }

    /// Move the Route option cursor up (clamps at 0). No-op unless the Route
    /// field is focused.
    pub fn mm_route_up(&mut self) {
        if self.mm_current_field() != Some(ModelField::Route) {
            return;
        }
        if let Some(m) = self.model_modal.as_mut() {
            m.route_sel = m.route_sel.saturating_sub(1);
        }
    }

    /// Move the Route option cursor down (clamps at the last option). No-op
    /// unless the Route field is focused.
    pub fn mm_route_down(&mut self) {
        if self.mm_current_field() != Some(ModelField::Route) {
            return;
        }
        let max = self.mm_route_option_count().saturating_sub(1);
        if let Some(m) = self.model_modal.as_mut() {
            m.route_sel = (m.route_sel + 1).min(max);
        }
    }

    /// Commit the highlighted Route option to `route`: option 0 = Auto (`None`);
    /// option `i` pins `endpoints[i-1]`'s provider name (fallback `name`). Stays
    /// on the Route field. No-op unless the Route field is focused.
    pub fn mm_route_commit(&mut self) {
        if self.mm_current_field() != Some(ModelField::Route) {
            return;
        }
        if let Some(m) = self.model_modal.as_mut() {
            if m.route_sel == 0 {
                m.route = None;
            } else if let Some(eps) = m.endpoints.as_ref() {
                if let Some(ep) = eps.get(m.route_sel - 1) {
                    let pick = ep
                        .provider_name
                        .clone()
                        .filter(|s| !s.is_empty())
                        .or_else(|| ep.name.clone().filter(|s| !s.is_empty()));
                    // Only commit when we actually resolved a name; otherwise
                    // leave the existing route untouched (skip).
                    if pick.is_some() {
                        m.route = pick;
                    }
                }
            }
        }
    }

    /// Append `c` to the active model-modal text field: Name → name; Model → the
    /// omnisearch query when the provider has a (non-empty) endpoint to search,
    /// else the raw model id. The Route/Role/button fields ignore typed chars.
    pub fn mm_push_char(&mut self, c: char) {
        let or = self.mm_provider_omnisearchable();
        match self.mm_current_field() {
            Some(ModelField::Name) => {
                if let Some(m) = self.model_modal.as_mut() {
                    m.name.push(c);
                }
            }
            Some(ModelField::Model) => {
                if let Some(m) = self.model_modal.as_mut() {
                    if or {
                        m.query.push(c);
                        m.result_sel = 0;
                    } else {
                        m.model_id.push(c);
                    }
                }
            }
            _ => {}
        }
    }

    /// Delete the last char of the active model-modal text field (mirrors
    /// [`Self::mm_push_char`]).
    pub fn mm_backspace(&mut self) {
        let or = self.mm_provider_omnisearchable();
        match self.mm_current_field() {
            Some(ModelField::Name) => {
                if let Some(m) = self.model_modal.as_mut() {
                    m.name.pop();
                }
            }
            Some(ModelField::Model) => {
                if let Some(m) = self.model_modal.as_mut() {
                    if or {
                        m.query.pop();
                        if m.query.is_empty() {
                            m.result_sel = 0;
                        }
                    } else {
                        m.model_id.pop();
                    }
                }
            }
            _ => {}
        }
    }

    /// Commit the chosen `model_id` from the omnisearch: set it on the modal,
    /// clear the query/selection, and arm the provider-endpoints load. The flags
    /// (`endpoints = None`, `endpoints_loading = true`, `endpoints_for = id`) make
    /// the UI show "loading providers…" immediately; the input layer returns
    /// [`Action::FetchModelEndpoints`](crate::controller::input::Action::FetchModelEndpoints)
    /// so the runtime spawns the actual fetch.
    pub fn mm_select_model(&mut self, model_id: String) {
        if let Some(m) = self.model_modal.as_mut() {
            m.model_id = model_id.clone();
            m.query.clear();
            m.result_sel = 0;
            // A different model has different upstream providers — reset the
            // route choice back to Auto so a stale pin can't carry over.
            m.route = None;
            m.route_sel = 0;
            m.endpoints = None;
            m.endpoints_loading = true;
            m.endpoints_for = Some(model_id);
        }
    }

    /// Commit a chosen `model_id` from the omnisearch WITHOUT arming the
    /// provider-endpoints machinery. Used when the edited provider is NOT
    /// OpenRouter: those providers have no upstream-route list (the Route field is
    /// hidden), so the endpoints flags must stay untouched rather than leaving a
    /// hidden "loading routes…" state stuck on. Clears the query/route, same as
    /// [`Self::mm_select_model`] minus the `endpoints*` writes.
    pub fn mm_set_model_simple(&mut self, model_id: String) {
        if let Some(m) = self.model_modal.as_mut() {
            m.model_id = model_id;
            m.query.clear();
            m.result_sel = 0;
            m.route = None;
            m.route_sel = 0;
        }
    }

    /// Arm a provider-endpoints load for the OPEN model modal, returning the
    /// model id to fetch (so the input layer can hand it to
    /// [`Action::FetchModelEndpoints`](crate::controller::input::Action::FetchModelEndpoints)).
    ///
    /// Used by the edit-open path: when an existing model is opened for edit and
    /// its provider is OpenRouter with a non-empty `model_id`, this sets the
    /// loading flags (`endpoints = None`, `endpoints_loading = true`,
    /// `endpoints_for = id`) so the UI shows "loading providers…" at once, and
    /// returns `Some(id)`. Returns `None` (and changes nothing) when no modal is
    /// open, the provider isn't OpenRouter, or the model id is empty — those
    /// cases have no endpoints API to call.
    pub fn mm_arm_endpoints_load(&mut self) -> Option<String> {
        if !self.mm_provider_is_openrouter() {
            return None;
        }
        let m = self.model_modal.as_mut()?;
        let id = m.model_id.trim().to_string();
        if id.is_empty() {
            return None;
        }
        m.endpoints = None;
        m.endpoints_loading = true;
        m.endpoints_for = Some(id.clone());
        Some(id)
    }

    /// The current `route_sel` index in the model modal (0 when no modal is
    /// open). Used by the input handler to decide whether Up/Down should move
    /// within the Route list or escape to the adjacent field.
    pub fn mm_route_sel(&self) -> usize {
        self.model_modal.as_ref().map(|m| m.route_sel).unwrap_or(0)
    }

    /// The current omnisearch query (empty string when no modal is open). Lets
    /// the input handler compute the result set against the model cache.
    pub fn mm_query(&self) -> &str {
        self.model_modal
            .as_ref()
            .map(|m| m.query.as_str())
            .unwrap_or("")
    }

    /// Move the omnisearch result cursor up (clamps at 0).
    pub fn mm_result_up(&mut self) {
        if let Some(m) = self.model_modal.as_mut() {
            m.result_sel = m.result_sel.saturating_sub(1);
        }
    }

    /// Move the omnisearch result cursor down, clamped to `max` (the last valid
    /// result index = `results.len().saturating_sub(1)`).
    pub fn mm_result_down(&mut self, max: usize) {
        if let Some(m) = self.model_modal.as_mut() {
            m.result_sel = (m.result_sel + 1).min(max);
        }
    }
}

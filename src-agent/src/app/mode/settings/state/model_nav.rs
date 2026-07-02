//! Model modal navigation, provider queries, omnisearch, and endpoints helpers
//! for [`SettingsState`].

use super::super::ModelField;
use super::SettingsState;

impl SettingsState {
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

    /// `true` when the modal's selected provider has a non-empty endpoint, so its
    /// `/models` catalogue can be fetched and the Model field becomes a live
    /// omnisearch. `false` when no modal is open, the index is out of range, or the
    /// provider's endpoint is blank (then the Model field is a plain text box).
    pub fn mm_provider_omnisearchable(&self) -> bool {
        self.model_modal
            .as_ref()
            .and_then(|m| self.providers.get(m.provider_idx))
            .map(|p| !p.endpoint.trim().is_empty())
            .unwrap_or(false)
    }

    /// The edited provider's `(endpoint, api_key)` for the on-demand catalogue
    /// fetch, or `None` when no modal is open / the provider index is out of range.
    /// The input handler hands these to `AppStateRest::request_catalogue`.
    pub fn mm_provider_conn(&self) -> Option<(String, String)> {
        self.model_modal
            .as_ref()
            .and_then(|m| self.providers.get(m.provider_idx))
            .map(|p| (p.endpoint.clone(), p.api_key.clone()))
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
            .map(|p| {
                p.api_type.is_routable() && p.endpoint.to_lowercase().contains("openrouter")
            })
            .unwrap_or(false)
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
            if m.is_edit() {
                v.push(ModelField::Role);
            }
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

    /// Move left in the model modal, dispatching on the focused field:
    /// - Provider → cycle provider backward (wrapping, resets search), then
    ///   re-clamp `field` since the Route field may appear/disappear.
    /// - Save/Cancel → step left within the button group, clamping at Save.
    /// - everything else (Name/Model/Route) → no-op.
    ///
    /// The Role field is NOT handled here: Enter on it opens the Role checkbox
    /// picker overlay ([`Self::open_role_picker`]); ←→ do nothing on that field.
    pub fn mm_left(&mut self) {
        let n = self.providers.len();
        match self.mm_current_field() {
            Some(ModelField::Provider) => {
                if let Some(m) = self.model_modal.as_mut() {
                    if n > 0 {
                        m.provider_idx = (m.provider_idx + n - 1) % n;
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
    /// - Provider → cycle provider forward (wrapping, resets search), then
    ///   re-clamp `field` since the Route field may appear/disappear.
    /// - Save/Cancel → step right within the button group, clamping at Cancel.
    /// - everything else (Name/Model/Route) → no-op.
    ///
    /// The Role field is NOT handled here — see [`Self::mm_left`].
    pub fn mm_right(&mut self) {
        let n = self.providers.len();
        match self.mm_current_field() {
            Some(ModelField::Provider) => {
                if let Some(m) = self.model_modal.as_mut() {
                    if n > 0 {
                        m.provider_idx = (m.provider_idx + 1) % n;
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
        self.model_modal.as_ref().map(|m| m.query.as_str()).unwrap_or("")
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


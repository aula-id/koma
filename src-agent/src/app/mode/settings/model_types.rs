//! Model-related types: [`ModelField`], [`ModelDraft`], [`ModelModal`],
//! [`RolePickerState`], [`ModelFilterMode`], and the [`filter_models`] omnisearch helper.

use super::provider_types::{ModelRole, new_uuid};

/// Scope filter for the models list display.
///
/// Controls which model entries are shown in the table. Cycled with Left/Right
/// when focus is on the Models Select screen. Does not affect persistence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelFilterMode {
    All,
    Local,
    Global,
}

impl ModelFilterMode {
    /// Does a model with the given `session_only` flag pass this filter?
    pub fn matches(self, session_only: bool) -> bool {
        match self {
            Self::All    => true,
            Self::Local  => session_only,
            Self::Global => !session_only,
        }
    }
}

/// Number of fixed control slots before the data rows in the models list:
/// 0=add global, 1=add local, 2=filter all, 3=filter local, 4=filter global.
///
/// Single source of truth used by BOTH input dispatch and the view renderer.
pub const MODEL_CTRL_SLOTS: usize = 5;

/// Which logical element the `model_sel` index points at in the Models Select
/// screen. Used by both the daemon input handler and the client renderer to
/// guarantee they agree on the slot layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelRowSel {
    AddGlobal,
    AddLocal,
    FilterAll,
    FilterLocal,
    FilterGlobal,
    /// Real index into `SettingsState::models`.
    Data(usize),
}

/// A single navigable field within the add/edit-model modal.
///
/// The concrete field SET is computed at runtime by
/// [`SettingsState::model_modal_fields`] because two fields are conditional:
/// `Route` appears only for an OpenRouter provider with a model selected, and
/// `Role` appears only in EDIT mode. The modal's `field` index addresses into
/// that computed `Vec<ModelField>`, so there are no hardcoded layout indices.
///
/// The save scope (global vs session-local) is now determined by which add
/// button opened the modal (`session_only` on `ModelModal`), not by a separate
/// `SaveSession` button — so there is only one `Save` button in the modal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelField {
    Name,
    Provider,
    Model,
    Route,
    Role,
    Save,
    Cancel,
}

/// State for the role multi-select picker overlay (model EDIT modal).
///
/// A bordered checkbox modal mirroring the `/agents` tool picker, but simpler:
/// the option set is fixed ([`ModelRole::ALL`], 4 entries) so there is NO text
/// filter. Opened from the model modal's Role field (Enter); closed by Enter
/// (confirm → write `selected_roles()` back into `ModelModal::roles`) or Esc
/// (cancel → discard). `checked` is parallel to `ModelRole::ALL`.
#[derive(Clone, Debug)]
pub struct RolePickerState {
    /// Parallel to [`ModelRole::ALL`]; `true` = this role is currently checked.
    pub checked: Vec<bool>,
    /// Highlighted row, in `0..ModelRole::ALL.len()`.
    pub cursor: usize,
}

impl RolePickerState {
    /// Seed the checkbox state from the model's current `roles`: each option is
    /// pre-checked when its [`ModelRole`] appears in `roles`.
    pub fn from_roles(roles: &[ModelRole]) -> Self {
        let checked: Vec<bool> = ModelRole::ALL
            .iter()
            .map(|r| roles.contains(r))
            .collect();
        Self { checked, cursor: 0 }
    }

    /// Move the cursor up (clamps at 0).
    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor down (clamps at the last role).
    pub fn down(&mut self) {
        if self.cursor + 1 < ModelRole::ALL.len() {
            self.cursor += 1;
        }
    }

    /// Flip the checked state of the role under the cursor.
    pub fn toggle(&mut self) {
        let i = self.cursor.min(ModelRole::ALL.len() - 1);
        self.checked[i] = !self.checked[i];
    }

    /// The checked roles, in [`ModelRole::ALL`] order.
    pub fn selected_roles(&self) -> Vec<ModelRole> {
        ModelRole::ALL
            .iter()
            .zip(self.checked.iter())
            .filter(|(_, &c)| c)
            .map(|(r, _)| *r)
            .collect()
    }
}

/// One model entry stored in-memory (stub only, not persisted). Maps a custom
/// display name to a concrete `model_id`, served by the API provider at
/// `provider_idx` in [`SettingsState::providers`].
#[derive(Clone, Debug)]
pub struct ModelDraft {
    /// Persisted identity (matches the `ModelEntry` uuid). Minted on create;
    /// preserved on edit so a saved model keeps its catalogue identity.
    pub uuid: String,
    /// Custom display name shown in the models table.
    pub name: String,
    /// Concrete model identifier, e.g. `"openai/gpt-4o-mini"`.
    pub model_id: String,
    /// Index into [`SettingsState::providers`] — which API provider serves it.
    pub provider_idx: usize,
    /// Role slots assigned to this model; empty = unassigned. A model may hold
    /// several roles (each role is globally unique across models).
    pub roles: Vec<ModelRole>,
    /// Pinned upstream provider for OpenRouter routing: the chosen endpoint's
    /// provider name. `None` = Auto (let OpenRouter route). Only meaningful for
    /// OpenRouter-served models; ignored for other providers.
    pub route: Option<String>,
    /// `true` = saved for this session only (not persisted globally);
    /// `false` = global scope.
    pub session_only: bool,
}

/// State for the "Add / Edit model" modal overlay.
///
/// When the chosen provider is OpenRouter and the Model field is focused, the
/// modal hosts a live omnisearch over the cached model catalogue (`query` +
/// `result_sel`). Selecting/opening an OpenRouter model arms the `endpoints*`
/// fields: a background fetch loads the model's provider endpoints, which the
/// view renders as a read-only providers list (price + uptime per provider).
///
/// The field layout is computed (never hardcoded): `field` indexes into the
/// `Vec<ModelField>` returned by [`SettingsState::model_modal_fields`]. The base
/// run is `Name, Provider, Model`; a `Route` field is inserted (OpenRouter +
/// model selected) and a `Role` field is inserted (EDIT mode); `Save, Cancel`
/// always close the list. Resolve the focused field with
/// [`SettingsState::mm_current_field`] instead of comparing raw indices.
#[derive(Clone, Debug)]
pub struct ModelModal {
    /// `Some(i)` = editing `models[i]`; `None` = adding a new entry.
    pub editing_idx: Option<usize>,
    /// Persisted identity carried through the modal: minted fresh in
    /// [`Self::new_add`], cloned from the edited draft on edit-open, and written
    /// back onto the saved [`ModelDraft`]. Empty falls back to a freshly minted
    /// uuid on save.
    pub uuid: String,
    /// Draft custom display name.
    pub name: String,
    /// Index into [`SettingsState::providers`] (which provider serves the model).
    pub provider_idx: usize,
    /// Draft concrete model id.
    pub model_id: String,
    /// Active field index (see layout comment above).
    pub field: usize,
    /// Draft role assignments; empty = unassigned. A model may hold several
    /// roles. Only editable in EDIT mode, via the Role checkbox picker overlay
    /// (the committed value — the picker writes its selection back here on OK).
    pub roles: Vec<ModelRole>,
    /// When `Some`, the Role checkbox picker overlay is open over this modal.
    /// All key input routes to the picker; the modal underneath is frozen until
    /// it confirms (Enter → commit into `roles`) or cancels (Esc → discard).
    pub role_picker: Option<RolePickerState>,
    /// Model omnisearch query (used when provider is OpenRouter and the Model
    /// field is focused).
    pub query: String,
    /// Highlighted row in the omnisearch results list.
    pub result_sel: usize,
    /// Pinned upstream provider for OpenRouter routing: the chosen endpoint's
    /// provider name. `None` = Auto (let OpenRouter route). Mirrors
    /// [`ModelDraft::route`]; committed into the draft on save.
    pub route: Option<String>,
    /// Cursor into the Route options list (0 = Auto; 1..=N index `endpoints`).
    pub route_sel: usize,
    // --- endpoints area: per-model provider list (display only) ---
    /// Fetched per-model provider endpoints. `None` until a fetch resolves;
    /// `Some(vec)` once loaded (an empty vec means "no providers found", also
    /// used to resolve a failed fetch). Rendered by `draw_model_modal`.
    pub endpoints: Option<Vec<crate::dto::openrouter::ModelEndpoint>>,
    /// `true` while the endpoints fetch is in flight (the view shows "loading
    /// providers…"); cleared when the fetch resolves.
    pub endpoints_loading: bool,
    /// The model id the in-flight / cached `endpoints` belong to. Used as a
    /// stale-guard in the drain so a rapid re-selection can't show a previous
    /// model's providers.
    pub endpoints_for: Option<String>,
    /// Scope chosen at add time (or inherited from the edited draft): `false` =
    /// global, `true` = session-local. The single Save button reads this to
    /// determine the persistence scope (set by which add button opened the modal).
    pub session_only: bool,
}

impl ModelModal {
    /// Blank ADD-mode modal targeting provider `provider_idx`.
    ///
    /// `session_only` pre-selects the save scope: `false` = global (opened via
    /// `[+add global]`), `true` = session-local (opened via `[+add local]`).
    /// Mints a fresh uuid so a model added in this session has a stable identity
    /// before save.
    pub fn new_add(provider_idx: usize, session_only: bool) -> Self {
        Self {
            editing_idx: None,
            uuid: new_uuid(),
            name: String::new(),
            provider_idx,
            model_id: String::new(),
            field: 0,
            roles: Vec::new(),
            role_picker: None,
            query: String::new(),
            result_sel: 0,
            route: None,
            route_sel: 0,
            endpoints: None,
            endpoints_loading: false,
            endpoints_for: None,
            session_only,
        }
    }
}

/// Filter the cached model catalogue by a (case-insensitive) substring match on
/// the model `id` or its human-readable `name`, returning up to 50 catalogue
/// indices. Used to drive the modal's model omnisearch.
pub fn filter_models(cache: &[crate::dto::openrouter::ModelInfo], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    cache
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.id.to_lowercase().contains(&q)
                || m.name
                    .as_deref()
                    .map(|n| n.to_lowercase().contains(&q))
                    .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .take(50)
        .collect()
}

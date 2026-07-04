//! Working state for the in-app `/settings` dashboard.
//!
//! Split into submodules for readability:
//! - [`path_ops`]    — path-list management and filesystem picker operations
//! - [`provider_ops`] — API Providers screen helpers
//! - [`model_ops`]   — Models Select screen: open/save/delete modal
//! - [`model_nav`]   — Models Select screen: modal navigation and text input

mod path_ops;
mod provider_ops;
mod model_ops;
mod model_nav;

use std::path::PathBuf;

use crate::model::app_config::{AppConfig, ThemeMode};
use crate::model::session::Session;
use crate::model::settings::InternetMode;
use crate::view::theme::ACCENTS;

use super::super::SettingField;
use super::picker::PathPicker;
use super::{ModelDraft, ModelFilterMode, ModelModal, ProviderDraft, ProviderModal};

/// Working state for the in-app `/settings` dashboard.
///
/// Holds editable *drafts* of every settable value; nothing is persisted until
/// the user saves (Esc from the sidebar), at which point the runtime reads these
/// fields back out and applies them.
///
/// Navigation is now THREE-level inside the detail pane for the path-list fields
/// (Workdir, Allowed dirs): `cat` selects a category in the sidebar; `field`
/// selects a row within the category's detail list; for a path-list field,
/// `list_editing` enters per-entry management (`list_sel` highlights a row) and a
/// `picker` overlay drives add/replace via the real filesystem. `in_detail`
/// tracks which pane has keyboard focus. `editing` means typing into a plain
/// text field.
#[derive(Debug, Clone)]
pub struct SettingsState {
    /// Selected category index into [`SETTING_CATEGORIES`](super::SETTING_CATEGORIES).
    pub cat: usize,
    /// Selected field index within `SETTING_CATEGORIES[cat].fields`.
    pub field: usize,
    /// `false` = focus on the sidebar; `true` = focus on the detail field list.
    pub in_detail: bool,
    /// `true` while typing into a text field; `false` while navigating.
    pub editing: bool,
    /// Draft API key (session-scoped).
    pub api_key: String,
    /// Draft OpenRouter model identifier.
    pub model: String,
    /// Draft OpenRouter provider slug (may be empty for default routing).
    pub provider: String,
    /// Draft session display name (applied via `rename_session` on save).
    pub name: String,
    /// Draft global theme mode.
    pub theme: ThemeMode,
    /// Draft global accent name (one of [`ACCENTS`]).
    pub accent: String,
    /// Draft working-directory path list for this session (min 1 entry on save).
    pub workdir: Vec<String>,
    /// Draft: project-awareness summary enabled.
    pub awareness_enabled: bool,
    /// Draft: awareness model source — `true` = inherit the session model,
    /// `false` = use the dedicated awareness model/provider below.
    pub awareness_inherit: bool,
    /// Draft: dedicated awareness model (used when `awareness_inherit` is false).
    pub awareness_model: String,
    /// Draft: dedicated awareness provider (used when `awareness_inherit` is false).
    pub awareness_provider: String,
    /// Draft: safety-harness master switch.
    pub classifier_enabled: bool,
    /// Draft: safety-classifier model.
    pub classifier_model: String,
    /// Draft: safety-classifier provider slug.
    pub classifier_provider: String,
    /// Draft: extra allowed folders as a managed path list. Seeded from
    /// `settings.allowed_folders` (or the launch cwd when empty) and written back
    /// to `Vec<String>` (trim, drop empties) on save.
    pub allowed_folders: Vec<String>,
    /// Draft: short-send token-saver master switch.
    pub short_send_enabled: bool,
    /// Draft: cache-warmth-adaptive summarization toggle.
    pub sliding_cache: bool,
    /// Draft: bash output saving (filtered + tee-to-disk) toggle.
    pub bash_saving: bool,
    /// Draft: internet-access tier toggle.
    pub internet_mode: InternetMode,
    /// The session's effective working directory, captured at construction. Used
    /// as the base for resolving workspace-relative paths in the FS picker.
    pub cwd: PathBuf,
    /// `true` when the user has entered a path-list field to manage its entries
    /// (one nesting level below field navigation, above the picker).
    pub list_editing: bool,
    /// Highlighted entry row within the active path list (while `list_editing`).
    pub list_sel: usize,
    /// Active filesystem directory picker overlay, if any. When `Some` it has
    /// keyboard focus (deepest nesting level) until confirmed or cancelled.
    pub picker: Option<PathPicker>,
    /// In-memory list of API provider drafts (stub only, not persisted).
    pub providers: Vec<ProviderDraft>,
    /// Selected row in the providers list. Index == `providers.len()` means the
    /// `[+ add]` button row is highlighted.
    pub prov_sel: usize,
    /// `true` after the first Ctrl+X: next Ctrl+X confirms the delete.
    pub prov_delete_armed: bool,
    /// Active add-provider modal, if open.
    pub prov_modal: Option<ProviderModal>,
    /// In-memory list of model drafts (stub only, not persisted).
    pub models: Vec<ModelDraft>,
    /// Selected row in the models list (operates over the VISIBLE filtered set).
    /// `0 .. visible_model_count()` selects a data row;
    /// `visible_model_count()` highlights `[+add global]`;
    /// `visible_model_count() + 1` highlights `[+add local]`.
    pub model_sel: usize,
    /// `true` after the first Ctrl+X on a model row: next Ctrl+X confirms.
    pub model_delete_armed: bool,
    /// Active add/edit-model modal, if open.
    pub model_modal: Option<ModelModal>,
    /// Current scope filter for the models table display. Cycled with Left/Right.
    pub model_filter: ModelFilterMode,
}

impl SettingsState {
    /// Build a dashboard pre-populated from the active session and global config.
    ///
    /// Text drafts come from `session.settings` (and `session.name`); the
    /// theme/accent drafts come from `config`. Starts on the sidebar of the
    /// first category with editing off.
    pub fn from(session: &Session, config: &AppConfig) -> Self {
        let effective_cwd = session.workdir();
        let workdir: Vec<String> = {
            let stored: Vec<String> = session
                .settings
                .workdir
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if stored.is_empty() {
                vec![effective_cwd.display().to_string()]
            } else {
                stored
            }
        };
        let allowed_folders: Vec<String> = if session.settings.allowed_folders.is_empty() {
            std::env::current_dir()
                .map(|p| vec![p.display().to_string()])
                .unwrap_or_else(|_| vec![effective_cwd.display().to_string()])
        } else {
            session.settings.allowed_folders.clone()
        };
        // Provider drafts come straight from the global catalogue (empty on a
        // fresh install — no demo seeds).
        let providers: Vec<ProviderDraft> = config
            .providers
            .iter()
            .map(|p| ProviderDraft {
                uuid: p.uuid.clone(),
                name: p.name.clone(),
                endpoint: p.endpoint.clone(),
                api_type: p.api_type,
                api_key: p.api_key.clone(),
            })
            .collect();
        // Model drafts: global catalogue entries (session_only = false) followed
        // by this session's override-layer models (session_only = true). Each
        // entry's `provider_uuid` is resolved back to a positional `provider_idx`
        // against the providers built above; a dangling uuid (provider deleted
        // out-of-band) falls back to idx 0 so the row surfaces for re-pick rather
        // than vanishing.
        let map_entry = |m: &crate::model::app_config::ModelEntry, session_only: bool| ModelDraft {
            uuid: m.uuid.clone(),
            name: m.name.clone(),
            model_id: m.model_id.clone(),
            provider_idx: config.provider_index_by_uuid(&m.provider_uuid).unwrap_or(0),
            // Fold the legacy single-role field into the multi-role list on load.
            roles: m.effective_roles(),
            route: m.route.clone(),
            session_only,
        };
        let mut models: Vec<ModelDraft> =
            config.models.iter().map(|m| map_entry(m, false)).collect();
        models.extend(
            session
                .settings
                .session_models
                .iter()
                .map(|m| map_entry(m, true)),
        );
        Self {
            cat: 0,
            field: 0,
            in_detail: false,
            editing: false,
            api_key: session.settings.api_key.clone(),
            model: session.settings.model.clone(),
            provider: session.settings.provider.clone(),
            name: session.name.clone(),
            theme: config.theme.clone(),
            accent: config.accent.clone(),
            workdir,
            awareness_enabled: session.settings.awareness_enabled,
            awareness_inherit: session.settings.awareness_inherit,
            awareness_model: session.settings.awareness_model.clone(),
            awareness_provider: session.settings.awareness_provider.clone(),
            classifier_enabled: session.settings.classifier_enabled,
            classifier_model: session.settings.classifier_model.clone(),
            classifier_provider: session.settings.classifier_provider.clone(),
            allowed_folders,
            short_send_enabled: session.settings.short_send_enabled,
            sliding_cache: session.settings.sliding_cache,
            bash_saving: session.settings.bash_saving,
            internet_mode: session.settings.internet_mode,
            cwd: effective_cwd,
            list_editing: false,
            list_sel: 0,
            picker: None,
            providers,
            prov_sel: 0,
            prov_delete_armed: false,
            prov_modal: None,
            models,
            model_sel: 0,
            model_delete_armed: false,
            model_modal: None,
            model_filter: ModelFilterMode::All,
        }
    }

    /// Return the [`SettingField`] currently highlighted in the detail pane.
    pub fn current_field(&self) -> SettingField {
        super::SETTING_CATEGORIES[self.cat].fields[self.field]
    }

    /// Move the cursor up.
    pub fn up(&mut self) {
        if self.in_detail {
            self.field = self.field.saturating_sub(1);
        } else {
            let prev = self.cat;
            self.cat = self.cat.saturating_sub(1);
            if self.cat != prev {
                self.field = 0;
            }
        }
    }

    /// Move the cursor down.
    pub fn down(&mut self) {
        if self.in_detail {
            let max = super::SETTING_CATEGORIES[self.cat].fields.len().saturating_sub(1);
            if self.field < max {
                self.field += 1;
            }
        } else {
            let max = super::SETTING_CATEGORIES.len().saturating_sub(1);
            if self.cat < max {
                self.cat += 1;
                self.field = 0;
            }
        }
    }

    /// Move focus to the detail pane (only if the current category has fields,
    /// or if the category is one of the special interactive screens — API
    /// Providers / Models Select — which carry no [`SettingField`] rows).
    pub fn focus_detail(&mut self) {
        if !super::SETTING_CATEGORIES[self.cat].fields.is_empty()
            || self.is_providers_category()
            || self.is_models_category()
        {
            self.in_detail = true;
            self.field = 0;
        }
    }

    /// Return focus to the sidebar; also exits editing/list/picker modes.
    pub fn focus_sidebar(&mut self) {
        self.in_detail = false;
        self.editing = false;
        self.list_editing = false;
        self.list_sel = 0;
        self.picker = None;
    }

    /// Act on Enter while in the detail pane.
    pub fn enter(&mut self) {
        if !self.in_detail {
            return;
        }
        match self.current_field() {
            SettingField::Theme => {
                self.theme = match self.theme {
                    ThemeMode::Dark  => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
            }
            SettingField::Accent => {
                // Accent is cycled with arrow keys; Enter is intentionally a no-op.
            }
            SettingField::AwarenessEnabled => {
                self.awareness_enabled = !self.awareness_enabled;
            }
            SettingField::AwarenessSource => {
                self.awareness_inherit = !self.awareness_inherit;
            }
            SettingField::AwarenessModel | SettingField::AwarenessProvider => {
                if !self.awareness_inherit {
                    self.editing = true;
                }
            }
            SettingField::ClassifierEnabled => {
                self.classifier_enabled = !self.classifier_enabled;
            }
            SettingField::ShortSendEnabled => {
                self.short_send_enabled = !self.short_send_enabled;
            }
            SettingField::SlidingCache => {
                self.sliding_cache = !self.sliding_cache;
            }
            SettingField::BashSaving => {
                self.bash_saving = !self.bash_saving;
            }
            SettingField::InternetMode => {
                self.internet_mode = self.internet_mode.toggled();
            }
            SettingField::Workdir | SettingField::AllowedFolders => {
                self.list_editing = true;
                self.list_sel = 0;
            }
            _ => {
                self.editing = true;
            }
        }
    }

    /// Append `c` to the draft of the current text field.
    pub fn push_char(&mut self, c: char) {
        let f = self.current_field();
        if let Some(s) = self.text_draft_mut(f) {
            s.push(c);
        }
    }

    /// Delete the last character from the current text field's draft.
    pub fn backspace(&mut self) {
        let f = self.current_field();
        if let Some(s) = self.text_draft_mut(f) {
            s.pop();
        }
    }

    /// Cycle the accent draft to the next/previous entry in [`ACCENTS`], wrapping.
    pub fn cycle_accent(&mut self, forward: bool) {
        let len = ACCENTS.len();
        if len == 0 {
            return;
        }
        let cur = ACCENTS.iter().position(|a| *a == self.accent).unwrap_or(0);
        let next = if forward {
            (cur + 1) % len
        } else {
            (cur + len - 1) % len
        };
        self.accent = ACCENTS[next].to_string();
    }
}

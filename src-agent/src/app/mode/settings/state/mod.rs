//! Working state for the in-app `/settings` dashboard.
//!
//! Split into submodules for readability:
//! - [`path_ops`]    — path-list management and filesystem picker operations
//! - [`provider_ops`] — API Providers screen helpers
//! - [`model_ops`]   — Models Select screen: open/save/delete modal
//! - [`model_nav`]   — Models Select screen: modal navigation and text input

mod path_ops;
mod provider_ops;
mod oauth_ops;
mod model_ops;
mod model_nav;

use std::path::PathBuf;

use crate::model::app_config::{AppConfig, ThemeMode};
use crate::model::session::Session;
use crate::model::settings::InternetMode;
use crate::view::theme::ACCENTS;

use super::super::SettingField;
use super::picker::PathPicker;
use super::{
    ModelDraft, ModelFilterMode, ModelModal, OAuthDraft, OAuthFlowState, ProviderDraft,
    ProviderModal,
};

/// Working state for the in-app `/settings` dashboard.
///
/// Holds editable *drafts* of every settable value; nothing is persisted until
/// the user saves (Esc from the menu), at which point the runtime reads these
/// fields back out and applies them.
///
/// Navigation is now PAGE-BASED: `page` tracks which full-screen page is visible
/// (Menu, Appearance, General, Providers, OAuth, Models, ProviderForm, ModelForm).
/// Esc goes back one level; Esc from the menu saves and closes.
#[derive(Debug, Clone)]
pub struct SettingsState {
    /// Which page is currently shown.
    pub page: super::SettingsPage,
    /// Cursor index on the menu page (0-4, maps to SettingsPage::MENU_ORDER).
    pub menu_sel: usize,
    /// Selected field index within the General page field list.
    pub field: usize,
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
    /// Draft global palette name (one of [`crate::view::theme::PALETTES`]);
    /// mirrors the applied `config.palette` so the Esc-save path writes it back.
    pub palette: String,
    /// Cursor index into [`crate::view::theme::PALETTES`] for the Appearance
    /// palette list (coolors-style picker). Up/Down move it; Enter applies the
    /// cursored palette live. Seeded to the index of `config.palette` in `from`.
    pub palette_sel: usize,
    /// Draft working-directory path list for this session (min 1 entry on save).
    pub workdir: Vec<String>,
    /// Draft: project-awareness summary enabled.
    pub awareness_enabled: bool,
    /// Draft: safety-harness master switch.
    pub classifier_enabled: bool,
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
    /// Draft: GUI Coding panel auto-save toggle.
    pub coding_autosave: bool,
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
    /// OAuth-authenticated connections (Codex / Kilo Code), appended AFTER
    /// `providers` in the model modal's provider cycle. Read-only reflection of
    /// `config.oauth_conns` — the `/settings` OAuth submenu (a later wave) is the
    /// only thing that adds/removes entries from the underlying catalogue.
    pub oauth_drafts: Vec<OAuthDraft>,
    /// Selected row in the OAuth submenu's connections list. Index ==
    /// `oauth_drafts.len()` means the `[+connect]` button row is highlighted.
    pub oauth_sel: usize,
    /// `Some(row)` after the first Ctrl+X on that row: the next Ctrl+X on the
    /// SAME row confirms the delete. Any navigation clears it.
    pub oauth_armed: Option<usize>,
    /// Current step of the OAuth submenu's connect flow (`Idle` = the plain
    /// connection list, no overlay).
    pub oauth_flow: OAuthFlowState,
    /// Selected row in the providers list. Index == `providers.len()` means the
    /// `[+ add]` button row is highlighted.
    pub prov_sel: usize,
    /// `true` after the first Ctrl+X: next Ctrl+X confirms the delete.
    pub prov_delete_armed: bool,
    /// W12b: a transient footer message set when the user tries to delete an
    /// EXTENSION-managed provider (the delete is refused — only uninstall removes it).
    /// Cleared on any navigation.
    pub prov_msg: Option<String>,
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
                // Carry the extension-ownership tag so a save round-trips it and the delete
                // action can refuse an ext-managed provider (W12b).
                ext_id: p.ext_id.clone(),
            })
            .collect();
        // OAuth drafts: read-only-at-load reflection of `config.oauth_conns`,
        // appended AFTER `providers` in the model modal's provider cycle AND shown
        // in the `/settings` OAuth submenu. `OAuthDraft::from_config` is the single
        // builder (also re-run after a submenu login/delete) so the label + status
        // computation never drifts between the two call sites.
        let oauth_drafts: Vec<OAuthDraft> = OAuthDraft::from_config(config);
        // Model drafts: global catalogue entries (session_only = false) followed
        // by this session's override-layer models (session_only = true). Each
        // entry's `provider_uuid` is resolved back to a positional `provider_idx`
        // against the providers built above; a dangling uuid (provider deleted
        // out-of-band) falls back to idx 0 so the row surfaces for re-pick rather
        // than vanishing.
        let map_entry = |m: &crate::model::app_config::ModelEntry, session_only: bool| {
            // Resolve `provider_uuid` against the MERGED provider cycle: real
            // providers first, then OAuth conns offset by `providers.len()`. A
            // dangling uuid (neither resolves) falls back to idx 0 for *display*
            // only — the original `provider_uuid` is preserved on the draft so a
            // later Esc/save cannot silently rebind the model to providers[0]
            // (koma free). See `actions/settings.rs::to_entry`.
            let provider_idx = config
                .provider_index_by_uuid(&m.provider_uuid)
                .or_else(|| {
                    config
                        .oauth_index_by_uuid(&m.provider_uuid)
                        .map(|i| config.providers.len() + i)
                })
                .unwrap_or(0);
            ModelDraft {
                uuid: m.uuid.clone(),
                name: m.name.clone(),
                model_id: m.model_id.clone(),
                provider_idx,
                // Authoritative binding — never drop this on an index miss.
                provider_uuid: m.provider_uuid.clone(),
                // Fold the legacy single-role field into the multi-role list on load.
                roles: m.effective_roles(),
                route: m.route.clone(),
                session_only,
                // Carry the clone-source identity through the settings editor so a
                // save that doesn't touch this model preserves the GUI picker match.
                source_uuid: m.source_uuid.clone(),
            }
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
            page: super::SettingsPage::Menu,
            menu_sel: 0,
            field: 0,
            editing: false,
            api_key: session.settings.api_key.clone(),
            model: session.settings.model.clone(),
            provider: session.settings.provider.clone(),
            name: session.name.clone(),
            theme: config.theme.clone(),
            accent: config.accent.clone(),
            palette: config.palette.clone(),
            // Seed the palette-list cursor to the applied palette so the accent
            // border and the `· selected` tag coincide on open (fallback 0).
            palette_sel: crate::view::theme::PALETTES
                .iter()
                .position(|(n, _)| *n == config.palette)
                .unwrap_or(0),
            workdir,
            awareness_enabled: session.settings.awareness_enabled,
            classifier_enabled: session.settings.classifier_enabled,
            allowed_folders,
            short_send_enabled: session.settings.short_send_enabled,
            sliding_cache: session.settings.sliding_cache,
            bash_saving: session.settings.bash_saving,
            coding_autosave: session.settings.coding_autosave,
            internet_mode: session.settings.internet_mode,
            cwd: effective_cwd,
            list_editing: false,
            list_sel: 0,
            picker: None,
            providers,
            oauth_drafts,
            oauth_sel: 0,
            oauth_armed: None,
            oauth_flow: OAuthFlowState::Idle,
            prov_sel: 0,
            prov_delete_armed: false,
            prov_msg: None,
            prov_modal: None,
            models,
            model_sel: 0,
            model_delete_armed: false,
            model_modal: None,
            model_filter: ModelFilterMode::All,
        }
    }

    /// Return the [`SettingField`] currently highlighted in the General page's
    /// field list.
    pub fn current_field(&self) -> SettingField {
        super::GENERAL_FIELDS[self.field]
    }

    /// Move the field cursor up (General page).
    pub fn up(&mut self) {
        self.field = self.field.saturating_sub(1);
    }

    /// Move the field cursor down (General page).
    pub fn down(&mut self) {
        let max = super::GENERAL_FIELDS.len().saturating_sub(1);
        if self.field < max {
            self.field += 1;
        }
    }

    /// Act on Enter for the current field (General page).
    pub fn enter(&mut self) {
        match self.current_field() {
            SettingField::Accent => {
                // Accent is cycled with arrow keys; Enter is intentionally a no-op.
            }
            SettingField::Palette => {
                // Palette is applied via Up/Down + Enter in the input handler
                // (live-apply needs `config`, which `enter()` cannot reach). The
                // handler intercepts Enter for the Palette field before calling
                // `enter()`, so this arm is intentionally unreachable.
            }
            SettingField::AwarenessEnabled => {
                self.awareness_enabled = !self.awareness_enabled;
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
            SettingField::CodingAutosave => {
                self.coding_autosave = !self.coding_autosave;
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

    /// Move the Appearance palette-list cursor to the PREVIOUS entry in the
    /// palette registry ([`crate::view::theme::PALETTES`]), wrapping. Guards an
    /// empty registry. Enter (in the input handler) applies the cursored palette
    /// live — this only moves the cursor.
    pub fn palette_up(&mut self) {
        let len = crate::view::theme::PALETTES.len();
        if len == 0 {
            return;
        }
        self.palette_sel = (self.palette_sel + len - 1) % len;
    }

    /// Move the Appearance palette-list cursor to the NEXT entry in the palette
    /// registry, wrapping. Guards an empty registry.
    pub fn palette_down(&mut self) {
        let len = crate::view::theme::PALETTES.len();
        if len == 0 {
            return;
        }
        self.palette_sel = (self.palette_sel + 1) % len;
    }
}

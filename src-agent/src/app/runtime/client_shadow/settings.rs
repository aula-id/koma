//! Shadow reconstruction for the `/settings` dashboard and its modals/drafts:
//! the biggest single cluster of [`super::modes`]. Split out for file size —
//! pure code motion, no behaviour change.
//!
//! `shadow_oauth_flow` and `shadow_oauth_provider` are bumped to `pub(super)`
//! (were private) because [`super::modes::shadow_onboard_provider`] — which
//! stays in the sibling `modes` module — also calls them (the guided
//! onboarding wizard reuses the same OAuth connect-flow shadow as Settings).
//! Every other function here is called only within this file.

use crate::app::mode::settings::{
    ModelDraft, ModelModal, OAuthDraft, OAuthFlowState, PathPicker, PickerMode, ProviderDraft,
    ProviderModal, RolePickerState, SettingsState,
};
use crate::dto::openrouter::{ModelEndpoint, ModelPricing};
use crate::ipc::proto::{ModelModalSnapshot, OAuthDraftSnapshot, PathPickerSnapshot, SettingsSnapshot};
use crate::model::app_config::{ApiType, ModelRole, ThemeMode};
use crate::model::settings::{InternetMode, Settings};

/// Rebuild the `/settings` dashboard ([`SettingsState`]) from its projection — the
/// largest reconstruction. Every draft + list + modal + picker is restored so the
/// settings view (and its pure helper methods, which recompute from these same
/// fields) renders exactly as the daemon's would.
pub(crate) fn shadow_settings(s: SettingsSnapshot) -> SettingsState {
    SettingsState {
        cat: s.cat,
        field: s.field,
        in_detail: s.in_detail,
        editing: s.editing,
        api_key: s.api_key,
        model: s.model,
        provider: s.provider,
        name: s.name,
        theme: shadow_theme(&s.theme),
        accent: s.accent,
        palette: s.palette,
        workdir: s.workdir,
        awareness_enabled: s.awareness_enabled,
        awareness_inherit: s.awareness_inherit,
        awareness_model: s.awareness_model,
        awareness_provider: s.awareness_provider,
        classifier_enabled: s.classifier_enabled,
        classifier_model: s.classifier_model,
        classifier_provider: s.classifier_provider,
        allowed_folders: s.allowed_folders,
        short_send_enabled: s.short_send_enabled,
        sliding_cache: s.sliding_cache,
        bash_saving: s.bash_saving,
        coding_autosave: s.coding_autosave,
        internet_mode: shadow_internet_mode(&s.internet_mode),
        cwd: std::path::PathBuf::from(s.cwd),
        list_editing: s.list_editing,
        list_sel: s.list_sel,
        picker: s.picker.map(shadow_path_picker),
        providers: s
            .providers
            .into_iter()
            .map(|p| ProviderDraft {
                uuid: p.uuid,
                name: p.name,
                endpoint: p.endpoint,
                api_type: shadow_api_type(&p.api_type),
                api_key: p.api_key,
                ext_id: p.ext_id,
            })
            .collect(),
        oauth_drafts: s
            .oauth_drafts
            .into_iter()
            .map(shadow_oauth_draft)
            .collect(),
        oauth_sel: s.oauth_sel,
        oauth_armed: s.oauth_armed,
        oauth_flow: shadow_oauth_flow(s.oauth_flow),
        prov_sel: s.prov_sel,
        prov_delete_armed: s.prov_delete_armed,
        prov_msg: s.prov_msg,
        prov_modal: s.prov_modal.map(|m| ProviderModal {
            name: m.name,
            endpoint: m.endpoint,
            api_type: shadow_api_type(&m.api_type),
            api_key: m.api_key,
            field: m.field,
        }),
        models: s
            .models
            .into_iter()
            .map(|m| ModelDraft {
                uuid: m.uuid,
                name: m.name,
                model_id: m.model_id,
                provider_idx: m.provider_idx,
                roles: m.roles.iter().map(|r| shadow_role(r)).collect(),
                route: m.route,
                session_only: m.session_only,
                // Display-only client shadow: the snapshot carries no source_uuid
                // (the authoritative daemon SettingsState owns it) and this shadow is
                // never persisted, so None here can't clobber the real value.
                source_uuid: None,
            })
            .collect(),
        model_sel: s.model_sel,
        model_delete_armed: s.model_delete_armed,
        model_modal: s.model_modal.map(shadow_model_modal),
        model_filter: match s.model_filter.as_str() {
            "local"  => crate::app::mode::settings::ModelFilterMode::Local,
            "global" => crate::app::mode::settings::ModelFilterMode::Global,
            _        => crate::app::mode::settings::ModelFilterMode::All,
        },
        palette_sel: s.palette_sel,
    }
}

/// Rebuild the add/edit-model modal ([`ModelModal`]) from its projection. The
/// endpoints are reconstructed from the serde mirror back into [`ModelEndpoint`]
/// (a `Default`-padded copy carrying just the rendered fields).
fn shadow_model_modal(m: ModelModalSnapshot) -> ModelModal {
    ModelModal {
        editing_idx: m.editing_idx,
        uuid: m.uuid,
        name: m.name,
        provider_idx: m.provider_idx,
        model_id: m.model_id,
        field: m.field,
        roles: m.roles.iter().map(|r| shadow_role(r)).collect(),
        role_picker: m.role_picker.map(|rp| RolePickerState {
            checked: rp.checked,
            cursor: rp.cursor,
        }),
        query: m.query,
        result_sel: m.result_sel,
        route: m.route,
        route_sel: m.route_sel,
        endpoints: m.endpoints.map(|eps| {
            eps.into_iter()
                .map(|ep| ModelEndpoint {
                    name: ep.name,
                    provider_name: ep.provider_name,
                    pricing: Some(ModelPricing {
                        prompt: ep.price_prompt,
                        completion: ep.price_completion,
                    }),
                    context_length: None,
                    quantization: None,
                    max_completion_tokens: None,
                    uptime_last_30m: ep.uptime_last_30m,
                    status: None,
                })
                .collect()
        }),
        endpoints_loading: m.endpoints_loading,
        endpoints_for: m.endpoints_for,
        session_only: m.session_only,
    }
}

/// Rebuild the FS directory picker overlay ([`PathPicker`]) from its projection.
///
/// The matches are the daemon's already-computed `read_dir` results, used VERBATIM
/// (the client never walks its own filesystem — its cwd is unrelated to the
/// daemon's session). Constructed as a struct literal rather than via
/// `PathPicker::new`, which would re-run `list_dirs` against the local FS.
pub(crate) fn shadow_path_picker(p: PathPickerSnapshot) -> PathPicker {
    PathPicker {
        query: p.query,
        matches: p.matches,
        sel: p.sel,
        mode: match p.replace_idx {
            None => PickerMode::Add,
            Some(i) => PickerMode::Replace(i),
        },
    }
}

/// Map a theme wire token back to a [`ThemeMode`] (unknown → Dark).
pub(crate) fn shadow_theme(t: &str) -> ThemeMode {
    match t {
        "light" => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

/// Map an internet-mode wire token back to an [`InternetMode`] (unknown → Simple).
pub(crate) fn shadow_internet_mode(t: &str) -> InternetMode {
    match t {
        "full" => InternetMode::Full,
        _ => InternetMode::Simple,
    }
}

/// Map an api-type wire token back to an [`ApiType`] (unknown → OpenAiCompatible).
pub(crate) fn shadow_api_type(t: &str) -> ApiType {
    match t {
        "anthropic" => ApiType::AnthropicCompatible,
        "codex" => ApiType::Codex,
        "koma_free" => ApiType::KomaFree,
        _ => ApiType::OpenAiCompatible,
    }
}

/// Map an OAuth-provider wire token back to an [`OAuthProvider`] (unknown → Codex).
///
/// `pub(super)` — also called from `super::modes::shadow_onboard_provider`.
pub(super) fn shadow_oauth_provider(t: &str) -> crate::model::app_config::OAuthProvider {
    match t {
        "kilocode" => crate::model::app_config::OAuthProvider::Kilocode,
        "xai" => crate::model::app_config::OAuthProvider::Xai,
        "claudeai" => crate::model::app_config::OAuthProvider::ClaudeAI,
        "komarun" => crate::model::app_config::OAuthProvider::KomaRun,
        _ => crate::model::app_config::OAuthProvider::Codex,
    }
}

/// Rebuild one [`OAuthDraft`] from its wire projection.
fn shadow_oauth_draft(o: OAuthDraftSnapshot) -> OAuthDraft {
    OAuthDraft {
        uuid: o.uuid,
        label: o.label,
        provider: shadow_oauth_provider(&o.provider),
        key: o.key,
        status: o.status,
    }
}

/// Rebuild the OAuth submenu's connect-flow state from its flat wire projection
/// (see [`crate::ipc::proto::OAuthFlowSnapshot`] for the field-reuse convention).
/// An unrecognised `kind` (e.g. a version-skewed peer, or the all-default decode
/// of a missing field) falls back to `Idle` rather than panicking.
///
/// `pub(super)` — also called from `super::modes::shadow_onboard_provider`.
pub(super) fn shadow_oauth_flow(s: crate::ipc::proto::OAuthFlowSnapshot) -> OAuthFlowState {
    match s.kind.as_str() {
        "starting" => OAuthFlowState::Starting,
        "pick" => OAuthFlowState::Pick(s.cursor),
        "codex_wait" => OAuthFlowState::CodexWait { url: s.url, frame: s.frame, copied: s.copied },
        "codex_paste" => OAuthFlowState::CodexPaste { input: s.input },
        "kilo_wait" => OAuthFlowState::KiloWait {
            user_code: s.user_code,
            verification_url: s.url,
            frame: s.frame,
            copied: s.copied,
        },
        "failed" => OAuthFlowState::Failed(s.error),
        _ => OAuthFlowState::Idle,
    }
}

/// Map a role wire token back to a [`ModelRole`] (unknown → Main, never lost).
fn shadow_role(r: &str) -> ModelRole {
    match r {
        "awareness" => ModelRole::Awareness,
        "safeguard" => ModelRole::Safeguard,
        "compactor" => ModelRole::Compactor,
        "planner" => ModelRole::Planner,
        _ => ModelRole::Main,
    }
}

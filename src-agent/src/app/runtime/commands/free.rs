//! Free command: `/free` — toggle THIS session onto the keyless koma-free tier.
//!
//! No stored toggle flag: the ON/OFF state is DERIVED from
//! `settings.session_models` via [`koma_free_main_idx`] — a LOCAL Main-role
//! override whose provider is the koma-free connection. `/free` only ever
//! writes `settings.session_models` (plus, when provisioning the koma-free
//! connection for the first time, `config.providers` / `config.install_id`);
//! it NEVER touches `config.models`, so the global Main assignment is
//! untouched and resurfaces the instant the local override is removed.

use anyhow::Result;

use crate::app::state::AppState;
use crate::model::app_config::{new_uuid, ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn};
use crate::model::settings::Settings;
use crate::service::koma_free::{KOMA_FREE_ENDPOINT, KOMA_FREE_MODEL};

/// Position in `settings.session_models` of a LOCAL koma-free Main override,
/// if one exists: an entry whose [`ModelEntry::effective_roles`] contains
/// [`ModelRole::Main`] AND whose `provider_uuid` resolves (in
/// `config.providers`) to a connection with `api_type == ApiType::KomaFree`.
/// This IS the `/free` on/off state — there is no separate stored flag.
pub(super) fn koma_free_main_idx(config: &AppConfig, settings: &Settings) -> Option<usize> {
    settings.session_models.iter().position(|e| {
        let source = e
            .source_uuid
            .as_ref()
            .and_then(|u| config.models.iter().find(|m| &m.uuid == u))
            .unwrap_or(e);
        e.effective_roles().contains(&ModelRole::Main)
            && config
                .providers
                .iter()
                .any(|p| p.uuid == source.provider_uuid && p.api_type == ApiType::KomaFree)
    })
}

/// Handle the `/free` command: toggle THIS session's Main role onto/off the
/// keyless koma-free tier.
///
/// - A local koma-free Main override already exists (`Some(idx)`) → remove it;
///   the global (or otherwise configured) Main resurfaces for this session.
///   Toast "back to your main model".
/// - None exists → provision (find-or-create) the koma-free [`ProviderConn`]
///   (mirrors `handle_setup_koma_free`), drop any OTHER local Main override
///   (a local custom Main "swaps" to koma-free), and push a fresh koma-free
///   Main [`ModelEntry`] onto `settings.session_models`. Toast "koma free".
pub(super) fn handle_free(state: &mut AppState) -> Result<()> {
    let Some(sess) = state.rest.fg().session.as_ref() else {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    };
    let idx = koma_free_main_idx(&state.rest.config, &sess.settings);

    if idx.is_some() {
        // Toggle OFF: drop the local override; global/config Main resurfaces —
        // which may be a DIFFERENT model than koma-free, so snapshot before/
        // after to catch a BUG FIX effort reset (stale effort from koma-free/
        // the old model must not carry onto whatever resurfaces).
        let before_main = state.rest.main_identity_now();
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            let previous = sess.settings.session_models.clone();
            crate::model::app_config::assign_role(
                &mut sess.settings.session_models,
                ModelRole::Main,
                None,
            );
            if let Err(error) = sess.save() {
                sess.settings.session_models = previous;
                return Err(error);
            }
        }
        state.rest.reset_effort_if_main_changed(before_main);
        state
            .rest
            .fg_mut()
            .set_toast_info("back to your main model".to_string());
        return Ok(());
    }

    // Toggle ON: pin this session's Main onto the keyless koma-free tier.
    if let Err(e) = set_session_koma_free(state) {
        state.rest.fg_mut().status = format!("error: {e}");
        return Ok(());
    }
    state.rest.fg_mut().set_toast_info("koma free".to_string());
    Ok(())
}

/// Pin the FOREGROUND session's Main role onto the keyless koma-free tier
/// (`ApiType::KomaFree` / [`KOMA_FREE_MODEL`]) — the reusable core of `/free`'s
/// toggle-ON path, shared with the GUI model quick-picker's synthetic "advertised
/// free" row (`SetSessionMain { model_uuid: Some(KOMA_FREE_SENTINEL) }`).
///
/// Idempotent: if the session is ALREADY on a koma-free Main override this is a no-op.
/// Otherwise it (find-or-)creates the koma-free [`ProviderConn`] (persisting
/// `config` only when newly provisioned), drops any OTHER local Main override, and
/// pushes a fresh koma-free Main [`ModelEntry`] — writing ONLY `settings.session_models`
/// (never `config.models`), so the global Main assignment is untouched. A no-op when
/// there is no foreground session to hold the override.
pub(crate) fn set_session_koma_free(state: &mut AppState) -> Result<()> {
    // No session → nowhere to pin a local override.
    let Some(sess) = state.rest.fg().session.as_ref() else {
        return Ok(());
    };
    // Already on koma-free Main → idempotent no-op.
    if koma_free_main_idx(&state.rest.config, &sess.settings).is_some() {
        return Ok(());
    }
    let previous_config = state.rest.config.clone();
    let missing_install_id = state.rest.config.install_id.is_empty();
    // BUG FIX: snapshot the resolved Main route before the swap below so the
    // caller-agnostic effort reset fires whether this was reached via the TUI
    // `/free` toggle-ON or the GUI model quick-picker's synthetic "advertised
    // free" row (`SetSessionMain { model_uuid: Some(KOMA_FREE_SENTINEL) }`).
    let before_main = state.rest.main_identity_now();

    // The koma-free `X-Koma` header must never be empty. `install_id` is
    // serde-default + Default-minted, but mint one defensively if it somehow
    // got cleared, then persist it below.
    if state.rest.config.install_id.is_empty() {
        state.rest.config.install_id = new_uuid();
    }

    // Find-or-create the koma-free provider connection (mirrors
    // `handle_setup_koma_free` in `runtime/actions/onboard.rs`). Resolve the
    // uuid into an owned String first so the immutable `find` borrow ends
    // before the `push` mutable borrow.
    let existing_provider = state
        .rest
        .config
        .providers
        .iter()
        .find(|p| p.api_type == ApiType::KomaFree)
        .map(|p| p.uuid.clone());
    let (provider_uuid, provisioned) = match existing_provider {
        Some(uuid) => (uuid, false),
        None => {
            let uuid = new_uuid();
            state.rest.config.providers.push(ProviderConn {
                uuid: uuid.clone(),
                name: "koma free".to_string(),
                api_type: ApiType::KomaFree,
                endpoint: KOMA_FREE_ENDPOINT.to_string(),
                // Keyless: auth rides the X-Koma / X-Session headers.
                api_key: String::new(),
                // Native provider (the keyless free tier), not extension-managed.
                ext_id: None,
            });
            (uuid, true)
        }
    };
    if provisioned || missing_install_id {
        if let Err(error) =
            crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config)
        {
            state.rest.config = previous_config;
            return Err(error);
        }
    }

    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        let previous = sess.settings.session_models.clone();
        crate::model::app_config::assign_role(
            &mut sess.settings.session_models,
            ModelRole::Main,
            None,
        );
        sess.settings.session_models.push(ModelEntry {
            uuid: new_uuid(),
            name: "koma free".to_string(),
            model_id: KOMA_FREE_MODEL.to_string(),
            provider_uuid,
            route: None,
            roles: vec![ModelRole::Main],
            role: None,
            source_uuid: None,
        });
        if let Err(error) = sess.save() {
            sess.settings.session_models = previous;
            return Err(error);
        }
    }
    state.rest.reset_effort_if_main_changed(before_main);
    Ok(())
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn free_toggle_preserves_secondary_roles_including_legacy_apple_ownership() {
        let path = std::env::temp_dir().join(format!("koma-free-test-{}", uuid::Uuid::new_v4()));
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.config.providers.push(ProviderConn {
            uuid: "free".into(),
            api_type: ApiType::KomaFree,
            endpoint: KOMA_FREE_ENDPOINT.into(),
            ..Default::default()
        });
        state.rest.config.install_id = "test-install".into();
        let settings = Settings {
            session_models: vec![ModelEntry {
                uuid: "legacy-free".into(),
                provider_uuid: "free".into(),
                model_id: KOMA_FREE_MODEL.into(),
                roles: vec![ModelRole::Main, ModelRole::Safeguard, ModelRole::Compactor],
                ..Default::default()
            }],
            ..Default::default()
        };
        state.rest.fg_mut().session = Some(crate::model::session::Session::new(
            "free-test".into(),
            path.clone(),
            "pwd".into(),
            settings,
            crate::model::conversation::Conversation::from_messages(vec![]),
        ));
        handle_free(&mut state).unwrap();
        assert_eq!(
            state
                .rest
                .fg()
                .session
                .as_ref()
                .unwrap()
                .settings
                .session_models[0]
                .roles,
            vec![ModelRole::Safeguard, ModelRole::Compactor]
        );
        set_session_koma_free(&mut state).unwrap();
        let entries = &state
            .rest
            .fg()
            .session
            .as_ref()
            .unwrap()
            .settings
            .session_models;
        assert_eq!(
            entries[0].roles,
            vec![ModelRole::Safeguard, ModelRole::Compactor]
        );
        assert_eq!(entries.last().unwrap().roles, vec![ModelRole::Main]);
        std::fs::remove_dir_all(path).unwrap();
    }
}

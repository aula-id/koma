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
        e.effective_roles().contains(&ModelRole::Main)
            && config
                .providers
                .iter()
                .any(|p| p.uuid == e.provider_uuid && p.api_type == ApiType::KomaFree)
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

    if let Some(idx) = idx {
        // Toggle OFF: drop the local override; global/config Main resurfaces —
        // which may be a DIFFERENT model than koma-free, so snapshot before/
        // after to catch a BUG FIX effort reset (stale effort from koma-free/
        // the old model must not carry onto whatever resurfaces).
        let before_main = state.rest.main_identity_now();
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            sess.settings.session_models.remove(idx);
            if let Err(e) = sess.save() {
                state.rest.fg_mut().status = format!("error: {e}");
                return Ok(());
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
            });
            (uuid, true)
        }
    };
    if provisioned {
        state.rest.config.save()?;
    }

    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        // Swap: drop any OTHER local Main override first so koma-free is the
        // only local Main override afterward.
        sess.settings
            .session_models
            .retain(|e| !e.effective_roles().contains(&ModelRole::Main));
        sess.settings.session_models.push(ModelEntry {
            uuid: new_uuid(),
            name: "koma free".to_string(),
            model_id: KOMA_FREE_MODEL.to_string(),
            provider_uuid,
            route: None,
            // Permissive posture (owner override): koma-free powers EVERY runtime
            // role, not just Main — Awareness/Safeguard/Compactor/Planner all
            // resolve to it too via the session-first scan in `resolve_role`, so a
            // keyless `/free` user gets the safety classifier and every other
            // secondary role instead of silently going unconfigured for them.
            roles: vec![
                ModelRole::Main,
                ModelRole::Awareness,
                ModelRole::Safeguard,
                ModelRole::Compactor,
                ModelRole::Planner,
            ],
            role: None,
            source_uuid: None,
        });
        sess.save()?;
    }
    state.rest.reset_effort_if_main_changed(before_main);
    Ok(())
}

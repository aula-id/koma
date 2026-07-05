//! First-run connection CHOOSER actions (`Mode::Onboard`): the three setup paths a
//! brand-new install picks between before any credentials are asked for.
//!
//! - [`handle_setup_koma_free`] — keyless free tier: mint/reuse a koma-free
//!   provider + Main model in the global config, create a session, drop into Chat.
//! - [`handle_onboard_provider`] — open `/settings` on the OAuth category to sign
//!   in to a provider account.
//! - [`handle_onboard_custom`] — open the existing [`KeyInputForm`] wizard for an
//!   own-endpoint + API-key setup.

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{KeyInputForm, Mode, SettingsState, SETTING_CATEGORIES};
use crate::app::runtime::build_client;
use crate::app::state::AppState;
use crate::model::app_config::{new_uuid, ApiType, ModelEntry, ModelRole, ProviderConn};
use crate::model::store;
use crate::service::koma_free::{KOMA_FREE_ENDPOINT, KOMA_FREE_MODEL};
use crate::service::openrouter::OpenRouterClient;

/// Handle [`Action::SetupKomaFree`](crate::controller::input::Action::SetupKomaFree):
/// configure the keyless koma-free tier and drop straight into Chat.
///
/// Idempotent-ish: reuses an existing koma-free provider (and its Main model) when
/// one is already configured rather than duplicating entries. The post-config
/// session + Chat + warm tail mirrors
/// [`handle_save_creds`](super::settings_creds::handle_save_creds), but KEYLESS —
/// the client-build gate uses [`Resolved::is_usable`](crate::app::resolve::Resolved::is_usable)
/// so an empty-api-key koma-free route still builds a (keyless) client instead of
/// leaving `client = None`.
pub(super) fn handle_setup_koma_free(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // The koma-free `X-Koma` header must never be empty. `install_id` is serde-default
    // + Default-minted, but mint one defensively if it somehow got cleared, then the
    // `save()` below persists it.
    if state.rest.config.install_id.is_empty() {
        state.rest.config.install_id = new_uuid();
    }

    // Reuse an existing koma-free provider if one is already configured; otherwise
    // mint it. Resolve the uuid into an owned String FIRST so the immutable `find`
    // borrow ends before the `push` mutable borrow.
    let existing_provider = state
        .rest
        .config
        .providers
        .iter()
        .find(|p| p.api_type == ApiType::KomaFree)
        .map(|p| p.uuid.clone());
    let provider_uuid = match existing_provider {
        Some(uuid) => uuid,
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
            uuid
        }
    };

    // Ensure a Main-role koma-free model entry exists (reuse if this provider already
    // has one). The forced endpoint/model live in `resolve::from_entry`, so the entry's
    // `model_id` is only a label — pin it to the canonical id for clarity.
    let has_main_model = state.rest.config.models.iter().any(|m| {
        m.provider_uuid == provider_uuid && m.effective_roles().contains(&ModelRole::Main)
    });
    if !has_main_model {
        state.rest.config.models.push(ModelEntry {
            uuid: new_uuid(),
            name: "koma free".to_string(),
            model_id: KOMA_FREE_MODEL.to_string(),
            provider_uuid: provider_uuid.clone(),
            route: None,
            roles: vec![ModelRole::Main],
            role: None,
        });
    }

    if let Err(e) = state.rest.config.save() {
        state.rest.fg_mut().status = format!("config save failed: {e}");
    }

    // Lazy session creation: the first-run chooser has no session yet.
    if state.rest.fg().session.is_none() {
        match store::create_session() {
            Ok(s) => state.rest.fg_mut().session = Some(s),
            Err(e) => {
                state.rest.fg_mut().status = format!("error: {e}");
                return Ok(());
            }
        }
    }

    // --- post-config tail (mirrors handle_save_creds, KEYLESS) ---
    // Seed last-used creds for future sessions: keyless, so an empty key + the
    // koma-free model id (config drives real routing, not these legacy fields).
    state.rest.remember_creds("", KOMA_FREE_MODEL, "");
    // KEYLESS client → no creds baked in; (re)build one, gated on the Main route being
    // usable. `is_usable()` returns true for a keyless koma-free route, so the client
    // is actually built (a plain `!api_key.is_empty()` gate would leave it `None`).
    *client = state.rest.fg().session.as_ref().and_then(|sess| {
        crate::app::resolve::resolve_role(
            &state.rest.config,
            &sess.settings,
            ModelRole::Main,
        )
        .filter(|r| r.is_usable())
        .map(|_| build_client())
    });
    // Seed THIS foreground session's own counters from its (new/empty) ledger.
    if let Some(p) = state.rest.fg().session.as_ref().map(|s| s.path.clone()) {
        let fg = state.rest.foreground;
        state.rest.load_token_totals(fg, &p);
    }
    state.rest.prev_session = None; // committed; discard fallback
    state.rest.spawn_pending = false; // a /new-spawned session is now committed
    state.rest.reset_scroll();
    // Land in Chat first, THEN warm (warm_session is non-blocking and may upgrade the
    // mode to Loading, so it must run LAST).
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    super::super::warm_session(state, client, handle);
    Ok(())
}

/// Handle [`Action::OnboardProvider`](crate::controller::input::Action::OnboardProvider):
/// open `/settings` focused on the OAuth category so the user can sign in.
///
/// A session is required to seed the settings drafts; the chooser may have none yet
/// (the `--local` first-run path is lazy), so create one first — mirroring the lazy
/// creation the other setup paths do.
pub(super) fn handle_onboard_provider(state: &mut AppState) -> Result<()> {
    if state.rest.fg().session.is_none() {
        match store::create_session() {
            Ok(s) => state.rest.fg_mut().session = Some(s),
            Err(e) => {
                state.rest.fg_mut().status = format!("error: {e}");
                return Ok(());
            }
        }
    }
    let Some(session) = state.rest.fg().session.as_ref() else {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    };
    let mut st = SettingsState::from(session, &state.rest.config);
    // Land the sidebar on the OAuth category (fall back to the default first category
    // if it is ever renamed/removed).
    if let Some(idx) = SETTING_CATEGORIES.iter().position(|c| c.name == "OAuth") {
        st.cat = idx;
    }
    *state.mode_mut() = Mode::Settings(Box::new(st));
    Ok(())
}

/// Handle [`Action::OnboardCustom`](crate::controller::input::Action::OnboardCustom):
/// open the existing first-run credentials wizard for an own-endpoint + key setup.
pub(super) fn handle_onboard_custom(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::KeyInput(KeyInputForm::new());
    Ok(())
}

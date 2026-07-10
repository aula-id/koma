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

use crate::app::mode::{KeyInputForm, Mode, OnboardProviderState, OnboardState};
use crate::app::runtime::build_client;
use crate::app::state::AppState;
use crate::model::app_config::{new_uuid, ModelEntry, ModelRole};
use crate::model::store;
use crate::service::koma_free::KOMA_FREE_MODEL;
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
    // Mint/reuse the koma-free provider + Main-role model in the global config. Shared
    // with the GUI `SetupKomaFree` path (see `crate::service::koma_free`) so both mint
    // byte-identical entries; idempotent + ensures a non-empty `install_id`.
    crate::service::koma_free::ensure_koma_free_config(&mut state.rest.config);

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
    // Land in Chat and STAY there: warm in the BACKGROUND (no Loading splash) so the
    // composer + double-Esc composer-clear / rewind are live IMMEDIATELY on a freshly-
    // onboarded session. A splash on the cold keyless koma-free route would otherwise
    // hang (routing Esc to skip-loading, not the double-Esc path). The awareness
    // summary + workspace index still fold in via the WarmEvent / dir_cache drains.
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    super::super::warm_session_background(state, client, handle);
    Ok(())
}

/// Handle [`Action::OnboardProvider`](crate::controller::input::Action::OnboardProvider):
/// open the guided provider onboarding wizard ([`Mode::OnboardProvider`]).
///
/// A session is required for the save-and-warm tail; the chooser may have none yet
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
    *state.mode_mut() = Mode::OnboardProvider(Box::new(OnboardProviderState::new()));
    Ok(())
}

/// Handle [`Action::OnboardProviderBack`](crate::controller::input::Action::OnboardProviderBack):
/// return from the wizard's Login picker to the first-run connection chooser with the
/// "provider" row (index 1) highlighted.
pub(super) fn handle_onboard_provider_back(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Onboard(Box::new(OnboardState { cursor: 1 }));
    Ok(())
}

/// Handle [`Action::OnboardProviderSaveModel`](crate::controller::input::Action::OnboardProviderSaveModel):
/// bind the chosen model as the GLOBAL Main model on the just-signed-in OAuth
/// connection, then create/warm a session and drop into Chat.
///
/// The save-and-warm tail mirrors [`handle_setup_koma_free`] and, like it, gates the
/// client (re)build on [`Resolved::is_usable`](crate::app::resolve::Resolved::is_usable)
/// rather than a non-empty api key — an OAuth Main route carries NO api key (auth rides
/// the connection's token lifecycle), so a plain `!api_key.is_empty()` gate would leave
/// `client = None`.
pub(super) fn handle_onboard_provider_save_model(
    model_id: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Read the wizard's just-created connection uuid + provider label out of the mode
    // (borrow released before mutating config/mode below).
    let (conn_uuid, provider_label) = match state.mode() {
        Mode::OnboardProvider(op) => (
            op.new_conn_uuid.clone(),
            op.provider.map(|p| p.label().to_string()).unwrap_or_default(),
        ),
        _ => (String::new(), String::new()),
    };

    // Name the entry after the chosen model id (fall back to the provider label, then
    // a generic "Main"); the id itself is the routing key.
    let name = if !model_id.trim().is_empty() {
        model_id.clone()
    } else if !provider_label.is_empty() {
        provider_label
    } else {
        "Main".to_string()
    };

    // Bind a Main-role model entry to the new OAuth connection (GLOBAL scope, all
    // roles route here). `route` is None — OAuth providers don't take an OpenRouter
    // upstream pin.
    state.rest.config.models.push(ModelEntry {
        uuid: new_uuid(),
        name,
        model_id: model_id.clone(),
        provider_uuid: conn_uuid,
        route: None,
        roles: vec![ModelRole::Main],
        role: None,
        source_uuid: None,
    });
    if let Err(e) = state.rest.config.save() {
        state.rest.fg_mut().status = format!("config save failed: {e}");
    }

    // Lazy session creation (defensive; wizard entry already created one).
    if state.rest.fg().session.is_none() {
        match store::create_session() {
            Ok(s) => state.rest.fg_mut().session = Some(s),
            Err(e) => {
                state.rest.fg_mut().status = format!("error: {e}");
                return Ok(());
            }
        }
    }

    // --- post-config tail (mirrors handle_setup_koma_free) ---
    // Seed last-used creds: keyless (config drives real routing), the chosen model id
    // for continuity.
    state.rest.remember_creds("", &model_id, "");
    // (Re)build the client gated on the Main route being USABLE (keyless-OAuth safe).
    *client = state.rest.fg().session.as_ref().and_then(|sess| {
        crate::app::resolve::resolve_role(&state.rest.config, &sess.settings, ModelRole::Main)
            .filter(|r| r.is_usable())
            .map(|_| build_client())
    });
    // Seed THIS foreground session's counters from its (new/empty) ledger.
    if let Some(p) = state.rest.fg().session.as_ref().map(|s| s.path.clone()) {
        let fg = state.rest.foreground;
        state.rest.load_token_totals(fg, &p);
    }
    state.rest.prev_session = None; // committed; discard fallback
    state.rest.spawn_pending = false; // a /new-spawned session is now committed
    state.rest.reset_scroll();
    // Land in Chat and STAY there: warm in the BACKGROUND (no Loading splash) so the
    // composer + double-Esc composer-clear / rewind are live IMMEDIATELY on a freshly-
    // onboarded session (mirrors handle_setup_koma_free). The awareness summary +
    // workspace index still fold in via the WarmEvent / dir_cache drains.
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    super::super::warm_session_background(state, client, handle);
    Ok(())
}

/// Handle [`Action::OnboardCustom`](crate::controller::input::Action::OnboardCustom):
/// open the existing first-run credentials wizard for an own-endpoint + key setup.
pub(super) fn handle_onboard_custom(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::KeyInput(KeyInputForm::new());
    Ok(())
}

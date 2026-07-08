//! koma-free keyless transport constants.
//!
//! koma-free is an OpenAI-compatible chat-completions gateway served at
//! [`KOMA_FREE_ENDPOINT`]; the client appends `/chat/completions` to it exactly
//! like any other OpenAI-compatible base URL, yielding
//! `https://koma.run/api/v1/koma-free/chat/completions`. Auth is two custom
//! headers (`X-Koma` install id + `X-Session`) with NO `Authorization` bearer —
//! see `service::openrouter::helpers::auth_headers_with_account`. Every request
//! pins [`KOMA_FREE_MODEL`].

use crate::model::app_config::{
    new_uuid, ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn,
};

/// Base URL for the koma-free gateway. NO trailing slash: the request path is
/// built as `{KOMA_FREE_ENDPOINT}/chat/completions`.
pub const KOMA_FREE_ENDPOINT: &str = "https://koma.run/api/v1/koma-free";

/// The only model id koma-free serves. Forced onto the resolved route so a
/// `/settings` model-id edit can never 404 the request.
pub const KOMA_FREE_MODEL: &str = "koma/apple";

/// Stable, opaque sentinel id for the SYNTHETIC "advertised free" row the GUI host
/// projects at the top of the model quick-picker (wave-3+4 free-pin). It is NOT a real
/// [`crate::model::app_config::ModelEntry`] uuid — `/free` never writes `config.models`
/// (see `runtime::commands::free`) — so this dedicated id can never collide with a
/// user-added global model (even one manually pinned to [`KOMA_FREE_MODEL`]). When it
/// round-trips back as a `SetSessionMain { model_uuid: Some(KOMA_FREE_SENTINEL) }`, the
/// handler routes through the `/free` find-or-create flow instead of a global clone.
pub const KOMA_FREE_SENTINEL: &str = "koma-free";

/// Mint (or reuse) the keyless koma-free provider + a Main-role koma-free model in `cfg`.
///
/// The CONFIG-mutation core shared by the TUI first-run chooser
/// ([`crate::app::runtime::actions::onboard`]'s `handle_setup_koma_free`) and the GUI
/// `SetupKomaFree` request, so both mint byte-identical entries. Idempotent: reuses an
/// existing [`ApiType::KomaFree`] provider (and its Main model) instead of duplicating,
/// and ensures a non-empty `install_id` (the `X-Koma` header must never be blank). Does
/// NOT persist — the caller saves `cfg` (and, on the GUI path, re-pushes its `Config`).
pub fn ensure_koma_free_config(cfg: &mut AppConfig) {
    // The koma-free `X-Koma` header must never be empty; mint an install id if missing.
    if cfg.install_id.is_empty() {
        cfg.install_id = new_uuid();
    }

    // Reuse an existing koma-free provider if one is configured; otherwise mint it.
    // Resolve the uuid into an owned String FIRST so the immutable `find` borrow ends
    // before the `push` mutable borrow.
    let provider_uuid = match cfg
        .providers
        .iter()
        .find(|p| p.api_type == ApiType::KomaFree)
        .map(|p| p.uuid.clone())
    {
        Some(uuid) => uuid,
        None => {
            let uuid = new_uuid();
            cfg.providers.push(ProviderConn {
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

    // Ensure a Main-role koma-free model entry exists (reuse if this provider already has
    // one). Pin the canonical koma-free model id.
    let has_main_model = cfg.models.iter().any(|m| {
        m.provider_uuid == provider_uuid && m.effective_roles().contains(&ModelRole::Main)
    });
    if !has_main_model {
        cfg.models.push(ModelEntry {
            uuid: new_uuid(),
            name: "koma free".to_string(),
            model_id: KOMA_FREE_MODEL.to_string(),
            provider_uuid,
            route: None,
            roles: vec![ModelRole::Main],
            role: None,
        });
    }
}

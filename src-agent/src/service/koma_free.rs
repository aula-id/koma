//! koma-free keyless transport constants.
//!
//! koma-free is an OpenAI-compatible chat-completions gateway served at
//! [`KOMA_FREE_ENDPOINT`]; the client appends `/chat/completions` to it exactly
//! like any other OpenAI-compatible base URL, yielding
//! `https://koma.run/api/v1/koma-free/chat/completions`. Auth is two custom
//! headers (`X-Koma` install id + `X-Session`) with NO `Authorization` bearer —
//! see `service::openrouter::helpers::auth_headers_with_account`. Every request
//! pins [`KOMA_FREE_MODEL`].

use crate::model::app_config::{new_uuid, ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn};

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

/// Provision Apple for unassigned roles only. Existing ownership survives repeated setup.
/// Does not persist; callers report save errors.
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
                // Native provider (the keyless free tier), not extension-managed.
                ext_id: None,
            });
            uuid
        }
    };

    let all_roles = [
        ModelRole::Main,
        ModelRole::Awareness,
        ModelRole::Safeguard,
        ModelRole::Compactor,
        ModelRole::Planner,
    ];
    let empty_roles: Vec<_> = all_roles
        .into_iter()
        .filter(|role| {
            !cfg.models
                .iter()
                .any(|m| m.effective_roles().contains(role))
        })
        .collect();
    if let Some(existing) = cfg
        .models
        .iter_mut()
        .find(|m| m.provider_uuid == provider_uuid)
    {
        existing.roles = existing.effective_roles();
        existing.role = None;
        existing.roles.extend(empty_roles);
    } else {
        cfg.models.push(ModelEntry {
            uuid: new_uuid(),
            name: "koma free".to_string(),
            model_id: KOMA_FREE_MODEL.to_string(),
            provider_uuid,
            route: None,
            roles: empty_roles,
            role: None,
            source_uuid: None,
        });
    }
}

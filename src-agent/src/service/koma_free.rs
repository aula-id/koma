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
    new_uuid, strip_role, ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn,
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

/// Mint (or reuse) the keyless koma-free provider + an ALL-roles koma-free model
/// (Main, Awareness, Safeguard, Compactor, Planner — permissive posture, owner
/// override: koma-free powers every runtime role) in `cfg`.
///
/// The CONFIG-mutation core shared by the TUI first-run chooser
/// ([`crate::app::runtime::actions::onboard`]'s `handle_setup_koma_free`) and the GUI
/// `SetupKomaFree` request, so both mint byte-identical entries. Idempotent AND
/// self-healing: reuses an existing [`ApiType::KomaFree`] provider (and its model
/// entry) instead of duplicating, UPGRADING the model entry's role set to the full
/// union if it was previously minted with a narrower one (e.g. an older build's
/// Main-only entry), and ensures a non-empty `install_id` (the `X-Koma` header must
/// never be blank). Does NOT persist — the caller saves `cfg` (and, on the GUI
/// path, re-pushes its `Config`).
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

    // Ensure a koma-free model entry exists holding EVERY runtime role (Main,
    // Awareness, Safeguard, Compactor, Planner) — permissive posture (owner
    // override): koma-free powers every role, not just Main, so a keyless install
    // gets the safety classifier and every other secondary role instead of
    // silently going unconfigured for them. Pin the canonical koma-free model id.
    //
    // Idempotent AND self-healing: reuse an existing koma-free model entry for
    // this provider if one exists, UPGRADING its role set to the full union
    // rather than only minting fresh with the full set. Older builds left a
    // koma-free entry with `roles: [Main]` only — without this upgrade, a
    // pre-existing install would keep a stale Main-only entry forever (this
    // function only mints when NO entry exists at all). `effective_roles()`
    // folds in the legacy single-role field too, so a pre-multi-role entry keeps
    // whatever it already held.
    let all_roles = [
        ModelRole::Main,
        ModelRole::Awareness,
        ModelRole::Safeguard,
        ModelRole::Compactor,
        ModelRole::Planner,
    ];
    let koma_free_uuid = if let Some(existing) = cfg
        .models
        .iter_mut()
        .find(|m| m.provider_uuid == provider_uuid)
    {
        let mut roles = existing.effective_roles();
        for role in all_roles {
            if !roles.contains(&role) {
                roles.push(role);
            }
        }
        existing.roles = roles;
        // The union now lives in `roles`; clear the stale legacy single-role field
        // so a later demotion of THIS entry (some other model stealing a role) can
        // never resurface a role from the legacy slot.
        existing.role = None;
        existing.uuid.clone()
    } else {
        let uuid = new_uuid();
        cfg.models.push(ModelEntry {
            uuid: uuid.clone(),
            name: "koma free".to_string(),
            model_id: KOMA_FREE_MODEL.to_string(),
            provider_uuid,
            route: None,
            roles: all_roles.to_vec(),
            role: None,
            source_uuid: None,
        });
        uuid
    };

    // Single-holder invariant (the same one `upsert_model_entry` keeps for the GUI
    // ModelForm save): koma-free now claims EVERY runtime role, so steal each of
    // those roles from every OTHER global model — from BOTH the roles vec and the
    // legacy role field (via `strip_role`). Without this, a model the user had
    // pinned to Main (or any role) stays a second effective holder and either
    // shadows koma-free or is shadowed BY it through `resolve_role`'s first-wins
    // scan (the exact "koma-free + grpk both hold Main" duplicate this closes).
    for other in cfg.models.iter_mut() {
        if other.uuid != koma_free_uuid {
            for role in all_roles {
                strip_role(other, role);
            }
        }
    }
}

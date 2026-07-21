//! Async kick-off functions for the `/store` marketplace browser (`Mode::ExtStore`):
//! browse the catalogue, fetch one extension's detail, and download an install artifact.
//!
//! Mirrors [`super::screen`]'s shape: each `kick_off_store_*` opens a FRESH channel,
//! stashes the receiver on `AppStateRest::store_rx` (replacing any prior one — only one
//! `/store` fetch is ever in flight at a time, since Browse -> Detail -> InstallConfirm
//! is a strictly sequential flow), and spawns the network work on the runtime `Handle`.
//! The result lands as a [`StoreEvent`], drained per-tick by
//! `event_loop::global::drains::drain_store` (the exact `ext_screen_rx`/`sec_health_rx`
//! shape) — the event loop is NEVER blocked on a store network call.
//!
//! `kick_off_store_install` mirrors `DaemonHub::install_extension`'s SYNCHRONOUS half
//! (platform-detect + resolve the koma.run bearer connection) before spawning the
//! network download; the caller surfaces a synchronous `Err` inline WITHOUT spawning
//! (unsupported platform / no koma.run sign-in), exactly like the GUI store hub does.
//! The on-loop verify+install tail (shared with the hub) lives in
//! `app::runtime::actions::ext_install::install_extension_core`, invoked by the drain
//! once a [`StoreEvent::InstallArtifact`] lands.

use tokio::runtime::Handle;

use crate::app::ext::store_api::{detect_platform, fetch_catalogue, fetch_detail, fetch_install_artifact};
use crate::app::state::AppStateRest;
use crate::ipc::proto::{StoreDetailWire, StoreItemWire};
use crate::model::app_config::OAuthProvider;

/// The delivered outcome of one async `/store` fetch/install, shipped back on
/// `AppStateRest::store_rx`.
#[derive(Debug)]
pub(crate) enum StoreEvent {
    /// A Browse catalogue fetch resolved (possibly with a network/parse error).
    Catalogue(Result<Vec<StoreItemWire>, String>),
    /// A Detail fetch for one extension id resolved.
    Detail(Box<Result<StoreDetailWire, String>>),
    /// An install download resolved: the raw zip + its advertised integrity fields,
    /// ready for the on-loop verify+install tail. `id` is echoed for the drain's fold.
    InstallArtifact {
        id: String,
        zip: Vec<u8>,
        sha256: String,
        signature: Option<String>,
    },
    /// An install download failed (network error, bad HTTP status, or an expired
    /// koma.run session) — surfaced directly, no artifact to install.
    InstallFailed { id: String, error: String },
}

/// Kick off `GET /extensions[?q&category]` for the Browse list. Opens a fresh channel
/// (replacing any prior one) and spawns the fetch; the result lands as
/// [`StoreEvent::Catalogue`].
pub(crate) fn kick_off_store_browse(
    rest: &mut AppStateRest,
    handle: &Handle,
    query: Option<String>,
    category: Option<String>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    rest.store_rx = Some(rx);
    handle.spawn(async move {
        let result = fetch_catalogue(query, category).await;
        let _ = tx.send(StoreEvent::Catalogue(result));
    });
}

/// Kick off `GET /extensions/{id}` for the Detail pane. Opens a fresh channel (replacing
/// any prior one) and spawns the fetch; the result lands as [`StoreEvent::Detail`].
pub(crate) fn kick_off_store_detail(rest: &mut AppStateRest, handle: &Handle, id: String) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    rest.store_rx = Some(rx);
    handle.spawn(async move {
        let result = fetch_detail(&id).await;
        let _ = tx.send(StoreEvent::Detail(Box::new(result)));
    });
}

/// Kick off an install download for extension `id` (optionally pinning `version`).
///
/// Detects the platform + resolves the koma.run OAuth bearer connection SYNCHRONOUSLY
/// (both need `rest`, mirroring `DaemonHub::install_extension`): an unsupported
/// platform or a missing koma.run sign-in is a synchronous `Err` the caller surfaces
/// inline WITHOUT spawning. Otherwise opens a fresh channel (replacing any prior one)
/// and spawns the bearer-refresh + download; the result lands as
/// [`StoreEvent::InstallArtifact`] / [`StoreEvent::InstallFailed`].
pub(crate) fn kick_off_store_install(
    rest: &mut AppStateRest,
    handle: &Handle,
    id: String,
    version: Option<String>,
) -> Result<(), String> {
    let Some(platform) = detect_platform() else {
        return Err("extensions are not available for this platform".to_string());
    };
    let Some(conn) = rest
        .config
        .oauth_conns
        .iter()
        .find(|c| c.provider == OAuthProvider::KomaRun)
    else {
        return Err("sign in to koma.run to install".to_string());
    };
    let oauth_uuid = conn.uuid.clone();
    let platform = platform.to_string();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    rest.store_rx = Some(rx);
    handle.spawn(async move {
        // A fresh (possibly just-refreshed) koma.run access token. Empty ⇒ the
        // connection is gone / unrecoverable — treat as a sign-in failure.
        let (bearer, _account) = crate::service::oauth::manager::fresh_key(&oauth_uuid, "").await;
        if bearer.trim().is_empty() {
            let _ = tx.send(StoreEvent::InstallFailed {
                id,
                error: "koma.run session expired — sign in again".to_string(),
            });
            return;
        }
        let event = match fetch_install_artifact(&id, version.as_deref(), &platform, &bearer).await {
            Ok((zip, sha256, signature)) => StoreEvent::InstallArtifact { id, zip, sha256, signature },
            Err(e) => StoreEvent::InstallFailed { id, error: e },
        };
        let _ = tx.send(event);
    });
    Ok(())
}

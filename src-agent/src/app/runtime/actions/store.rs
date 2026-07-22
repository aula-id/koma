//! Action handlers for the `/store` marketplace browser: CloseStore, StoreRetryBrowse,
//! StoreOpenDetail, StoreInstallConfirm.
//!
//! The network fetches (browse/detail/install-download) run ASYNC via
//! `app::ext::ext_store::kick_off_store_*`, landing on `AppStateRest::store_rx`; the
//! per-tick `event_loop::global::drains::drain_store` folds the reply into the open
//! `Mode::ExtStore` (and, for a landed install artifact, runs the shared on-loop
//! `ext_install::install_extension_core`) — so no handler here blocks the event loop.

use anyhow::Result;

use crate::app::mode::{Mode, StoreSubMode};
use crate::app::state::AppState;
use crate::model::app_config::OAuthProvider;

/// Handle `Action::CloseStore`: leave the marketplace browser and return to Chat.
pub(super) fn handle_close_store(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}

/// Handle `Action::StoreRetryBrowse`: reset the Browse loading/error state and re-kick
/// the async catalogue fetch (the `r` retry key, only reachable while an error is shown).
pub(super) fn handle_store_retry_browse(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if let Mode::ExtStore(s) = state.mode_mut() {
        s.loading = true;
        s.error = None;
    }
    crate::app::ext::ext_store::kick_off_store_browse(&mut state.rest, handle, None, None);
    Ok(())
}

/// Handle `Action::StoreOpenDetail`: enter Detail for the row highlighted in Browse and
/// kick off the async detail fetch. A no-op when nothing is selected.
pub(super) fn handle_store_open_detail(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let Some(id) = (match state.mode() {
        Mode::ExtStore(s) => s.current().map(|r| r.id.clone()),
        _ => None,
    }) else {
        return Ok(());
    };
    if let Mode::ExtStore(s) = state.mode_mut() {
        s.sub_mode = StoreSubMode::Detail;
        s.detail = None;
        s.detail_loading = true;
        s.detail_error = None;
    }
    crate::app::ext::ext_store::kick_off_store_detail(&mut state.rest, handle, id);
    Ok(())
}

/// Handle `Action::StoreInstallConfirm` (`y` in InstallConfirm): re-verify the koma.run
/// bearer is still on file (defense in depth — the keypress that ARMED InstallConfirm
/// already gated on it), then kick off the async install download. A missing connection
/// surfaces inline without spawning, mirroring `DaemonHub::install_extension`'s
/// synchronous failure path.
pub(super) fn handle_store_install_confirm(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let Some(id) = (match state.mode() {
        Mode::ExtStore(s) => s.current().map(|r| r.id.clone()),
        _ => None,
    }) else {
        return Ok(());
    };

    if !state
        .rest
        .config
        .oauth_conns
        .iter()
        .any(|c| c.provider == OAuthProvider::KomaRun)
    {
        if let Mode::ExtStore(s) = state.mode_mut() {
            s.komarun_connected = false;
            s.install_error = Some("connect koma.run in /settings → OAuth first".to_string());
        }
        return Ok(());
    }

    if let Mode::ExtStore(s) = state.mode_mut() {
        s.installing = true;
        s.install_error = None;
    }
    if let Err(e) =
        crate::app::ext::ext_store::kick_off_store_install(&mut state.rest, handle, id, None)
    {
        if let Mode::ExtStore(s) = state.mode_mut() {
            s.installing = false;
            s.install_error = Some(e);
        }
    }
    Ok(())
}

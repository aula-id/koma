//! Action handlers for the `/settings` OAuth submenu: OAuthStart, OAuthCancel,
//! OAuthPaste, OAuthDelete.
//!
//! `OAuthStart` spawns the Codex browser flow or the Kilo Code device flow on a
//! background task (mirrors `settings::handle_fetch_model_endpoints`): a fresh
//! `oauth_rx` channel is opened, the previous flow (if any) is aborted, and the
//! spawned future sends `OAuthEvent`s the event-loop's `service_global` drains
//! into the open OAuth submenu. `OAuthPaste` and `OAuthDelete` are synchronous —
//! no background task, so they apply directly within this bracketed action.

use anyhow::Result;

use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::Mode;
use crate::app::state::{AppState, AppStateRest};
use crate::model::app_config::{OAuthConn, OAuthProvider};
use crate::service::oauth::OAuthEvent;

/// Handle `Action::OAuthStart`: supersede any in-flight flow, arm the
/// transitional `Starting` screen, and spawn the chosen provider's flow.
pub(super) fn handle_oauth_start(
    provider: OAuthProvider,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Supersede an older in-flight flow: abort its task (a dropped receiver
    // alone wouldn't stop a browser-wait or device-poll loop) and drop the
    // stale receiver.
    if let Some(h) = state.rest.oauth_task.take() {
        h.abort();
    }
    state.rest.oauth_rx = None;

    if let Mode::Settings(s) = state.mode_mut() {
        s.oauth_flow = OAuthFlowState::Starting;
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    state.rest.oauth_rx = Some(rx);

    let join = match provider {
        OAuthProvider::Codex => handle.spawn(run_codex_flow(tx)),
        OAuthProvider::Kilocode => handle.spawn(run_kilo_flow(tx)),
    };
    state.rest.oauth_task = Some(join.abort_handle());
    Ok(())
}

/// The Codex browser flow: build the PKCE authorization URL, open the system
/// browser, wait on the loopback redirect, then exchange the code for tokens.
/// Sends exactly one terminal event (`Success` or `Failed`) after an initial
/// `CodexUrl`; a dropped receiver (flow superseded/cancelled) makes every send
/// a silent no-op.
async fn run_codex_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let auth = crate::service::oauth::codex::build_auth_url();
    let _ = tx.send(OAuthEvent::CodexUrl { url: auth.url.clone() });
    crate::service::oauth::browser::open_in_browser(&auth.url);

    let cb = match crate::service::oauth::loopback::catch_callback(&auth.pkce.state, 300).await {
        Ok(cb) => cb,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };

    let http = reqwest::Client::new();
    match crate::service::oauth::codex::exchange_code(&http, &cb.code, &auth.pkce.verifier).await {
        Ok(tokens) => {
            let conn = crate::service::oauth::codex::to_conn(tokens);
            let _ = tx.send(OAuthEvent::Success { conn });
        }
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
        }
    }
}

/// The Kilo Code device flow: request a device code, open the system browser to
/// its verification URL, poll for approval, then fetch the profile (org id /
/// email) to label the connection. Sends exactly one terminal event after an
/// initial `KiloCode`.
async fn run_kilo_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let http = reqwest::Client::new();
    let dc = match crate::service::oauth::kilo::device_init(&http).await {
        Ok(dc) => dc,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };
    let _ = tx.send(OAuthEvent::KiloCode {
        user_code: dc.code.clone(),
        verification_url: dc.verification_url.clone(),
    });
    crate::service::oauth::browser::open_in_browser(&dc.verification_url);

    let token = match crate::service::oauth::kilo::poll(&http, &dc.code, dc.expires_in).await {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };
    let (org_id, email) = crate::service::oauth::kilo::profile(&http, &token).await;
    let conn = crate::service::oauth::kilo::to_conn(token, org_id, email);
    let _ = tx.send(OAuthEvent::Success { conn });
}

/// Handle `Action::OAuthCancel`: abort the in-flight task, drop the receiver,
/// and return the submenu to `Idle`. The daemon owns the flow state, so this
/// does not optimistically pre-set anything beyond what actually happened.
pub(super) fn handle_oauth_cancel(state: &mut AppState) -> Result<()> {
    if let Some(h) = state.rest.oauth_task.take() {
        h.abort();
    }
    state.rest.oauth_rx = None;
    if let Mode::Settings(s) = state.mode_mut() {
        s.oauth_flow = OAuthFlowState::Idle;
    }
    Ok(())
}

/// Handle `Action::OAuthPaste`: build a connection straight from a hand-pasted
/// raw access token (no refresh/id token, no known expiry) and persist it via
/// the same path a completed browser/device flow uses.
pub(super) fn handle_oauth_paste(
    input: String,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let tokens = crate::service::oauth::codex::TokenResponse {
        access_token: input.trim().to_string(),
        refresh_token: String::new(),
        id_token: String::new(),
        expires_in: None,
    };
    let conn = crate::service::oauth::codex::to_conn(tokens);
    apply_login_result(&mut state.rest, conn, handle, |s, outcome| match outcome {
        Ok(drafts) => {
            s.oauth_drafts = drafts;
            s.oauth_flow = OAuthFlowState::Idle;
        }
        Err(e) => {
            s.oauth_flow = OAuthFlowState::Failed(format!("login saved locally but config write failed: {e}"));
        }
    });
    Ok(())
}

/// Persist a completed OAuth login: seed the token-refresh cache (fire-and-forget
/// background task), append to `config.oauth_conns`, save `config.json`, rebuild
/// `oauth_drafts`, then hand the result to `apply` so the caller can fold it into
/// whichever `SettingsState`(s) it's responsible for — the BRACKETED current
/// session here (synchronous callers), or every session in `Mode::Settings` in the
/// `service_global` drain of an async flow's `Success` event (de-globalized, C3).
fn apply_login_result(
    rest: &mut AppStateRest,
    conn: OAuthConn,
    handle: &tokio::runtime::Handle,
    apply: impl FnOnce(&mut crate::app::mode::settings::SettingsState, Result<Vec<crate::app::mode::settings::OAuthDraft>, String>),
) {
    let seeded = conn.clone();
    handle.spawn(async move {
        crate::service::oauth::manager::seed(&seeded).await;
    });
    rest.config.oauth_conns.push(conn);
    let outcome = rest
        .config
        .save()
        .map_err(|e| e.to_string())
        .map(|_| crate::app::mode::settings::OAuthDraft::from_config(&rest.config));
    if let Mode::Settings(s) = &mut rest.fg_mut().mode {
        apply(s, outcome);
    }
}

/// Handle `Action::OAuthDelete`: remove the connection from `config.oauth_conns`,
/// persist, evict its token-refresh cache entry, and rebuild `oauth_drafts` in
/// the open submenu.
pub(super) fn handle_oauth_delete(
    uuid: String,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    state.rest.config.oauth_conns.retain(|c| c.uuid != uuid);
    if let Err(e) = state.rest.config.save() {
        state.rest.fg_mut().status = format!("config save failed: {e}");
    }
    let drafts = crate::app::mode::settings::OAuthDraft::from_config(&state.rest.config);
    if let Mode::Settings(s) = state.mode_mut() {
        s.oauth_drafts = drafts;
        s.oauth_sel = s.oauth_sel.min(s.oauth_drafts.len());
        s.oauth_armed = None;
    }
    handle.spawn(async move {
        crate::service::oauth::manager::evict(&uuid).await;
    });
    Ok(())
}

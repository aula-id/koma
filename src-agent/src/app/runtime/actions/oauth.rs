//! Action handlers for the `/settings` OAuth submenu and `Mode::OnboardProvider`
//! guided wizard: OAuthStart, OAuthCancel, OAuthPaste, OAuthDelete.
//!
//! `OAuthStart` spawns the Codex browser flow or the Kilo Code device flow on a
//! background task (mirrors `settings::handle_fetch_model_endpoints`): a fresh
//! `oauth_rx` channel is opened, the previous flow (if any) is aborted, and the
//! spawned future sends `OAuthEvent`s the event-loop's `service_global` drains
//! into the open OAuth submenu or wizard. `OAuthPaste` and `OAuthDelete` are
//! synchronous — no background task, so they apply directly within this bracketed
//! action.

use anyhow::Result;

use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::{Mode, OnboardProviderStep};
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
    // Disarm any GUI OAuth push side-channel the superseded flow armed. This runs on
    // EVERY start path (a GUI `StartOAuth` request AND a TUI keypress → `SendKey` →
    // here), so arm/disarm is path-agnostic: a TUI-originated start leaves it disarmed,
    // and the GUI request handler re-arms with its own client id AFTER this returns —
    // so a stale id can never push another client this flow's state (incl. the email /
    // plan / account_id PII on success).
    state.rest.oauth_gui_client = None;

    // Optimistic transitional paint on whichever oauth-flow-bearing mode is active.
    match state.mode_mut() {
        Mode::Settings(s) => s.oauth_flow = OAuthFlowState::Starting,
        Mode::OnboardProvider(op) => op.oauth_flow = OAuthFlowState::Starting,
        _ => {}
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    state.rest.oauth_rx = Some(rx);

    let join = match provider {
        OAuthProvider::Codex => handle.spawn(run_codex_flow(tx)),
        OAuthProvider::Kilocode => handle.spawn(run_kilo_flow(tx)),
        OAuthProvider::Xai => handle.spawn(run_xai_flow(tx)),
        OAuthProvider::ClaudeAI => handle.spawn(run_claude_flow(tx)),
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

    let cb = match crate::service::oauth::loopback::catch_callback(
        &auth.pkce.state,
        300,
        crate::service::oauth::registry::CODEX_PORT,
    )
    .await
    {
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

/// The Claude (Anthropic) browser flow: build the PKCE authorization URL, open the
/// system browser, wait on the loopback redirect (port 54545), then exchange the code
/// for tokens. Mirrors `run_codex_flow` exactly, against Anthropic's own endpoints.
async fn run_claude_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let auth = crate::service::oauth::claude::build_auth_url();
    let _ = tx.send(OAuthEvent::CodexUrl { url: auth.url.clone() });
    crate::service::oauth::browser::open_in_browser(&auth.url);

    let cb = match crate::service::oauth::loopback::catch_callback(
        &auth.pkce.state,
        300,
        crate::service::oauth::registry::CLAUDE_PORT,
    )
    .await
    {
        Ok(cb) => cb,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };

    let http = reqwest::Client::new();
    match crate::service::oauth::claude::exchange_code(
        &http,
        &cb.code,
        &auth.pkce.verifier,
        &auth.pkce.state,
    )
    .await
    {
        Ok(tokens) => {
            let conn = crate::service::oauth::claude::to_conn(tokens);
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

/// The xAI (Grok) device flow: request a device code, open the verification URL,
/// poll xAI's DISCOVERED token endpoint for approval, then build the connection
/// from the returned access + refresh token set. Reuses the `KiloCode` event as
/// the generic device-code carrier (`user_code` + `verification_url`). Sends
/// exactly one terminal event after an initial `KiloCode`.
async fn run_xai_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let http = reqwest::Client::new();
    let dc = match crate::service::oauth::xai::device_init(&http).await {
        Ok(dc) => dc,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };
    let _ = tx.send(OAuthEvent::KiloCode {
        user_code: dc.user_code.clone(),
        verification_url: dc.verification_url.clone(),
    });
    crate::service::oauth::browser::open_in_browser(&dc.verification_url);

    let tokens = match crate::service::oauth::xai::poll(
        &http,
        &dc.device_code,
        dc.expires_in,
        dc.interval,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };
    let conn = crate::service::oauth::xai::to_conn(tokens);
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
    // Disarm the GUI OAuth push side-channel regardless of who cancelled (a GUI
    // `CancelOAuth` request OR a TUI Esc keypress) — the flow is over, so no further
    // state should be pushed to a previously-armed client (path-agnostic disarm).
    state.rest.oauth_gui_client = None;
    match state.mode_mut() {
        Mode::Settings(s) => s.oauth_flow = OAuthFlowState::Idle,
        // The wizard has no idle connections list — cancelling a wait returns to the
        // provider picker.
        Mode::OnboardProvider(op) => op.oauth_flow = OAuthFlowState::Pick(0),
        _ => {}
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
    apply_login_result(&mut state.rest, conn, handle);
    Ok(())
}

/// Persist a completed OAuth login and fold it into the FOREGROUND session's mode:
/// seed the token-refresh cache (fire-and-forget), append to `config.oauth_conns`,
/// save `config.json`, then either rebuild `oauth_drafts` (`Mode::Settings`) or
/// advance the guided wizard to its model picker (`Mode::OnboardProvider`).
///
/// Synchronous callers only (`OAuthPaste`): the current session is the one that
/// initiated the paste, so folding into `fg()` is correct here — unlike the async
/// flow's `Success` event, which is de-globalized in the `service_global` drain (C3).
fn apply_login_result(
    rest: &mut AppStateRest,
    conn: OAuthConn,
    handle: &tokio::runtime::Handle,
) {
    let seeded = conn.clone();
    // Fire-and-forget the token-refresh cache seed on the explicit runtime handle
    // (the event loop may run outside a tokio runtime context).
    handle.spawn(async move {
        crate::service::oauth::manager::seed(&seeded).await;
    });
    let conn_uuid = conn.uuid.clone();
    let conn_provider = conn.provider;
    rest.config.oauth_conns.push(conn);
    // Pre-compute the outcome (drafts on success) BEFORE the mode borrow so the fold
    // below doesn't need to re-borrow `rest.config` while holding `&mut ...mode`.
    let outcome = rest
        .config
        .save()
        .map_err(|e| e.to_string())
        .map(|_| crate::app::mode::settings::OAuthDraft::from_config(&rest.config));
    match &mut rest.fg_mut().mode {
        Mode::Settings(s) => match outcome {
            Ok(drafts) => {
                s.oauth_drafts = drafts;
                s.oauth_flow = OAuthFlowState::Idle;
            }
            Err(e) => {
                s.oauth_flow = OAuthFlowState::Failed(format!(
                    "login saved locally but config write failed: {e}"
                ));
            }
        },
        // Guided wizard: advance straight to the model picker bound to the new conn.
        Mode::OnboardProvider(op) => match outcome {
            Ok(_) => {
                op.new_conn_uuid = conn_uuid;
                op.provider = Some(conn_provider);
                op.step = OnboardProviderStep::ModelSelect;
                op.oauth_flow = OAuthFlowState::Idle;
                op.query.clear();
                op.result_sel = 0;
            }
            Err(e) => {
                op.oauth_flow = OAuthFlowState::Failed(format!(
                    "login saved locally but config write failed: {e}"
                ));
            }
        },
        _ => {}
    }
}

/// Handle `Action::OAuthCopyUrl`: best-effort copy the active wait screen's URL
/// to the system clipboard. Synchronous, fire-and-forget — on success, sets
/// the active `CodexWait`/`KiloWait` variant's `copied` flag so the view can
/// show a confirmation line; on failure (or in any other `oauth_flow` state,
/// e.g. `Starting`, which has no URL yet) this is a silent no-op — no error
/// state churn.
pub(super) fn handle_oauth_copy_url(state: &mut AppState) -> Result<()> {
    let flow = match state.mode_mut() {
        Mode::Settings(s) => Some(&mut s.oauth_flow),
        Mode::OnboardProvider(op) => Some(&mut op.oauth_flow),
        _ => None,
    };
    if let Some(flow) = flow {
        match flow {
            OAuthFlowState::CodexWait { url, copied, .. } => {
                if crate::service::oauth::browser::copy_to_clipboard(url) {
                    *copied = true;
                }
            }
            OAuthFlowState::KiloWait { verification_url, copied, .. } => {
                if crate::service::oauth::browser::copy_to_clipboard(verification_url) {
                    *copied = true;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Handle `Action::OAuthOpenUrl`: re-open the active wait screen's URL in the
/// system browser (in case the initial automatic open didn't land, or the
/// user closed the tab). Synchronous, fire-and-forget — the result is ignored
/// the same way the initial flow-start open ignores it.
pub(super) fn handle_oauth_open_url(state: &mut AppState) -> Result<()> {
    let flow = match state.mode() {
        Mode::Settings(s) => Some(&s.oauth_flow),
        Mode::OnboardProvider(op) => Some(&op.oauth_flow),
        _ => None,
    };
    if let Some(flow) = flow {
        match flow {
            OAuthFlowState::CodexWait { url, .. } => {
                crate::service::oauth::browser::open_in_browser(url);
            }
            OAuthFlowState::KiloWait { verification_url, .. } => {
                crate::service::oauth::browser::open_in_browser(verification_url);
            }
            _ => {}
        }
    }
    Ok(())
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

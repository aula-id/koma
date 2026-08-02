//! Action handlers for the `/settings` OAuth submenu and `Mode::OnboardProvider`
//! guided wizard: OAuthStart, OAuthCancel, OAuthPaste, OAuthDelete.
//!
//! `OAuthStart` spawns the chosen provider's flow (`service::oauth::flow::run_flow` —
//! shared with the GUI host-relay's detached login path) on a background task (mirrors
//! `settings::handle_fetch_model_endpoints`): a fresh `oauth_rx` channel is opened, the
//! previous flow (if any) is aborted, and the spawned future sends `OAuthEvent`s the
//! event-loop's `service_global` drains into the open OAuth submenu or wizard.
//! `OAuthPaste` and `OAuthDelete` are synchronous — no background task, so they apply
//! directly within this bracketed action.

use anyhow::Result;

use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::{Mode, OnboardProviderStep};
use crate::app::state::{AppState, AppStateRest};
use crate::model::app_config::{OAuthConn, OAuthProvider};

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
        Mode::Settings(s) => s.oauth_flow = OAuthFlowState::Starting { provider },
        Mode::OnboardProvider(op) => op.oauth_flow = OAuthFlowState::Starting { provider },
        _ => {}
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    state.rest.oauth_rx = Some(rx);

    // The five per-provider flow bodies (Codex/Claude/Koma PKCE browser flows; Kilo
    // Code/xAI device flows) now live in `service::oauth::flow` — a SINGLE copy shared
    // with the GUI host-relay's detached `HostCtl::StartOAuth` path (`client::host`),
    // which spawns the exact same dispatcher for a pre-session login. No behaviour
    // change here: `run_flow` internally matches `provider` the same way this match
    // used to.
    let join = handle.spawn(crate::service::oauth::flow::run_flow(provider, tx));
    state.rest.oauth_task = Some(join.abort_handle());
    Ok(())
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
/// the same path a completed browser/device flow uses. The `provider` field
/// determines which conn constructor to use (Codex, CommandCode).
pub(super) fn handle_oauth_paste(
    provider: OAuthProvider,
    token: String,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let conn = match provider {
        OAuthProvider::Codex => {
            let tokens = crate::service::oauth::codex::TokenResponse {
                access_token: token.trim().to_string(),
                refresh_token: String::new(),
                id_token: String::new(),
                expires_in: None,
            };
            crate::service::oauth::codex::to_conn(tokens)
        }
        OAuthProvider::CommandCode => {
            crate::service::oauth::commandcode::to_conn(token.trim(), "", "")
        }
        other => {
            // Unsupported paste provider — set a failed message and return.
            match state.mode_mut() {
                Mode::Settings(s) => {
                    s.oauth_flow =
                        OAuthFlowState::Failed(format!("paste not supported for {:?}", other));
                }
                Mode::OnboardProvider(op) => {
                    op.oauth_flow =
                        OAuthFlowState::Failed(format!("paste not supported for {:?}", other));
                }
                _ => {}
            }
            return Ok(());
        }
    };
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
fn apply_login_result(rest: &mut AppStateRest, conn: OAuthConn, handle: &tokio::runtime::Handle) {
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
            OAuthFlowState::CodexWait { url, copied, .. }
                if crate::service::oauth::browser::copy_to_clipboard(url) =>
            {
                *copied = true;
            }
            OAuthFlowState::KiloWait {
                verification_url,
                copied,
                ..
            } if crate::service::oauth::browser::copy_to_clipboard(verification_url) => {
                *copied = true;
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
            OAuthFlowState::KiloWait {
                verification_url, ..
            } => {
                crate::service::oauth::browser::open_in_browser(verification_url);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Handle `Action::OAuthDelete`: remove the connection from `config.oauth_conns`,
/// cascade-drop models that pointed at it, rebind consumers → inherit, persist,
/// evict its token-refresh cache entry, and rebuild `oauth_drafts` in the open submenu.
pub(super) fn handle_oauth_delete(
    uuid: String,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    use std::collections::HashSet;
    let purge = state.rest.config.cascade_remove_oauth_conn(&uuid);
    // `HashSet` used below for rebind + draft filter.
    if let Err(e) = state.rest.config.save() {
        state.rest.fg_mut().status = format!("config save failed: {e}");
    } else {
        // Always walk agent .md files → inherit main for any model that no longer exists.
        let dead_models: HashSet<String> = purge.models_removed.iter().cloned().collect();
        let mut dead_providers = HashSet::new();
        dead_providers.insert(uuid.clone());
        let cfg = state.rest.config.clone();
        let report = crate::app::cascade::rebind_consumers_after_model_removal(
            Some(state),
            &cfg,
            &dead_models,
            &dead_providers,
            purge.main_reset,
        );
        if !purge.models_removed.is_empty() || report.agents_cleared > 0 || purge.main_reset {
            state
                .rest
                .fg_mut()
                .set_toast_info(crate::app::cascade::cascade_status_line("oauth", &report));
        }
    }
    let drafts = crate::app::mode::settings::OAuthDraft::from_config(&state.rest.config);
    if let Mode::Settings(s) = state.mode_mut() {
        s.oauth_drafts = drafts;
        s.oauth_sel = s.oauth_sel.min(s.oauth_drafts.len());
        s.oauth_armed = None;
        // Drop model drafts whose provider_uuid was the deleted oauth conn (or a
        // cascaded-away model uuid) so the open settings view matches disk.
        if !purge.models_removed.is_empty() {
            let dead: HashSet<String> = purge.models_removed.iter().cloned().collect();
            s.models.retain(|m| {
                // ModelDraft has uuid + provider resolved at load; filter by uuid.
                !dead.contains(&m.uuid)
            });
        }
    }
    handle.spawn(async move {
        crate::service::oauth::manager::evict(&uuid).await;
    });
    Ok(())
}

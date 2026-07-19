//! Provider-agnostic OAuth login FLOW bodies — the actual browser/device-code dance for
//! each of the five OAuth-backed providers (Codex, Claude, Koma, Kilo Code, xAI). Each
//! `run_*_flow` fn opens the system browser (or a device-code verification URL), waits on
//! the loopback callback / device poll, exchanges for tokens, and sends exactly one
//! terminal [`OAuthEvent`] (`Success`/`Failed`) after an initial progress event
//! (`CodexUrl`/`KiloCode`) — a dropped receiver (flow superseded/cancelled) makes every
//! send a silent no-op.
//!
//! Factored out of `app::runtime::actions::oauth` (0.2.28 → GUI-detached OAuth wave) so
//! BOTH callers — the daemon's `Action::OAuthStart` handler (an attached session's flow)
//! and the GUI host-relay's detached `HostCtl::StartOAuth` handler (the home-screen /
//! pre-session flow, `client::host::host_swapper`) — spawn the SAME code, sending
//! progress through whatever `OAuthEvent` sink the caller wires up. Neither caller's
//! behaviour changes: this is pure code motion plus the [`run_flow`] dispatcher that
//! replaces each caller's own provider `match`.

use super::OAuthEvent;
use crate::model::app_config::OAuthProvider;

/// Dispatch to the flow for `provider`, sending every progress/terminal event through
/// `tx`. The single entry point both the daemon and the detached host spawn — adding a
/// provider means adding one arm here, not touching either caller.
pub async fn run_flow(provider: OAuthProvider, tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    match provider {
        OAuthProvider::Codex => run_codex_flow(tx).await,
        OAuthProvider::Kilocode => run_kilo_flow(tx).await,
        OAuthProvider::Xai => run_xai_flow(tx).await,
        OAuthProvider::ClaudeAI => run_claude_flow(tx).await,
        OAuthProvider::KomaRun => run_komarun_flow(tx).await,
        OAuthProvider::KomaPremium => run_komarun_flow(tx).await,
        // W11: an extension-delegated flow is NEVER driven through here — it runs
        // off-loop in the daemon hub (`requests_oauth::run_ext_oauth_delegate`), keyed
        // by an `ext:<id>:<provider>` picker id, and never via `Action::OAuthStart`
        // (which is what spawns `run_flow`). This arm is exhaustiveness-only; terminate
        // defensively rather than silently doing nothing (which would hang the wait
        // screen on the "flow ended unexpectedly" disconnect path).
        OAuthProvider::Extension => {
            let _ = tx.send(OAuthEvent::Failed {
                error: "extension OAuth is delegated, not run natively".to_string(),
            });
        }
    }
}

/// The Codex browser flow: build the PKCE authorization URL, open the system browser,
/// wait on the loopback redirect, then exchange the code for tokens. Sends exactly one
/// terminal event (`Success` or `Failed`) after an initial `CodexUrl`; a dropped
/// receiver (flow superseded/cancelled) makes every send a silent no-op.
async fn run_codex_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let auth = super::codex::build_auth_url();
    let _ = tx.send(OAuthEvent::CodexUrl {
        url: auth.url.clone(),
    });
    super::browser::open_in_browser(&auth.url);

    let cb =
        match super::loopback::catch_callback(&auth.pkce.state, 300, super::registry::CODEX_PORT)
            .await
        {
            Ok(cb) => cb,
            Err(e) => {
                let _ = tx.send(OAuthEvent::Failed { error: e });
                return;
            }
        };

    let http = reqwest::Client::new();
    match super::codex::exchange_code(&http, &cb.code, &auth.pkce.verifier).await {
        Ok(tokens) => {
            let conn = super::codex::to_conn(tokens);
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
    let auth = super::claude::build_auth_url();
    let _ = tx.send(OAuthEvent::CodexUrl {
        url: auth.url.clone(),
    });
    super::browser::open_in_browser(&auth.url);

    let cb =
        match super::loopback::catch_callback(&auth.pkce.state, 300, super::registry::CLAUDE_PORT)
            .await
        {
            Ok(cb) => cb,
            Err(e) => {
                let _ = tx.send(OAuthEvent::Failed { error: e });
                return;
            }
        };

    let http = reqwest::Client::new();
    match super::claude::exchange_code(&http, &cb.code, &auth.pkce.verifier, &auth.pkce.state).await
    {
        Ok(tokens) => {
            let conn = super::claude::to_conn(tokens);
            let _ = tx.send(OAuthEvent::Success { conn });
        }
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
        }
    }
}

/// The Koma (koma.run) browser flow: build the PKCE authorization URL, open the system
/// browser, wait on the loopback redirect (port 51004), then exchange the code for
/// tokens. Mirrors `run_claude_flow` exactly, against koma.run's own (form-encoded, no
/// client_id/scope) endpoints.
async fn run_komarun_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let auth = super::komarun::build_auth_url();
    let _ = tx.send(OAuthEvent::CodexUrl {
        url: auth.url.clone(),
    });
    super::browser::open_in_browser(&auth.url);

    let cb =
        match super::loopback::catch_callback(&auth.pkce.state, 300, super::registry::KOMA_PORT)
            .await
        {
            Ok(cb) => cb,
            Err(e) => {
                let _ = tx.send(OAuthEvent::Failed { error: e });
                return;
            }
        };

    let http = reqwest::Client::new();
    match super::komarun::exchange_code(&http, &cb.code, &auth.pkce.verifier, &auth.pkce.state)
        .await
    {
        Ok(tokens) => {
            let conn = super::komarun::to_conn(tokens);
            let _ = tx.send(OAuthEvent::Success { conn });
        }
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
        }
    }
}

/// The Kilo Code device flow: request a device code, open the system browser to its
/// verification URL, poll for approval, then fetch the profile (org id / email) to
/// label the connection. Sends exactly one terminal event after an initial `KiloCode`.
async fn run_kilo_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let http = reqwest::Client::new();
    let dc = match super::kilo::device_init(&http).await {
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
    super::browser::open_in_browser(&dc.verification_url);

    let token = match super::kilo::poll(&http, &dc.code, dc.expires_in).await {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };
    let (org_id, email) = super::kilo::profile(&http, &token).await;
    let conn = super::kilo::to_conn(token, org_id, email);
    let _ = tx.send(OAuthEvent::Success { conn });
}

/// The xAI (Grok) device flow: request a device code, open the verification URL, poll
/// xAI's DISCOVERED token endpoint for approval, then build the connection from the
/// returned access + refresh token set. Reuses the `KiloCode` event as the generic
/// device-code carrier (`user_code` + `verification_url`). Sends exactly one terminal
/// event after an initial `KiloCode`.
async fn run_xai_flow(tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>) {
    let http = reqwest::Client::new();
    let dc = match super::xai::device_init(&http).await {
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
    super::browser::open_in_browser(&dc.verification_url);

    let tokens = match super::xai::poll(&http, &dc.device_code, dc.expires_in, dc.interval).await {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    };
    let conn = super::xai::to_conn(tokens);
    let _ = tx.send(OAuthEvent::Success { conn });
}

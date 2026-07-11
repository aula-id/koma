//! OAuth-surface arm bodies for [`super::core::DaemonHub`] — the streaming GUI OAuth
//! login flow (Codex browser PKCE, Kilo Code device code, Codex paste-token) split out of
//! `requests.rs` for file size. Every mutating handler drives the flow through the SAME
//! `Action::OAuth*` the TUI reaches (via `apply_action`), so the daemon never forks the flow
//! logic; this wave is wire + per-client push, not new flow behaviour. The one addition to
//! the shared actions is the GUI-push arm/disarm bookkeeping (below) — kept path-agnostic so
//! it stays correct no matter which client (GUI request or TUI keypress) drives a flow.
//!
//! # How progress reaches the GUI client
//!
//! `StartOAuth` spawns the flow via `Action::OAuthStart` (which stores `oauth_rx`/
//! `oauth_task` on `rest`, AND disarms `oauth_gui_client` as it supersedes any prior flow),
//! THEN arms `state.rest.oauth_gui_client` with the requesting client's id as the last write
//! — so a GUI-originated start routes here and a racing TUI-originated start (which never
//! re-arms) stays disarmed. The shared `Action::OAuthCancel` handler disarms too, and a
//! disconnecting armed client is disarmed on detach/socket-EOF, so a stale id can never
//! misroute another client's flow state (incl. the email/plan/account_id PII on success).
//! The flow's background events are drained OUTSIDE any client bracket by
//! `event_loop::global::drains::drain_oauth`, which — with the client armed — queues each
//! transition onto `state.rest.oauth_pushes`. [`drain_oauth_pushes`](DaemonHub::
//! drain_oauth_pushes) (run once per tick by the daemon loop) turns each queued push into a
//! seq'd [`DaemonEvent::OAuthState`] to that client. The drain's per-mode `oauth_flow` fold
//! and its config persist run UNCHANGED, so a TUI client in `Mode::Settings`/
//! `OnboardProvider` still renders the flow off its snapshot — TUI parity is preserved.

use std::sync::Arc;

use crate::app::runtime::actions::apply_action;
use crate::app::state::AppState;
use crate::controller::input::Action;
use crate::ipc::proto::{ClientRequest, DaemonEvent, OAuthConnWire, OAuthProviderWire};
use crate::model::app_config::OAuthProvider;
use crate::service::openrouter::OpenRouterClient;

use super::core::DaemonHub;

impl DaemonHub {
    /// Route the whole GUI OAuth family (the one read + the four mutations) to its specific
    /// handler below — called from `requests.rs`'s single OAuth group arm so that router
    /// stays thin (the per-handler flow docs live here). `requests.rs` only ever passes the
    /// five OAuth variants, so the `_` catch-all is unreachable in practice.
    pub(super) fn oauth(
        &mut self,
        idx: usize,
        req: ClientRequest,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        match req {
            ClientRequest::GetOAuthState => self.get_oauth_state(idx, state),
            ClientRequest::StartOAuth { provider } => {
                self.start_oauth(idx, state, client, handle, provider)
            }
            ClientRequest::SubmitOAuthPaste { token } => {
                self.submit_oauth_paste(idx, state, client, handle, token)
            }
            ClientRequest::CancelOAuth => self.cancel_oauth(idx, state, client, handle),
            ClientRequest::DeleteOAuthConn { uuid } => {
                self.delete_oauth_conn(idx, state, client, handle, uuid)
            }
            _ => {}
        }
    }

    /// GUI OAuth screen opened: reply with the current `idle` state (persisted connections
    /// and available providers). Strictly READ-ONLY (no attach / snapshot / foreground move)
    /// and ALWAYS replies — mirrors the `get_settings` / `list_agents` one-shot.
    pub(super) fn get_oauth_state(&mut self, idx: usize, state: &AppState) {
        self.send_oauth_state(idx, state, "idle", None, None, None, None);
    }

    /// Start an OAuth login flow for `provider` (`"codex"` / `"kilocode"` / `"xai"` /
    /// `"codex_paste"`).
    ///
    /// For the browser/device flows: reuse the EXISTING `Action::OAuthStart` machinery
    /// (spawns `run_codex_flow`/`run_kilo_flow`/`run_xai_flow`, stores `oauth_rx`/`oauth_task`
    /// — its mode fold is a no-op in the GUI daemon session's Chat mode), THEN ARM this client as the
    /// push target, and reply `starting`; subsequent progress streams via `drain_oauth_pushes`.
    /// Arming AFTER the action matters: `handle_oauth_start` disarms `oauth_gui_client` on
    /// every start path (a stale GUI arm can't survive a supersede), so this GUI-originated
    /// arm must be the last write. `codex_paste` is synchronous (no background task), so it
    /// just surfaces the paste input (`paste`) and needs no armed target — the token arrives
    /// via `SubmitOAuthPaste`.
    pub(super) fn start_oauth(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        provider: String,
    ) {
        let client_id = self.clients[idx].id;
        match provider.as_str() {
            "codex" | "kilocode" | "xai" | "claudeai" | "komarun" => {
                let p = match provider.as_str() {
                    "kilocode" => OAuthProvider::Kilocode,
                    "xai" => OAuthProvider::Xai,
                    "claudeai" => OAuthProvider::ClaudeAI,
                    "komarun" => OAuthProvider::KomaRun,
                    _ => OAuthProvider::Codex,
                };
                // Spawn the flow FIRST — `handle_oauth_start` DISARMS `oauth_gui_client`
                // as it supersedes any prior flow (path-agnostic disarm) — THEN arm THIS
                // client as the last write, so the GUI-originated arm wins and a racing
                // TUI-originated start (which never re-arms) correctly stays disarmed. The
                // arm lands before the next tick's `drain_oauth`, so the first transition
                // already routes here.
                let _ = apply_action(Action::OAuthStart(p), state, client, handle);
                state.rest.oauth_gui_client = Some(client_id);
                self.send_oauth_state(idx, state, "starting", None, None, None, None);
            }
            "codex_paste" => {
                self.send_oauth_state(idx, state, "paste", None, None, None, None);
            }
            other => {
                self.send_oauth_state(
                    idx,
                    state,
                    "failed",
                    None,
                    None,
                    None,
                    Some(format!("unknown oauth provider: {other}")),
                );
            }
        }
    }

    /// Complete the Codex paste-token flow: reuse the EXISTING `Action::OAuthPaste` path
    /// (build a connection from the raw token, seed the refresh cache, persist), then reply
    /// with the terminal `success` state carrying the freshly-persisted connection list.
    ///
    /// Guarded on a non-empty token: `handle_oauth_paste`/`apply_login_result` persist a
    /// Codex conn from ANY string with no armed-flow gate, so a stray/empty
    /// `SubmitOAuthPaste` would otherwise add a bogus conn. An empty/whitespace token is
    /// rejected — re-surface the `paste` input rather than persisting garbage.
    pub(super) fn submit_oauth_paste(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        token: String,
    ) {
        if token.trim().is_empty() {
            self.send_oauth_state(idx, state, "paste", None, None, None, None);
            return;
        }
        let _ = apply_action(Action::OAuthPaste(token), state, client, handle);
        self.send_oauth_state(idx, state, "success", None, None, None, None);
    }

    /// Cancel an in-flight OAuth flow: reuse the EXISTING `Action::OAuthCancel` path, which
    /// aborts the background task, drops its receiver, AND disarms `oauth_gui_client` (the
    /// shared handler does the disarm now, path-agnostic — so a late transition from the
    /// just-aborted flow can't re-push regardless of who cancelled), then reply `idle`.
    pub(super) fn cancel_oauth(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let _ = apply_action(Action::OAuthCancel, state, client, handle);
        self.send_oauth_state(idx, state, "idle", None, None, None, None);
    }

    /// Delete a persisted OAuth connection by `uuid`: reuse the EXISTING `Action::OAuthDelete`
    /// path (remove from `config.oauth_conns` + persist + evict the token-refresh cache), then
    /// reply with a fresh `idle` state carrying the updated connection list.
    pub(super) fn delete_oauth_conn(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        uuid: String,
    ) {
        let _ = apply_action(Action::OAuthDelete(uuid), state, client, handle);
        self.send_oauth_state(idx, state, "idle", None, None, None, None);
    }

    /// Build + send client `idx` a [`DaemonEvent::OAuthState`] for `phase`, always
    /// (re)building the TOKENLESS connection list from the live `config` and the
    /// provider catalogue from the data-driven registry — so every push keeps the webview
    /// store authoritative. Shared by every OAuth handler above AND by
    /// [`drain_oauth_pushes`](Self::drain_oauth_pushes). `send_to` delivers regardless of
    /// attach state (like `send_settings_values`), so the OAuth screen works pre-session.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn send_oauth_state(
        &mut self,
        idx: usize,
        state: &AppState,
        phase: &str,
        url: Option<String>,
        user_code: Option<String>,
        verification_url: Option<String>,
        error: Option<String>,
    ) {
        self.send_to(
            idx,
            DaemonEvent::OAuthState {
                phase: phase.to_string(),
                url,
                user_code,
                verification_url,
                error,
                conns: oauth_conn_wires(state),
                providers: oauth_provider_wires(),
            },
        );
    }

    /// Drain the GUI OAuth push outbox (`state.rest.oauth_pushes`) queued by `drain_oauth`
    /// and reply to each initiating client with a seq'd [`DaemonEvent::OAuthState`]. Called
    /// once per tick by the daemon loop (right after `drain_list_routes`), the EXACT
    /// `drain_list_models`/`drain_list_routes` mirror for a flow-progress transition: a
    /// background flow task can't advance the per-client seq, so the queued transition is
    /// turned into a `send_to` frame here. Delivered whether or not the client is
    /// session-attached (the OAuth screen is used pre-session); a vanished client is
    /// silently dropped.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_oauth_pushes(
        &mut self,
        state: &mut AppState,
    ) {
        if state.rest.oauth_pushes.is_empty() {
            return;
        }
        let pushes = std::mem::take(&mut state.rest.oauth_pushes);
        for p in pushes {
            if let Some(idx) = self.clients.iter().position(|c| c.id == p.client_id) {
                self.send_oauth_state(
                    idx,
                    state,
                    p.phase,
                    p.url,
                    p.user_code,
                    p.verification_url,
                    p.error,
                );
            }
        }
    }
}

/// Map the persisted `config.oauth_conns` to the TOKENLESS wire list for a
/// [`DaemonEvent::OAuthState`]. ONLY display/identity fields cross — the wire type
/// ([`OAuthConnWire`]) has no `access_token`/`refresh_token`/`id_token` field to set, so a
/// secret can never reach the webview.
fn oauth_conn_wires(state: &AppState) -> Vec<OAuthConnWire> {
    state
        .rest
        .config
        .oauth_conns
        .iter()
        .map(|c| OAuthConnWire {
            uuid: c.uuid.clone(),
            name: c.name.clone(),
            provider: c.provider.wire_id().to_string(),
            email: c.email.clone(),
            plan: c.plan.clone(),
            account_id: c.account_id.clone(),
        })
        .collect()
}

/// The available-provider wire list, from the data-driven
/// [`crate::service::oauth::registry::oauth_providers`] source of truth — so a new provider
/// surfaces in the webview by extending THAT list alone, never a wire builder.
fn oauth_provider_wires() -> Vec<OAuthProviderWire> {
    crate::service::oauth::registry::oauth_providers()
        .into_iter()
        .map(|(id, label, kind)| OAuthProviderWire {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
        })
        .collect()
}

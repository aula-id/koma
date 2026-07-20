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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use koma_extension::protocol::OAuthProviderDef;

use crate::app::ext::ExtHostManager;
use crate::app::runtime::actions::apply_action;
use crate::app::state::{AppState, ExtOAuthFlow};
use crate::controller::input::Action;
use crate::ipc::proto::{ClientRequest, DaemonEvent, OAuthConnWire, OAuthProviderWire};
use crate::model::app_config::{InstalledExtension, OAuthConn, OAuthProvider};
use crate::model::store;
use crate::service::oauth::OAuthEvent;
use crate::service::openrouter::OpenRouterClient;

use super::core::DaemonHub;

/// The wire grant string an extension must have been `granted` for its declared OAuth
/// providers to surface as picker rows (and for koma to drive its `oauth.*` invokes).
const OAUTH_CONTRIBUTE_WIRE: &str = "oauth:contribute";

/// Per-`oauth.*` invoke round-trip cap. Deliberately UNDERCUTS nothing here (there is no
/// outer reader like the panel bridge); it bounds one begin/poll invoke so a wedged
/// extension can't stall the flow indefinitely — the 5-minute overall budget still applies.
const EXT_OAUTH_INVOKE_TIMEOUT: Duration = Duration::from_secs(25);

/// Overall begin→poll budget before the delegated flow gives up with `failed: timed out`.
const EXT_OAUTH_OVERALL_BUDGET: Duration = Duration::from_secs(300);

/// How long to wait between `oauth.poll` invokes.
const EXT_OAUTH_POLL_INTERVAL: Duration = Duration::from_secs(3);

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
            "codex" | "kilocode" | "xai" | "claudeai" | "komarun" | "clinepass" | "commandcode" => {
                let p = match provider.as_str() {
                    "kilocode" => OAuthProvider::Kilocode,
                    "xai" => OAuthProvider::Xai,
                    "claudeai" => OAuthProvider::ClaudeAI,
                    "komarun" => OAuthProvider::KomaRun,
                    "clinepass" => OAuthProvider::ClinePass,
                    "commandcode" => OAuthProvider::CommandCode,
                    _ => OAuthProvider::Codex,
                };
                // W11: a native start also supersedes any in-flight DELEGATED ext flow.
                // `handle_oauth_start` aborts the `oauth_task` handle, but an ext poll task
                // runs on `spawn_blocking` and can't be aborted — so signal its cancel flag
                // here first (it exits at its next check; its sends fall on the now-dropped
                // receiver). No-op for a pure-native session (`oauth_ext_flow` is `None`).
                supersede_ext_flow(state);
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
                state.rest.oauth_paste_provider = OAuthProvider::Codex;
                self.send_oauth_state(idx, state, "paste", None, None, None, None);
            }
            "clinepass_paste" => {
                state.rest.oauth_paste_provider = OAuthProvider::ClinePass;
                self.send_oauth_state(idx, state, "paste", None, None, None, None);
            }
            "commandcode_paste" => {
                state.rest.oauth_paste_provider = OAuthProvider::CommandCode;
                self.send_oauth_state(idx, state, "paste", None, None, None, None);
            }
            // W11: an `ext:<extension_id>:<provider_id>` picker id delegates the whole
            // login to that extension over the `oauth.*` invoke contract.
            other if other.starts_with("ext:") => {
                self.start_oauth_ext(idx, state, handle, other);
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

    /// Start a DELEGATED extension OAuth flow for the picker id `ext:<ext_id>:<provider_id>`
    /// (W11). Validates SYNCHRONOUSLY on the loop (parse + installed + enabled + granted
    /// `oauth:contribute` + the manifest actually declares that provider + a live ext
    /// manager), pushing the terminal `failed` phase on any miss so the picker never hangs.
    /// On success it supersedes any prior flow, opens a fresh `oauth_rx` channel, and spawns
    /// the off-loop begin/poll task ([`run_ext_oauth_delegate`]) on `spawn_blocking` — which
    /// feeds the SAME `OAuthEvent` channel `drain_oauth` already drains, so the phase-push +
    /// config-persist + refresh-seed machinery is reused verbatim. Arms this client as the
    /// push target and replies `starting`.
    fn start_oauth_ext(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        id: &str,
    ) {
        let client_id = self.clients[idx].id;

        // A one-liner for every validation early-return: terminal `failed` push (keeps the
        // phase contract — the picker settles on Failed, never a stuck spinner).
        macro_rules! fail {
            ($msg:expr) => {{
                self.send_oauth_state(idx, state, "failed", None, None, None, Some($msg));
                return;
            }};
        }

        let Some((ext_id, provider_id)) = parse_ext_provider_id(id) else {
            fail!(format!("unknown oauth provider: {id}"));
        };
        let Some(record) = state
            .rest
            .config
            .installed_extensions
            .iter()
            .find(|e| e.id == ext_id)
            .cloned()
        else {
            fail!("unknown oauth provider (extension not installed)".to_string());
        };
        if !record.enabled {
            fail!("extension is disabled".to_string());
        }
        if !record.granted.iter().any(|g| g == OAUTH_CONTRIBUTE_WIRE) {
            fail!("extension lacks the oauth:contribute grant".to_string());
        }
        // Capture the declared provider def (not just its presence) — W12 reads its
        // `chat_endpoint`/`api_type`/`refresh` to stamp the ext-backed conn's model-provider
        // meta on a successful login, so an ext token becomes a resolvable model provider.
        let Some(provider_def) = read_ext_oauth_providers(&ext_id)
            .into_iter()
            .find(|p| p.id == provider_id)
        else {
            fail!("extension does not declare this oauth provider".to_string());
        };
        let Some(mgr) = state.rest.ext_manager.clone() else {
            fail!("extension not available".to_string());
        };

        // Supersede any prior flow (native OR ext), mirroring `handle_oauth_start`'s
        // supersede WITHOUT its TUI mode fold (the GUI daemon session is in Chat, where the
        // fold is a no-op). Abort the prior task handle (no-op for a `spawn_blocking` ext
        // task) AND signal a prior ext flow's cancel flag; then open a fresh channel.
        if let Some(h) = state.rest.oauth_task.take() {
            h.abort();
        }
        supersede_ext_flow(state);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<OAuthEvent>();
        state.rest.oauth_rx = Some(rx);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_task = Arc::clone(&cancel);
        let provider_for_task = provider_id.clone();
        let join = handle.spawn_blocking(move || {
            run_ext_oauth_delegate(&mgr, record, &provider_for_task, &provider_def, &cancel_task, tx);
        });
        state.rest.oauth_task = Some(join.abort_handle());
        state.rest.oauth_ext_flow = Some(ExtOAuthFlow {
            ext_id,
            provider_id,
            cancel,
        });
        // Arm THIS client as the push target LAST (mirrors the native path), so the first
        // `drain_oauth` transition already routes here.
        state.rest.oauth_gui_client = Some(client_id);
        self.send_oauth_state(idx, state, "starting", None, None, None, None);
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
        let provider = state.rest.oauth_paste_provider;
        state.rest.oauth_paste_provider = OAuthProvider::Codex; // reset after use
        let _ = apply_action(Action::OAuthPaste { provider, token }, state, client, handle);
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
        // W11: if a DELEGATED ext flow is in flight, signal its off-loop poll task to exit
        // (it can't be aborted) and fire a best-effort `oauth.cancel` at the extension
        // (2s, result ignored) so it can tear down its own pending login. Done BEFORE the
        // shared cancel path resets the channel. No-op for a native flow.
        if let Some(flow) = state.rest.oauth_ext_flow.take() {
            flow.cancel.store(true, Ordering::SeqCst);
            if let Some(mgr) = state.rest.ext_manager.clone() {
                let ext_id = flow.ext_id;
                let provider_id = flow.provider_id;
                handle.spawn_blocking(move || {
                    let _ = mgr.invoke_with_timeout(
                        &ext_id,
                        "oauth.cancel",
                        serde_json::json!({ "providerId": provider_id }),
                        Duration::from_secs(2),
                    );
                });
            }
        }
        let _ = apply_action(Action::OAuthCancel, state, client, handle);
        state.rest.oauth_paste_provider = OAuthProvider::Codex; // reset on cancel
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
                providers: oauth_provider_wires(&state.rest.config.installed_extensions),
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
            // W11: a terminal transition (`success`/`failed`) ends any in-flight delegated
            // ext flow — its off-loop poll task has already exited on its own, so just drop
            // the tracking. Cleared regardless of whether the initiating client still
            // exists. No-op for a native flow (it never sets `oauth_ext_flow`).
            let terminal = p.phase == "success" || p.phase == "failed";
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
            if terminal {
                state.rest.oauth_ext_flow = None;
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

/// The available-provider wire list: the NATIVE providers from the data-driven
/// [`crate::service::oauth::registry::oauth_providers`] source of truth, then the W11
/// EXTENSION-contributed rows appended (so `registry.rs`'s signature stays stable — new
/// native providers extend THAT list, ext rows are surfaced here from the installed set).
fn oauth_provider_wires(installed: &[InstalledExtension]) -> Vec<OAuthProviderWire> {
    let mut wires: Vec<OAuthProviderWire> = crate::service::oauth::registry::oauth_providers()
        .into_iter()
        .map(|(id, label, kind)| OAuthProviderWire {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
        })
        .collect();
    wires.extend(ext_oauth_provider_rows(installed));
    wires
}

/// The W11 EXTENSION-contributed OAuth picker rows: for every installed extension, read
/// its manifest's declared providers (only when it is enabled AND holds the
/// `oauth:contribute` grant — a manifest read is skipped otherwise) and turn each into a
/// picker row via the pure [`ext_oauth_rows_for`]. Best-effort: an extension with an
/// unreadable/unparsable manifest simply contributes no rows.
fn ext_oauth_provider_rows(installed: &[InstalledExtension]) -> Vec<OAuthProviderWire> {
    let mut rows = Vec::new();
    for ext in installed {
        // Skip the manifest read entirely for a disqualified extension (disabled or
        // ungranted) — the pure builder re-checks, so this is only an optimisation.
        let providers = if ext.enabled && ext.granted.iter().any(|g| g == OAUTH_CONTRIBUTE_WIRE) {
            read_ext_oauth_providers(&ext.id)
        } else {
            Vec::new()
        };
        rows.extend(ext_oauth_rows_for(&ext.id, ext.enabled, &ext.granted, &providers));
    }
    rows
}

/// PURE ext-row builder (unit-tested): one picker row per declared provider, gated on the
/// extension being ENABLED and holding the `oauth:contribute` grant. Returns empty when
/// disabled, ungranted, or there are no declared providers. The row `id` is the routing key
/// `ext:<ext_id>:<provider_id>` [`start_oauth`] parses back; the `kind` is the manifest
/// `method` mapped to the GUI badge (see [`method_to_kind`]).
fn ext_oauth_rows_for(
    ext_id: &str,
    enabled: bool,
    granted: &[String],
    providers: &[OAuthProviderDef],
) -> Vec<OAuthProviderWire> {
    if !enabled {
        return Vec::new();
    }
    if !granted.iter().any(|g| g == OAUTH_CONTRIBUTE_WIRE) {
        return Vec::new();
    }
    providers
        .iter()
        .map(|p| OAuthProviderWire {
            id: format!("ext:{ext_id}:{}", p.id),
            label: p.name.clone(),
            kind: method_to_kind(&p.method).to_string(),
        })
        .collect()
}

/// Map an [`OAuthProviderDef::method`] to the GUI picker badge `kind`: `"browser"` → `pkce`,
/// `"device_code"` → `device`, `"paste"` → `paste`. An unrecognised method falls back to
/// `pkce` (the GUI renders any non-`device`/`paste` kind as the "browser" badge anyway).
fn method_to_kind(method: &str) -> &'static str {
    match method {
        "browser" => "pkce",
        "device_code" => "device",
        "paste" => "paste",
        _ => "pkce",
    }
}

/// Best-effort read of an installed extension's declared `contributes.oauth_providers`
/// straight off `extensions_dir()/<id>/manifest.json` (the registry entry doesn't carry
/// contributions). Mirrors [`super::requests_ext::read_ext_panels`]: a
/// missing/unreadable/unparsable manifest yields an empty list, logged via
/// `append_global_error_log` so a parse failure is still visible.
fn read_ext_oauth_providers(id: &str) -> Vec<OAuthProviderDef> {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return Vec::new(),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<koma_extension::protocol::ExtensionManifest>(&raw) {
        Ok(m) => m.contributes.oauth_providers,
        Err(e) => {
            store::append_global_error_log(
                "ext oauth",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            Vec::new()
        }
    }
}

/// Parse a picker id `ext:<ext_id>:<provider_id>` into its two parts. `None` for anything
/// that isn't that exact shape (missing prefix, missing separator, or an empty part). PURE.
/// Extension ids are reverse-DNS (`[A-Za-z0-9._-]`, colon-free), so splitting on the FIRST
/// `:` after the prefix isolates the id cleanly.
fn parse_ext_provider_id(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("ext:")?;
    let (ext_id, provider_id) = rest.split_once(':')?;
    if ext_id.is_empty() || provider_id.is_empty() {
        return None;
    }
    Some((ext_id.to_string(), provider_id.to_string()))
}

/// Signal any in-flight DELEGATED ext flow's off-loop poll task to exit (it can't be
/// aborted like a native async task) and drop its tracking. A no-op when no ext flow is in
/// flight. Shared by the native-start supersede and `start_oauth_ext`'s own supersede.
fn supersede_ext_flow(state: &mut AppState) {
    if let Some(flow) = state.rest.oauth_ext_flow.take() {
        flow.cancel.store(true, Ordering::SeqCst);
    }
}

// ─── delegated ext OAuth flow (W11) ─────────────────────────────────────────────────────

/// What `oauth.begin` asked koma to surface (parsed from the extension's reply). PURE
/// classification, so the reply→outcome mapping is unit-testable without a live extension.
#[derive(Debug, PartialEq, Eq)]
enum BeginOutcome {
    /// Browser method: surface `url` (the `waiting_url` phase; koma does NOT auto-open it).
    Browser { url: String },
    /// Device method: surface a user code + verification URL (the `waiting_code` phase).
    Device { user_code: String, verification_url: String },
    /// The begin step failed (or returned an unusable reply).
    Failed(String),
}

/// A successful `oauth.poll` token payload. Only `access_token` is required; the rest are
/// optional identity / lifecycle hints koma stores on the ext-backed [`OAuthConn`].
#[derive(Debug, PartialEq, Eq)]
struct ExtToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
    email: Option<String>,
    label: Option<String>,
}

/// The decision for one `oauth.poll` reply. PURE, so the poll-loop logic is unit-testable.
#[derive(Debug, PartialEq, Eq)]
enum PollDecision {
    /// Still pending (or an unrecognised-but-non-terminal reply) — keep polling.
    Continue,
    /// Login completed — persist this token as an ext-backed connection.
    Success(ExtToken),
    /// The flow failed terminally.
    Failed(String),
}

/// Classify an `oauth.begin` reply. A bare `{ "error": … }` (or any reply lacking BOTH a
/// device code and a url) is `Failed`; a device code (`userCode` + `verificationUrl`) wins
/// over a `url` when both are present. PURE.
fn parse_begin(reply: &Value) -> BeginOutcome {
    if let Some(err) = reply.get("error").and_then(Value::as_str) {
        return BeginOutcome::Failed(err.to_string());
    }
    let user_code = reply
        .get("userCode")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let verification_url = reply
        .get("verificationUrl")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if let (Some(uc), Some(vu)) = (user_code, verification_url) {
        return BeginOutcome::Device {
            user_code: uc.to_string(),
            verification_url: vu.to_string(),
        };
    }
    if let Some(url) = reply
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return BeginOutcome::Browser {
            url: url.to_string(),
        };
    }
    BeginOutcome::Failed(
        "extension oauth.begin returned neither a url nor a device code".to_string(),
    )
}

/// Classify an `oauth.poll` reply into a [`PollDecision`]. `{"status":"success","token":{…}}`
/// → `Success` (requires a non-empty `access_token`, else `Failed`); `{"status":"failed",…}`
/// or a bare `{"error":…}` → `Failed`; `{"status":"pending"}`, an unknown status, or a
/// reply with neither status nor error → `Continue` (keep polling until the overall budget).
/// PURE.
fn decide_poll(reply: &Value) -> PollDecision {
    match reply.get("status").and_then(Value::as_str) {
        Some("success") => {
            let token = reply.get("token");
            let access = token
                .and_then(|t| t.get("access_token"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if access.is_empty() {
                return PollDecision::Failed(
                    "extension reported success without an access_token".to_string(),
                );
            }
            let field = |k: &str| {
                token
                    .and_then(|t| t.get(k))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            PollDecision::Success(ExtToken {
                access_token: access,
                refresh_token: field("refresh_token"),
                expires_at: token.and_then(|t| t.get("expires_at")).and_then(Value::as_u64),
                email: field("email"),
                label: field("label"),
            })
        }
        Some("failed") => PollDecision::Failed(
            reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("extension OAuth failed")
                .to_string(),
        ),
        Some("pending") => PollDecision::Continue,
        // An unknown status is treated as still-in-progress (bounded by the overall budget).
        Some(_) => PollDecision::Continue,
        None => match reply.get("error").and_then(Value::as_str) {
            Some(err) => PollDecision::Failed(err.to_string()),
            None => PollDecision::Continue,
        },
    }
}

/// Build the ext-backed [`OAuthConn`] koma persists on a successful delegated login. The
/// uuid is minted HOST-side; `provider` is the [`OAuthProvider::Extension`] marker with the
/// real identity carried in `ext_id`/`provider_id`. Only `access_token` is guaranteed
/// present; the rest are best-effort from the extension's token payload.
///
/// W12: the model-provider meta (`chat_endpoint`/`api_type`/refresh descriptor) is stamped
/// from the manifest [`OAuthProviderDef`] `def`. `api_type` is NORMALIZED to `"openai"` /
/// `"anthropic"` (an unrecognised or absent value stores `None`), so a conn whose manifest
/// declared no usable model endpoint stays account-login-only — [`models.register`] refuses
/// it and resolution treats a referencing entry as dangling. Empty declared strings collapse
/// to `None` (nothing to route / refresh against).
fn build_ext_conn(
    ext_id: &str,
    provider_id: &str,
    def: &OAuthProviderDef,
    token: ExtToken,
) -> OAuthConn {
    let non_empty = |s: &str| -> Option<String> {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    };
    let chat_endpoint = def.chat_endpoint.as_deref().and_then(non_empty);
    let api_type = normalize_ext_api_type(def.api_type.as_deref());
    let (refresh_token_url, refresh_client_id) = match &def.refresh {
        Some(r) => (non_empty(&r.token_url), non_empty(&r.client_id)),
        None => (None, None),
    };
    OAuthConn {
        uuid: uuid::Uuid::new_v4().to_string(),
        name: token
            .label
            .unwrap_or_else(|| format!("{ext_id}:{provider_id}")),
        provider: OAuthProvider::Extension,
        access_token: token.access_token,
        refresh_token: token.refresh_token.unwrap_or_default(),
        id_token: String::new(),
        expires_at: token.expires_at.unwrap_or(0),
        last_refresh: 0,
        account_id: String::new(),
        org_id: String::new(),
        email: token.email.unwrap_or_default(),
        plan: String::new(),
        ext_id: Some(ext_id.to_string()),
        provider_id: Some(provider_id.to_string()),
        chat_endpoint,
        api_type,
        refresh_token_url,
        refresh_client_id,
    }
}

/// W12: normalize a manifest [`OAuthProviderDef::api_type`] to the stored wire string koma
/// resolves. Only `"openai"` and `"anthropic"` are model-provider wire types koma can
/// dispatch (mapping to `OpenAiCompatible` / `AnthropicCompatible` at resolution — see
/// [`crate::model::app_config::OAuthConn::ext_model_route`]); anything else — an
/// account-login-only provider that omits `api_type`, or an unknown/legacy value like
/// `"openai_compatible"` — stores `None`, which `models.register` later refuses as
/// "provider is account-login only".
fn normalize_ext_api_type(api_type: Option<&str>) -> Option<String> {
    match api_type.map(str::trim) {
        Some("openai") => Some("openai".to_string()),
        Some("anthropic") => Some("anthropic".to_string()),
        _ => None,
    }
}

/// Sleep up to `total`, checking `cancel` every 200ms so a supersede/cancel is noticed
/// promptly (a `spawn_blocking` task can't be aborted). Returns `true` if cancelled.
fn sleep_cancellable(total: Duration, cancel: &AtomicBool) -> bool {
    const STEP: Duration = Duration::from_millis(200);
    let mut elapsed = Duration::ZERO;
    while elapsed < total {
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        let nap = STEP.min(total - elapsed);
        std::thread::sleep(nap);
        elapsed += nap;
    }
    cancel.load(Ordering::SeqCst)
}

/// The OFF-LOOP body of a delegated ext OAuth flow (runs on `spawn_blocking` — both
/// [`ExtHostManager::ensure_started`] and [`ExtHostManager::invoke_with_timeout`] block the
/// calling thread). Drives the extension through `oauth.begin` → repeated `oauth.poll` and
/// emits [`OAuthEvent`]s on `tx` — the SAME channel `drain_oauth` drains, so the
/// phase-push, config-persist, and refresh-seed machinery is reused verbatim (a browser
/// reply becomes `CodexUrl`/`waiting_url`, a device reply `KiloCode`/`waiting_code`, a
/// completed login `Success`, any failure `Failed`).
///
/// Cancellation is COOPERATIVE via `cancel` (a `spawn_blocking` task can't be aborted): it
/// is checked before every blocking step and between poll naps, and on cancel the task
/// returns WITHOUT a terminal event (the shared cancel path has already reset `oauth_rx` +
/// disarmed the push client, so the dropped `tx` is never observed and nothing is pushed).
/// Every OTHER exit path emits exactly one terminal event, so the wait screen never hangs.
fn run_ext_oauth_delegate(
    mgr: &Arc<ExtHostManager>,
    record: InstalledExtension,
    provider_id: &str,
    provider_def: &OAuthProviderDef,
    cancel: &AtomicBool,
    tx: tokio::sync::mpsc::UnboundedSender<OAuthEvent>,
) {
    // Superseded/cancelled before we even started.
    if cancel.load(Ordering::SeqCst) {
        return;
    }
    // Delegated OAuth needs a PERSISTENT daemon: the begin/poll handshake holds state (a
    // pending device code) across invokes; a oneshot is respawned per invoke.
    if record.kind != "daemon" {
        let _ = tx.send(OAuthEvent::Failed {
            error: "extension OAuth requires a daemon-kind extension".to_string(),
        });
        return;
    }
    if let Err(e) = mgr.ensure_started(&record) {
        let _ = tx.send(OAuthEvent::Failed {
            error: format!("extension failed to start: {e:#}"),
        });
        return;
    }
    if cancel.load(Ordering::SeqCst) {
        return;
    }

    // Fresh `{ "providerId": … }` params per invoke (provider_id is borrowed, &str is Copy).
    let params = || serde_json::json!({ "providerId": provider_id });

    let begin = mgr.invoke_with_timeout(&record.id, "oauth.begin", params(), EXT_OAUTH_INVOKE_TIMEOUT);
    if cancel.load(Ordering::SeqCst) {
        return;
    }
    let begin = match begin {
        Ok(v) => v,
        Err(e) => {
            let _ = tx.send(OAuthEvent::Failed {
                error: format!("oauth.begin failed: {e:#}"),
            });
            return;
        }
    };
    match parse_begin(&begin) {
        BeginOutcome::Browser { url } => {
            let _ = tx.send(OAuthEvent::CodexUrl { url });
        }
        BeginOutcome::Device {
            user_code,
            verification_url,
        } => {
            let _ = tx.send(OAuthEvent::KiloCode {
                user_code,
                verification_url,
            });
        }
        BeginOutcome::Failed(e) => {
            let _ = tx.send(OAuthEvent::Failed { error: e });
            return;
        }
    }

    let deadline = Instant::now() + EXT_OAUTH_OVERALL_BUDGET;
    loop {
        if sleep_cancellable(EXT_OAUTH_POLL_INTERVAL, cancel) {
            return;
        }
        if Instant::now() >= deadline {
            let _ = tx.send(OAuthEvent::Failed {
                error: "extension OAuth timed out".to_string(),
            });
            return;
        }
        let poll = mgr.invoke_with_timeout(&record.id, "oauth.poll", params(), EXT_OAUTH_INVOKE_TIMEOUT);
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let poll = match poll {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(OAuthEvent::Failed {
                    error: format!("oauth.poll failed: {e:#}"),
                });
                return;
            }
        };
        match decide_poll(&poll) {
            PollDecision::Continue => continue,
            PollDecision::Success(token) => {
                let conn = build_ext_conn(&record.id, provider_id, provider_def, token);
                let _ = tx.send(OAuthEvent::Success { conn });
                return;
            }
            PollDecision::Failed(e) => {
                let _ = tx.send(OAuthEvent::Failed { error: e });
                return;
            }
        }
    }
}

#[cfg(test)]
mod ext_oauth_tests {
    use super::*;
    use serde_json::json;

    fn def(id: &str, name: &str, method: &str) -> OAuthProviderDef {
        OAuthProviderDef {
            id: id.to_string(),
            name: name.to_string(),
            method: method.to_string(),
            chat_endpoint: None,
            api_type: None,
            refresh: None,
        }
    }

    // ── row surfacing (pure builder) ─────────────────────────────────────────────────

    /// Granted + enabled + a declared provider → exactly one row, with the exact
    /// `ext:<ext_id>:<provider_id>` id, the provider `name` as the label, and the badge kind
    /// mapped from `method`.
    #[test]
    fn rows_for_granted_declared_provider() {
        let providers = [def("demo", "Demo Login", "device_code")];
        let rows = ext_oauth_rows_for(
            "run.koma.example.oauth-demo-daemon",
            true,
            &["oauth:contribute".to_string()],
            &providers,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "ext:run.koma.example.oauth-demo-daemon:demo");
        assert_eq!(rows[0].label, "Demo Login");
        assert_eq!(rows[0].kind, "device");
    }

    /// A declared provider but NO `oauth:contribute` grant → no rows (the authorization gate).
    #[test]
    fn rows_none_without_grant() {
        let providers = [def("demo", "Demo Login", "device_code")];
        assert!(ext_oauth_rows_for("ext.a", true, &[], &providers).is_empty());
        // An unrelated grant does not unlock it either.
        assert!(ext_oauth_rows_for("ext.a", true, &["agents:read".to_string()], &providers).is_empty());
    }

    /// A disabled extension contributes no rows even when granted + declaring providers.
    #[test]
    fn rows_none_when_disabled() {
        let providers = [def("demo", "Demo Login", "browser")];
        assert!(ext_oauth_rows_for("ext.a", false, &["oauth:contribute".to_string()], &providers).is_empty());
    }

    /// Granted + enabled but declaring NO providers → no rows.
    #[test]
    fn rows_none_without_declared_providers() {
        assert!(ext_oauth_rows_for("ext.a", true, &["oauth:contribute".to_string()], &[]).is_empty());
    }

    /// Multiple declared providers → one row each, ids kept distinct.
    #[test]
    fn rows_one_per_declared_provider() {
        let providers = [def("gh", "GitHub", "browser"), def("gl", "GitLab", "device_code")];
        let rows = ext_oauth_rows_for("acme.ext", true, &["oauth:contribute".to_string()], &providers);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "ext:acme.ext:gh");
        assert_eq!(rows[0].kind, "pkce");
        assert_eq!(rows[1].id, "ext:acme.ext:gl");
        assert_eq!(rows[1].kind, "device");
    }

    #[test]
    fn method_maps_to_badge_kind() {
        assert_eq!(method_to_kind("browser"), "pkce");
        assert_eq!(method_to_kind("device_code"), "device");
        assert_eq!(method_to_kind("paste"), "paste");
        // Unknown method → browser badge.
        assert_eq!(method_to_kind("carrier_pigeon"), "pkce");
    }

    // ── picker-id parsing (start_oauth routing) ─────────────────────────────────────

    #[test]
    fn parse_ext_id_valid() {
        assert_eq!(
            parse_ext_provider_id("ext:run.koma.example.oauth-demo-daemon:demo"),
            Some(("run.koma.example.oauth-demo-daemon".to_string(), "demo".to_string()))
        );
    }

    #[test]
    fn parse_ext_id_malformed_is_none() {
        // Not an ext id (a native provider).
        assert_eq!(parse_ext_provider_id("codex"), None);
        // Prefix but no separator.
        assert_eq!(parse_ext_provider_id("ext:justanid"), None);
        // Empty ext id or empty provider id.
        assert_eq!(parse_ext_provider_id("ext::demo"), None);
        assert_eq!(parse_ext_provider_id("ext:some.ext:"), None);
        // Empty / prefix-only.
        assert_eq!(parse_ext_provider_id(""), None);
        assert_eq!(parse_ext_provider_id("ext:"), None);
    }

    // ── oauth.begin classification ──────────────────────────────────────────────────

    #[test]
    fn begin_browser() {
        assert_eq!(
            parse_begin(&json!({ "url": "https://example.com/auth" })),
            BeginOutcome::Browser { url: "https://example.com/auth".to_string() }
        );
    }

    #[test]
    fn begin_device() {
        assert_eq!(
            parse_begin(&json!({ "userCode": "ABCD-1234", "verificationUrl": "https://example.com/activate" })),
            BeginOutcome::Device {
                user_code: "ABCD-1234".to_string(),
                verification_url: "https://example.com/activate".to_string(),
            }
        );
    }

    #[test]
    fn begin_error_and_empty_are_failed() {
        assert!(matches!(parse_begin(&json!({ "error": "nope" })), BeginOutcome::Failed(e) if e == "nope"));
        // Neither a url nor a (complete) device code → failed, never a stuck spinner.
        assert!(matches!(parse_begin(&json!({})), BeginOutcome::Failed(_)));
        assert!(matches!(parse_begin(&json!({ "userCode": "X" })), BeginOutcome::Failed(_)));
        assert!(matches!(parse_begin(&json!({ "url": "" })), BeginOutcome::Failed(_)));
    }

    // ── oauth.poll decision ─────────────────────────────────────────────────────────

    #[test]
    fn poll_pending_continues() {
        assert_eq!(decide_poll(&json!({ "status": "pending" })), PollDecision::Continue);
        // Unknown status and empty reply are both non-terminal (keep polling until budget).
        assert_eq!(decide_poll(&json!({ "status": "warming_up" })), PollDecision::Continue);
        assert_eq!(decide_poll(&json!({})), PollDecision::Continue);
    }

    #[test]
    fn poll_success_maps_token() {
        let d = decide_poll(&json!({
            "status": "success",
            "token": {
                "access_token": "at-123",
                "refresh_token": "rt-456",
                "expires_at": 1_800_000_000u64,
                "email": "me@example.com",
                "label": "My Account"
            }
        }));
        assert_eq!(
            d,
            PollDecision::Success(ExtToken {
                access_token: "at-123".to_string(),
                refresh_token: Some("rt-456".to_string()),
                expires_at: Some(1_800_000_000),
                email: Some("me@example.com".to_string()),
                label: Some("My Account".to_string()),
            })
        );
    }

    #[test]
    fn poll_success_minimal_token() {
        // Only access_token → the rest default to None.
        let d = decide_poll(&json!({ "status": "success", "token": { "access_token": "at-only" } }));
        assert_eq!(
            d,
            PollDecision::Success(ExtToken {
                access_token: "at-only".to_string(),
                refresh_token: None,
                expires_at: None,
                email: None,
                label: None,
            })
        );
    }

    #[test]
    fn poll_success_without_access_token_is_failed() {
        // A "success" with an empty/missing access_token is a protocol violation → failed.
        assert!(matches!(
            decide_poll(&json!({ "status": "success", "token": { "access_token": "" } })),
            PollDecision::Failed(_)
        ));
        assert!(matches!(
            decide_poll(&json!({ "status": "success", "token": {} })),
            PollDecision::Failed(_)
        ));
        assert!(matches!(
            decide_poll(&json!({ "status": "success" })),
            PollDecision::Failed(_)
        ));
    }

    #[test]
    fn poll_failed_and_bare_error() {
        assert!(matches!(
            decide_poll(&json!({ "status": "failed", "error": "user denied" })),
            PollDecision::Failed(e) if e == "user denied"
        ));
        // A bare error object (no status) is terminal too — malformed replies never hang.
        assert!(matches!(
            decide_poll(&json!({ "error": "extension crashed" })),
            PollDecision::Failed(e) if e == "extension crashed"
        ));
        // A "failed" without an error message still fails with a default reason.
        assert!(matches!(decide_poll(&json!({ "status": "failed" })), PollDecision::Failed(_)));
    }

    // ── conn construction ───────────────────────────────────────────────────────────

    #[test]
    fn build_conn_stamps_ext_identity() {
        let token = ExtToken {
            access_token: "at".to_string(),
            refresh_token: Some("rt".to_string()),
            expires_at: Some(42),
            email: Some("e@x.test".to_string()),
            label: Some("Nice Label".to_string()),
        };
        // An account-login-only def (no chat_endpoint/api_type) → the conn is NOT a model
        // provider (its W12 meta stays None).
        let conn = build_ext_conn("run.koma.ext.demo", "demo", &def("demo", "Demo", "browser"), token);
        assert_eq!(conn.provider, OAuthProvider::Extension);
        assert_eq!(conn.ext_id.as_deref(), Some("run.koma.ext.demo"));
        assert_eq!(conn.provider_id.as_deref(), Some("demo"));
        assert_eq!(conn.name, "Nice Label"); // label wins
        assert_eq!(conn.access_token, "at");
        assert_eq!(conn.refresh_token, "rt");
        assert_eq!(conn.expires_at, 42);
        assert_eq!(conn.email, "e@x.test");
        assert!(!conn.uuid.is_empty()); // minted host-side
        assert!(conn.chat_endpoint.is_none());
        assert!(conn.api_type.is_none());
        assert!(conn.ext_model_route().is_none(), "account-login-only conn is not a model provider");
    }

    /// W12: a def declaring a chat endpoint + a recognised api_type + a refresh descriptor
    /// stamps the conn's model-provider meta, so the ext token becomes a resolvable provider.
    #[test]
    fn build_conn_stamps_model_provider_meta() {
        let provider_def = OAuthProviderDef {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            method: "browser".to_string(),
            chat_endpoint: Some("https://api.demo.test/v1".to_string()),
            api_type: Some("openai".to_string()),
            refresh: Some(koma_extension::protocol::OAuthRefreshDef {
                token_url: "https://demo.test/token".to_string(),
                client_id: "cid".to_string(),
            }),
        };
        let token = ExtToken {
            access_token: "at".to_string(),
            refresh_token: Some("rt".to_string()),
            expires_at: Some(42),
            email: None,
            label: None,
        };
        let conn = build_ext_conn("e.ext", "demo", &provider_def, token);
        assert_eq!(conn.chat_endpoint.as_deref(), Some("https://api.demo.test/v1"));
        assert_eq!(conn.api_type.as_deref(), Some("openai"));
        assert_eq!(conn.refresh_token_url.as_deref(), Some("https://demo.test/token"));
        assert_eq!(conn.refresh_client_id.as_deref(), Some("cid"));
        assert!(conn.ext_model_route().is_some(), "a declared model provider resolves");
    }

    /// W12: only `"openai"`/`"anthropic"` are accepted api_type wires; anything else (an
    /// unknown/legacy value, or absent) normalizes to `None` (account-login-only).
    #[test]
    fn normalize_ext_api_type_accepts_only_known_wires() {
        assert_eq!(normalize_ext_api_type(Some("openai")).as_deref(), Some("openai"));
        assert_eq!(normalize_ext_api_type(Some("  anthropic  ")).as_deref(), Some("anthropic"));
        assert_eq!(normalize_ext_api_type(Some("openai_compatible")), None);
        assert_eq!(normalize_ext_api_type(Some("")), None);
        assert_eq!(normalize_ext_api_type(None), None);
    }

    #[test]
    fn build_conn_falls_back_to_ext_provider_name() {
        let token = ExtToken {
            access_token: "at".to_string(),
            refresh_token: None,
            expires_at: None,
            email: None,
            label: None,
        };
        let conn = build_ext_conn("run.koma.ext.demo", "demo", &def("demo", "Demo", "browser"), token);
        assert_eq!(conn.name, "run.koma.ext.demo:demo"); // no label → id fallback
        assert_eq!(conn.refresh_token, "");
        assert_eq!(conn.expires_at, 0);
    }
}

// W13: additional regression suite — pure addition, sibling file, never touches the
// `ext_oauth_tests` module above.
#[cfg(test)]
#[path = "requests_oauth_test.rs"]
mod requests_oauth_test;

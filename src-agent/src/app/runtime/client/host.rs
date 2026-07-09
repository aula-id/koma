//! GUI host-relay layer — the GUI host IS the daemon client. Split out of
//! [`super`] (the `client` module) for file size — pure code motion, no
//! behaviour change.
//!
//! The desktop GUI (`crate::app::runtime::gui`) runs a `tao`/GTK event loop on
//! its main thread and CANNOT host tokio there (`event_loop.run` diverges).
//! So the daemon connection + the headless fold loop run HERE on a background
//! client-thread with its own tokio runtime — the daemon->JS direction pushes
//! JSON envelopes out through the `push` sink (an `EventLoopProxy::send_event`
//! closure the host supplies), and the JS->daemon direction arrives as
//! [`super::HostCtl`] control messages + a shared `live_req` the ipc thread
//! forwards `SubmitInput` through.
//!
//! `compute_file_diff`/`compute_usage_preview` (in the sibling [`super::diff`]
//! module) are called from `host_swapper` below — a cross-sibling call, which
//! is why they're bumped to `pub(super)` there. Everything in this file uses
//! fully-qualified `crate::app::runtime::manage::…` paths for the daemon
//! management calls (this file's `super` is `client`, not `runtime`, so the
//! old bare `super::manage::…` relative path from `mod.rs` doesn't resolve
//! here unchanged).

use crate::ipc::proto::ClientRequest;
use crate::model::store;

use super::connect::{connect_attach_and_handshake, Connection};
use super::diff::{compute_file_diff, compute_usage_preview};
use super::host_config::{apply_swapper_config_mutation, push_swapper_config};
use super::project::push_hub;
use super::push_proto::{
    push_file_diff, push_model_list, push_route_list, push_settings_values, push_switching,
    push_usage_preview,
};
use super::swapper::build_local_hub;
use super::{push_loop, render, HostCtl, StreamView};

/// The host-relay run-loop's next step, mirroring [`super::ClientState`] for the headless
/// GUI host: show the swapper, attach a session, or leave.
enum HostStep {
    /// Show the detached session swapper (the hub) and wait for a pick.
    Swapper,
    /// Attach to this session UUID and fold its frames into pushes. `workdir` is the
    /// folder a GUI `[+ new session]` native-picker chose (the new session's working
    /// dir); `None` for every other attach (existing pick, `--session` boot, daemon
    /// `/new` hand-off) inherits the host's cwd.
    Attach {
        id: String,
        workdir: Option<std::path::PathBuf>,
    },
    /// Leave the host-relay entirely (the window is gone).
    Done,
}

/// Headless twin of [`super::attach_session`]: attach + build-skew auto-restart WITHOUT a
/// terminal spinner (the GUI host owns no TTY). Ensures the daemon is up, connects +
/// handshakes, and on a CONFIRMED build mismatch restarts the stale daemon via the
/// SAME silent [`crate::app::runtime::manage::restart_daemon`] machinery (`quiet = true`) — at most
/// once — then reconnects. A daemon that sends no `Hello` is never restarted on that
/// absence alone (mirrors [`super::attach_session`]'s loop guard).
fn attach_session_headless(
    handle: &tokio::runtime::Handle,
    session_id: &str,
    workdir: Option<&std::path::Path>,
) -> anyhow::Result<Connection> {
    crate::app::runtime::manage::ensure_daemon_running(session_id, false, workdir).map_err(|e| {
        anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}")
    })?;

    let sock_path = store::daemon_sock_path(session_id)?;
    let my_fingerprint = store::build_fingerprint();

    let mut conn = connect_attach_and_handshake(handle, &sock_path)?;
    let mut already_restarted = false;
    while conn
        .daemon_version
        .as_deref()
        .is_some_and(|v| v != my_fingerprint)
    {
        if already_restarted {
            eprintln!(
                "koma: daemon still reports a different build after a restart; \
                 continuing against it"
            );
            break;
        }
        already_restarted = true;

        // Tear down the stale connection's bridge before restarting (drop the request
        // sender so the writer drains + exits; the reader observes the daemon's death
        // as EOF), then restart SILENTLY (no alt-screen spinner — there is no TTY).
        drop(conn.req_tx);
        drop(conn.frame_rx);
        crate::app::runtime::manage::restart_daemon(session_id, true)
            .map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}"))?;

        conn = connect_attach_and_handshake(handle, &sock_path)?;
    }
    Ok(conn)
}

/// Run the host-relay client on a background thread: own a tokio runtime and run the
/// two-state machine (swapper / attached) that PUSHES the shadow state into the
/// webview. The `push` sink hands a ready JSON envelope to the main tao thread;
/// `ctl_rx` carries [`HostCtl`] intents from the ipc handler; `ctl_tx` is a SELF-clone of
/// that channel's sender, handed to the off-thread session-lifecycle workers (kill / delete)
/// so they can route a follow-up [`HostCtl::RefreshHub`] back into whichever host state is
/// active once a daemon is confirmed dead / a session deleted; `live_req` is the shared slot
/// the ipc handler forwards `SubmitInput` through (updated on every (re)attach).
///
/// Holding `ctl_tx` for the relay's whole life means `ctl_rx` never observes `Disconnected`,
/// so the loop's control channel closes only at PROCESS exit — which is exactly when the GUI
/// tears down anyway (`tao`'s `event_loop.run` diverges into `process::exit` on window
/// close), so the relay thread is reaped there rather than via a channel-close signal.
///
/// Startup: `--session <id>` attaches straight to that session; otherwise the host
/// opens cold into the SWAPPER (the hub) so the user picks a live session, a history
/// session, or `[+ new session]`. A detach (socket close, or the daemon's `OpenSwapper`
/// hand-off) falls back to the swapper; a failed attach degrades to the swapper rather
/// than crashing.
///
/// W0 scope: the swapper RENDERS the hub and resolves `SelectSession` / `NewSession`
/// to an attach. The full in-hub key semantics (Ctrl+X nuke, history delete, cursor
/// nav) are W1 — the client-side keyboard swapper is not driven here.
pub(in crate::app::runtime) fn run_host_relay(
    opts: crate::cli::Opts,
    push: impl Fn(String) + Clone + Send + 'static,
    ctl_tx: std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: std::sync::mpsc::Receiver<HostCtl>,
    live_req: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: std::sync::Arc<std::sync::Mutex<StreamView>>,
) {
    // The client owns no sessions; it needs the config dirs only to resolve sockets.
    let _ = store::ensure_dirs();

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[gui] could not build the host-relay tokio runtime: {e}");
            return;
        }
    };
    let handle = rt.handle().clone();

    let mut push_state = push_loop::PushState::new();
    // The session the host is (or was) attached to, so the swapper flags the row it
    // came from as `is_foreground` and a `ToSwapper` fallback remembers it.
    let mut current_session_id: Option<String> = None;

    // Startup: attach directly to `--session`, else open cold into the swapper.
    let mut step = match opts.session.clone() {
        Some(id) => HostStep::Attach { id, workdir: None },
        None => HostStep::Swapper,
    };

    loop {
        step = match step {
            HostStep::Done => break,
            HostStep::Swapper => host_swapper(
                &handle,
                &push,
                &ctl_tx,
                &ctl_rx,
                &mut push_state,
                current_session_id.as_deref(),
            ),
            HostStep::Attach { id, workdir } => host_attached(
                &handle,
                &push,
                &ctl_tx,
                &ctl_rx,
                &live_req,
                &live_marks,
                &live_view,
                &mut push_state,
                &mut current_session_id,
                id,
                workdir,
            ),
        };
    }

    // Drop the runtime LAST so the active connection's reader task is cancelled after
    // the loop exits.
    drop(rt);
}

/// Spawn an OFF-THREAD escalating kill of the session-daemon `id`, then fire a follow-up
/// [`HostCtl::RefreshHub`] once the daemon is confirmed dead.
///
/// The escalating [`crate::app::runtime::manage::kill_session_daemon`] BLOCKS up to the grace budget (it
/// waits for death via `wait_until_dead` before each signal), so it must never run inline on
/// the host control loop (the swapper's `recv` or the attached 16ms fold). Running it on a
/// plain OS thread — then routing the refresh back through the SAME `ctl_tx` the ipc handler
/// uses — lets whichever host state is active (`host_swapper`'s `RefreshHub` re-push, or
/// `push_loop`'s off-thread sweep) rebuild the hub AFTER the row is genuinely gone, so a
/// just-killed daemon can never linger as a COOKING row.
pub(super) fn spawn_kill_and_refresh(ctl_tx: std::sync::mpsc::Sender<HostCtl>, id: String) {
    std::thread::spawn(move || {
        crate::app::runtime::manage::kill_session_daemon(&id); // blocks until dead (or the budget is spent)
        let _ = ctl_tx.send(HostCtl::RefreshHub);
    });
}

/// Spawn an OFF-THREAD escalating ensure-death of the session-daemon `id`, with NO follow-up
/// refresh. Used by the `New { kill: true }` switch: the OLD daemon is reaped WHILE the host
/// attaches a BRAND-NEW session, so the new attach must not wait on the old daemon's corpse
/// (hence off-thread) and there is no hub to refresh (we land in the new session, not the
/// hub). The graceful `QuitDaemon` the caller already queued on the live conn is flushed by
/// teardown; this guarantees the old daemon actually dies even if that graceful quit wedged.
#[allow(dead_code)]
pub(super) fn spawn_ensure_dead(id: String) {
    std::thread::spawn(move || {
        crate::app::runtime::manage::kill_session_daemon(&id);
    });
}

/// Spawn an OFF-THREAD history-only DELETE of session `id` (on-disk dir tree + registry row),
/// then fire a follow-up [`HostCtl::RefreshHub`].
///
/// The webview only ever sends a uuid; the path is resolved HOST-side from
/// [`store::list_all_sessions`]. Defense in depth: the delete is SKIPPED (leaving just the
/// refresh) when the uuid is currently LIVE ([`crate::app::runtime::manage::list_live_sessions`]) or its
/// on-disk lock is held (`meta.locked`) — a live session must never be deleted out from under
/// its daemon; `store::delete_session`'s sessions-root guard is the final backstop. Off-thread
/// because `list_live_sessions` connect-probes every socket (blocking).
pub(super) fn spawn_delete_and_refresh(ctl_tx: std::sync::mpsc::Sender<HostCtl>, id: String) {
    std::thread::spawn(move || {
        // Never delete a session that is currently live or whose on-disk lock is held.
        let live: std::collections::HashSet<String> = crate::app::runtime::manage::list_live_sessions()
            .into_iter()
            .map(|s| s.session_id)
            .collect();
        if !live.contains(&id) {
            if let Ok(metas) = store::list_all_sessions() {
                if let Some(meta) = metas.into_iter().find(|m| m.id == id && !m.locked) {
                    // Tighten the TOCTOU gap between the live/locked snapshot above and the
                    // physical remove: re-probe THIS session's daemon liveness (bind-as-oracle
                    // connect) immediately before deleting and skip if a daemon came up in the
                    // interim, so a session is never deleted out from under a live daemon.
                    // (`store::delete_session`'s sessions-root guard stays the final backstop.)
                    if !crate::app::runtime::manage::daemon_alive(&id) {
                        let _ = store::delete_session(&meta.path);
                    }
                }
            }
        }
        let _ = ctl_tx.send(HostCtl::RefreshHub);
    });
}


/// UN-ATTACHED GUI model-picker fetch (a [`HostCtl::ListModels`] serviced by the swapper):
/// load the GLOBAL config, resolve the provider by uuid, and `GET {endpoint}/models`,
/// returning the model ids. Returns an EMPTY list on an unknown provider OR any fetch error
/// — the caller ALWAYS pushes a reply, so the React picker's spinner clears. Mirrors the
/// daemon's attached-path `ClientRequest::ListModels` handler (`hub::requests`), but sources
/// the provider from disk since the swapper holds no in-memory `AppConfig`.
async fn fetch_models_for_provider(provider: &str) -> Vec<String> {
    let cfg = crate::model::app_config::AppConfig::load();
    let Some(p) = cfg.providers.iter().find(|p| p.uuid == provider) else {
        return Vec::new();
    };
    let c = crate::app::runtime::session_mgmt::build_client();
    let conn = crate::service::openrouter::Conn {
        endpoint: &p.endpoint,
        api_key: &p.api_key,
        api_type: crate::model::app_config::ApiType::OpenAiCompatible,
        account_id: "",
        oauth_uuid: "",
        install_id: "",
    };
    c.list_models(conn)
        .await
        .map(|v| v.into_iter().map(|m| m.id).collect::<Vec<_>>())
        .unwrap_or_default()
}

/// UN-ATTACHED GUI route-picker fetch (a [`HostCtl::ListRoutes`] serviced by the swapper):
/// load the GLOBAL config, resolve the provider by uuid, GATE on it being an OpenRouter-
/// style routable endpoint (the model-endpoints API is OpenRouter-specific — a non-OpenRouter
/// provider gets an immediate EMPTY list with no network call), then `GET
/// {endpoint}/models/{model_id}/endpoints`, flattening each route to the wire subset.
/// Returns EMPTY on an unknown/non-OpenRouter provider OR any fetch error (the caller always
/// pushes a reply → the form falls back to "Auto"). Mirrors the daemon's attached-path
/// `ClientRequest::ListRoutes` handler, including its OpenRouter gate.
async fn fetch_routes_for_provider(
    provider: &str,
    model_id: &str,
) -> Vec<crate::ipc::proto::ModelEndpointWire> {
    let cfg = crate::model::app_config::AppConfig::load();
    let Some(p) = cfg.providers.iter().find(|p| p.uuid == provider) else {
        return Vec::new();
    };
    // OpenRouter-only gate, mirroring the daemon path: the endpoints API is OpenRouter-
    // specific, so a non-OpenRouter provider yields an empty route list (form → "Auto").
    if !(p.api_type.is_routable() && p.endpoint.to_lowercase().contains("openrouter")) {
        return Vec::new();
    }
    let c = crate::app::runtime::session_mgmt::build_client();
    let conn = crate::service::openrouter::Conn {
        endpoint: &p.endpoint,
        api_key: &p.api_key,
        api_type: crate::model::app_config::ApiType::OpenAiCompatible,
        account_id: "",
        oauth_uuid: "",
        install_id: "",
    };
    c.list_model_endpoints(conn, model_id)
        .await
        .map(|eps| {
            eps.into_iter()
                .map(|ep| crate::ipc::proto::ModelEndpointWire {
                    name: ep.name,
                    provider_name: ep.provider_name,
                    price_prompt: ep.pricing.as_ref().and_then(|pr| pr.prompt.clone()),
                    price_completion: ep.pricing.as_ref().and_then(|pr| pr.completion.clone()),
                    uptime_last_30m: ep.uptime_last_30m,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// The SWAPPER arm: build the hub from cross-daemon discovery, push it, and block for
/// a control message. A `Ready` (page reload) re-discovers + re-pushes; a
/// `Select`/`New` resolves to an attach; a closed control channel (window gone) ends
/// the relay.
///
/// W0: no background live-refresh probe and no in-hub keyboard nav (those are W1) — the
/// hub is a static snapshot until the user picks. React animates any spinner locally.
fn host_swapper<P: Fn(String) + Clone + Send + 'static>(
    handle: &tokio::runtime::Handle,
    push: &P,
    ctl_tx: &std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    push_state: &mut push_loop::PushState,
    current: Option<&str>,
) -> HostStep {
    // Build + push the hub (discovery blocks briefly; fine — nothing renders here).
    let hub = build_local_hub(current);
    push_state.reset();
    push_hub(&hub, push, push_state);
    // The swapper holds no daemon snapshot, so the attached `push_loop`'s Config push
    // never runs here — the Connector/MCP panels would cold-open EMPTY. Read the loaded
    // global config directly and push a `Config` envelope so FIRST open shows the real
    // providers/models/mcp (bug #3/#4). `reset()` above cleared `config_json`, so this
    // (re)emits every swapper entry.
    push_swapper_config(push, push_state);

    loop {
        match ctl_rx.recv() {
            // Page reloaded (`Ready`) OR the ResumePalette opened (`RefreshHub`):
            // rediscover the live set + re-push the hub. In the swapper the blocking
            // discovery sweep is fine — nothing renders on this thread here.
            // (`ToSwapper` — a cancel that lands while already detached — is a harmless
            // hub re-push here: we are already showing the hub.)
            Ok(HostCtl::Ready) | Ok(HostCtl::RefreshHub) | Ok(HostCtl::ToSwapper) => {
                let hub = build_local_hub(current);
                push_state.reset();
                push_hub(&hub, push, push_state);
                // Re-emit config too (a `Ready` reload re-mounts the panels).
                push_swapper_config(push, push_state);
            }
            // Pre-session config mutation (onboarding theme/provider/model): apply it
            // directly to `~/.koma/config.json` and re-push `Config` so the panels + theme
            // repaint and `needsOnboarding` clears. Stay in the swapper (no attach).
            Ok(HostCtl::ConfigMutate(req)) => {
                apply_swapper_config_mutation(&req, push, push_state);
            }
            // UN-ATTACHED live model / route fetch (the GUI Connector picker during
            // onboarding / in the swapper, where there is no attached daemon to forward a
            // `ListModels`/`ListRoutes` to). Resolve the provider from the GLOBAL config and
            // run the network GET OFF this thread — a blocking HTTP call must never stall the
            // ctl loop — then push the SAME `ModelList`/`RouteList` envelope the attached
            // daemon path emits. The spawned worker ALWAYS pushes a reply (an EMPTY list on an
            // unknown provider or any fetch error), so the React picker's spinner can never
            // hang. A clone of the (Clone) push sink rides into the task so it can reach the
            // webview after this arm hands control back to the recv loop below.
            Ok(HostCtl::ListModels { provider }) => {
                let push2 = P::clone(push);
                handle.spawn(async move {
                    let models = fetch_models_for_provider(&provider).await;
                    push_model_list(&push2, provider, models);
                });
            }
            Ok(HostCtl::ListRoutes { provider, model_id }) => {
                let push2 = P::clone(push);
                handle.spawn(async move {
                    let routes = fetch_routes_for_provider(&provider, &model_id).await;
                    push_route_list(&push2, provider, model_id, routes);
                });
            }
            // Explore FILE CHANGED panel: host-side diff fetch (git + fs are blocking,
            // so this runs on a plain OS thread rather than the async runtime). Never
            // touches the daemon in either host state — see `compute_file_diff`.
            Ok(HostCtl::FileDiff { path }) => {
                let push2 = P::clone(push);
                let cur = current.map(str::to_string);
                std::thread::spawn(move || {
                    let result = compute_file_diff(&path, cur.as_deref());
                    push_file_diff(&push2, result);
                });
            }
            // GUI Usage panel opened while detached (StartScreen / swapper): the ledger is
            // a global file the host reads directly, so this never touches a daemon in
            // either state — see `compute_usage_preview`. Sqlite I/O is blocking, so it
            // runs on a plain OS thread like `FileDiff` above. `scope` AND `session` both
            // ride along unchanged so the reply echoes them (the React panel drops a
            // reply whose scope OR session id no longer matches what's currently
            // selected/attached — a stale cross-session reply must never render). A
            // "session" scope with no session attached (there is none — this is the
            // swapper) simply queries with `session: None` passed through by the ipc
            // handler, which only sets `Some(uuid)` when a session IS attached.
            Ok(HostCtl::UsagePreview { session, scope }) => {
                let push2 = P::clone(push);
                std::thread::spawn(move || {
                    let result = compute_usage_preview(session.as_deref());
                    push_usage_preview(&push2, result, scope, session);
                });
            }
            // GUI Settings tab opened while detached (StartScreen / swapper): there is no
            // foreground session, so answer from the GLOBAL config — the active palette +
            // `Settings` DEFAULTS (empty name/workdir). ALWAYS a reply so the tab's loading
            // state clears. Cheap, synchronous (a config load), so it runs inline.
            Ok(HostCtl::GetSettings) => {
                let cfg = crate::model::app_config::AppConfig::load();
                let d = crate::model::settings::Settings::default();
                push_settings_values(
                    push,
                    String::new(),
                    Vec::new(),
                    d.short_send_enabled,
                    d.sliding_cache,
                    d.bash_saving,
                    d.internet_mode.as_str().to_string(),
                    cfg.palette,
                    String::new(),
                );
            }
            // A hub row's KILL button. In the swapper there is no ATTACHED session, so this
            // is always a background/live-row kill: escalate the kill OFF this thread (it
            // blocks up to the grace budget) and let the follow-up `RefreshHub` rebuild the
            // hub once the daemon is confirmed dead — the killed row can't linger in COOKING.
            Ok(HostCtl::KillSession(id)) => {
                spawn_kill_and_refresh(ctl_tx.clone(), id);
            }
            // A hub HISTORY row's DELETE button: physically remove that session OFF this
            // thread (it connect-probes every live socket for the live/locked guard), then
            // `RefreshHub`. The delete is refused host-side for a live/locked session.
            Ok(HostCtl::DeleteSession(id)) => {
                spawn_delete_and_refresh(ctl_tx.clone(), id);
            }
            // A hub pick → attach that session; `[+ new session]` → mint + attach. Fire the
            // swap-START loader signal first (this thread will BLOCK in the attach next, so
            // this push is the last thing the webview hears until the new Snapshot lands).
            Ok(HostCtl::Select(id)) => {
                push_switching(push, &id);
                return HostStep::Attach { id, workdir: None };
            }
            // `[+ new session]`: the GUI picker already ran (this only fires after a folder
            // was confirmed — a cancel sends nothing), so mint a fresh session and attach
            // it AT the chosen `workdir`. `None` (empty-state / non-GUI) keeps the host cwd.
            // `kill` is only meaningful from the ATTACHED chat view (there is no attached
            // session to reap in the swapper), so it is ignored here — a start-screen new is
            // always a plain add.
            Ok(HostCtl::New { workdir, kill: _ }) => {
                let new_id = uuid::Uuid::new_v4().to_string();
                push_switching(push, &new_id);
                return HostStep::Attach { id: new_id, workdir };
            }
            // The ipc side hung up (window gone) — leave the host.
            Err(_) => return HostStep::Done,
        }
    }
}

/// The ATTACHED arm: attach `id` (build-skew safe), publish its request sender for the
/// ipc `Submit`, fold its frames into pushes via [`push_loop::push_loop`], then tear the
/// connection down and translate the loop's [`push_loop::HostTransition`] into the next
/// [`HostStep`]. A failed attach degrades to the swapper rather than crashing.
#[allow(clippy::too_many_arguments)]
fn host_attached(
    handle: &tokio::runtime::Handle,
    push: &dyn Fn(String),
    ctl_tx: &std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    live_req: &std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: &std::sync::Arc<std::sync::Mutex<StreamView>>,
    push_state: &mut push_loop::PushState,
    current: &mut Option<String>,
    id: String,
    workdir: Option<std::path::PathBuf>,
) -> HostStep {
    let mut conn = match attach_session_headless(handle, &id, workdir.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[gui] host-relay could not attach session {id}: {e:#}");
            // Degrade to the swapper (fresh discovery) — the user can pick again.
            return HostStep::Swapper;
        }
    };
    *current = Some(id);

    // Publish this connection's request sender so the ipc handler's `Submit` lands on
    // the CURRENT daemon; take the handshake's prebuffered frames for the fold.
    if let Ok(mut g) = live_req.lock() {
        *g = Some(conn.req_tx.clone());
    }
    let prebuffered = std::mem::take(&mut conn.prebuffered);
    push_state.reset();

    // Enter the runtime context ONLY for the fold loop (a reconstructed shadow
    // sub-agent mints an inert AbortHandle, which needs a runtime in scope) — SCOPED
    // so the guard drops before `teardown_connection`'s `block_on`.
    let transition = {
        let _rt_ctx = handle.enter();
        push_loop::push_loop(
            push,
            &conn.frame_rx,
            &conn.req_tx,
            prebuffered,
            ctl_tx,
            ctl_rx,
            push_state,
            current.as_deref(),
            live_marks,
            live_view,
        )
    };

    // Retract the live sender + clear the staged-marker mirror before teardown so a
    // late `Submit` can't race a half-torn-down connection or append stale markers,
    // then flush the polite `Detach`.
    if let Ok(mut g) = live_req.lock() {
        *g = None;
    }
    if let Ok(mut m) = live_marks.lock() {
        m.clear();
    }
    // Reset the stream view so the NEXT attach starts with no stream tab open (the new
    // daemon's fresh HubClient starts with none too) — a stale sub-agent/bash id from the
    // session we're leaving must never bleed into the next session's fold.
    if let Ok(mut v) = live_view.lock() {
        *v = StreamView::default();
    }
    super::teardown_connection(handle, conn);

    match transition {
        // Carry any GUI-picker workdir (a hub `New` while attached) into the next attach;
        // a daemon `/new` hand-off / a `Select` carries `None` (inherit the host cwd).
        push_loop::HostTransition::Attach { id, workdir } => HostStep::Attach { id, workdir },
        push_loop::HostTransition::ToSwapper => HostStep::Swapper,
        push_loop::HostTransition::Exit => HostStep::Done,
    }
}

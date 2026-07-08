//! Thin attach client — the `koma --attach` core (daemon stage 6).
//!
//! [`client_run`] connects to a running daemon's unix socket, attaches, and then
//! renders the daemon's state + forwards input. It does NONE of the real work:
//! no `service_all_sessions`, no turn machinery, no agent runtime. It maintains a
//! SHADOW [`AppState`] populated PURELY from the daemon's
//! [`DaemonEvent::Snapshot`] / [`DaemonEvent::Delta`] frames and feeds that shadow
//! to the EXISTING [`crate::view::draw`] — so the attach client renders identically
//! to a local TUI, with zero second render path to drift.
//!
//! ## Module layout
//!
//! | Submodule   | Contents                                                         |
//! |-------------|------------------------------------------------------------------|
//! | `connect`   | `Connection` struct + `connect_attach_and_handshake` (sync)     |
//! | `render`    | `render_loop`, `advance_local_animations`, frame-pacing consts  |
//! | `shadow`    | `apply_frame`, `apply_snapshot`, `apply_delta`, seq-gap, clock  |
//! | `input`     | `local_echo`, `QuitConfirmKey`, quit overlay keys               |
//! | `bridge`    | `reader_task`, `writer_task`, transport consts                  |

#![allow(unused_imports)]
#![allow(dead_code)]

mod connect;
mod render;
mod shadow;
mod input;
mod bridge;
mod swapper;

use std::io::{stdout, Stdout};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{Mode, SessionHub};
use crate::ipc::proto::ClientRequest;
use crate::model::store;

use connect::{connect_attach_and_handshake, Connection};
use bridge::WRITER_FLUSH_TIMEOUT;
use swapper::{build_local_hub, run_swapper, SwapperOutcome};

use crate::app::runtime::terminal::TerminalGuard;

/// The client's run-loop state: either ATTACHED to a session-daemon (rendering its frames
/// and forwarding input) or running the local detached SWAPPER (the `/resume` picker).
///
/// A swap is "detach-then-pick": entering the swapper from an attached session first tears
/// the connection down (leaving that daemon cooking) so the daemon's snapshots can't
/// clobber the local hub, then runs the swapper STANDALONE; a pick attaches to the chosen
/// daemon, a cancel reconnects to the one just left.
enum ClientState {
    /// Live attached to a session-daemon.
    Attached(Connection),
    /// Detached, showing the local cross-daemon swapper.
    Swapper(SessionHub),
}

/// Restart the stale session-daemon while showing an animated "reopening" spinner.
/// The restart (`manage::restart_daemon`) is blocking (~1s), so it runs on a
/// background thread while THIS (main) thread — which owns the terminal — draws the
/// spinner each frame until the restart completes, then propagates its result.
/// Kept fully silent (quiet=true) so nothing bleeds into the alt-screen.
fn restart_daemon_animated(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    session_id: &str,
) -> Result<()> {
    let sid = session_id.to_string();
    let worker = std::thread::spawn(move || super::manage::restart_daemon(&sid, true));

    // Load the actual user config (dark/light, accent) so the spinner matches the
    // user's real theme rather than always using defaults.
    let cfg = crate::model::app_config::AppConfig::load();
    let palette = crate::view::theme::palette(&cfg);

    let mut frame: u64 = 0;
    // ~80ms per braille glyph → a calm spin; check completion each frame.
    const FRAME: Duration = Duration::from_millis(80);
    while !worker.is_finished() {
        let start = Instant::now();
        let _ = terminal.draw(|f| crate::view::loading::draw_reopening(f, frame, &palette));
        frame = frame.wrapping_add(1);
        if let Some(rem) = FRAME.checked_sub(start.elapsed()) {
            std::thread::sleep(rem);
        }
    }

    // Propagate the restart's result; a panicked worker thread → generic error.
    match worker.join() {
        Ok(res) => res.map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}")),
        Err(_) => Err(anyhow::anyhow!("reopening thread panicked during daemon restart")),
    }
}

/// Attach to a session-daemon, spawning it if needed, and run the build-skew handshake.
///
/// The single attach primitive used everywhere the client connects: the initial
/// non-resume attach, a swapper PICK, and a swapper CANCEL-reconnect. It:
/// 1. ensures the session's daemon is RUNNING ([`super::manage::ensure_daemon_running`],
///    `resume=false`): for a LIVE session this is a no-op (the daemon is already up); for
///    a brand-new minted id it spawns a fresh `--daemon --session <id>` (create branch);
///    for an on-disk history id it spawns one that create-or-LOADs that session;
/// 2. connects + attaches + runs the `Hello` handshake ([`connect_attach_and_handshake`]);
/// 3. on a CONFIRMED build-skew mismatch, restarts that one stale daemon (AT MOST ONCE)
///    via the SAME machinery `koma daemon restart` uses and reconnects.
///
/// # Build-skew auto-restart (task #142)
///
/// The koma daemon outlives a rebuild, so a freshly-built client can attach to a daemon
/// still running OLD code and silently render its stale frames (this caused a phantom
/// `/agents` bug). The client compares its OWN fresh fingerprint
/// ([`store::build_fingerprint`]) against the daemon's reported `Hello`; on a mismatch it
/// restarts the stale daemon and reconnects. LOOP GUARD: the restart fires at most once —
/// if the just-spawned daemon STILL mismatches (it shouldn't, it was built from the current
/// binary) it warns and renders against it rather than restart-looping. A daemon that sends
/// no `Hello` (slow / pre-handshake) is never restarted on that absence alone.
fn attach_session(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    handle: &tokio::runtime::Handle,
    session_id: &str,
) -> Result<Connection> {
    // Make sure a daemon owns this session before we connect. No-op when it is already
    // live (the bind-as-oracle probe inside short-circuits); spawns + waits otherwise.
    super::manage::ensure_daemon_running(session_id, false, None)
        .map_err(|e| anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}"))?;

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

        // Tear down the stale connection's bridge before restarting: drop our request
        // sender (the writer drains + exits) and let the reader observe the daemon's
        // death as EOF. Both old tasks self-terminate; the runtime persists.
        drop(conn.req_tx);
        drop(conn.frame_rx);

        restart_daemon_animated(terminal, session_id)?;

        conn = connect_attach_and_handshake(handle, &sock_path)?;
    }
    Ok(conn)
}

/// Tear a live [`Connection`] down cleanly: queue a polite `Detach`, then drop the request
/// sender and JOIN the writer so the final frame(s) flush before the runtime is touched.
///
/// Run on EVERY exit from an [`ClientState::Attached`] — a plain exit, a render error, AND
/// the detach-then-swap into the swapper — so the source daemon is never left orphaned
/// (socket open, no controller) and never stuck mid-hub. The `/quit` overlay's `[k]`/`[d]`
/// paths may have queued their own `Detach` (and a `QuitDaemon` for `[k]`, which kills the
/// daemon outright) already; this extra `Detach` is then a harmless no-op (the daemon
/// deregistered this client by id, so a second one matches nobody). Dropping `req_tx` closes the outbound channel, which the
/// writer observes as `Disconnected`: it drains EVERY remaining queued request to the
/// socket and returns. We JOIN it (bounded by [`WRITER_FLUSH_TIMEOUT`], so a wedged socket
/// can't hang exit) BEFORE returning, which is what guarantees the shutdown frames land.
/// MUST NOT be called while a tokio runtime context is entered (it `block_on`s).
fn teardown_connection(handle: &tokio::runtime::Handle, conn: Connection) {
    let Connection {
        frame_rx: _,
        req_tx,
        writer_handle,
        prebuffered: _,
        daemon_version: _,
    } = conn;

    let _ = req_tx.send(ClientRequest::Detach);
    drop(req_tx);
    let _ = handle.block_on(async {
        tokio::time::timeout(WRITER_FLUSH_TIMEOUT, writer_handle).await
    });
}

// ─── host-relay: the GUI host IS the daemon client ───────────────────────────────
//
// The desktop GUI (`crate::app::runtime::gui`) runs a `tao`/GTK event loop on its
// main thread and CANNOT host tokio there (`event_loop.run` diverges). So the daemon
// connection + the headless fold loop run HERE on a background client-thread with its
// own tokio runtime — the daemon->JS direction pushes JSON envelopes out through the
// `push` sink (an `EventLoopProxy::send_event` closure the host supplies), and the
// JS->daemon direction arrives as [`HostCtl`] control messages + a shared `live_req`
// the ipc thread forwards `SubmitInput` through.

/// Control messages from the GUI ipc thread (main tao thread) to the host-relay
/// client-thread. `SubmitInput` does NOT ride this channel — it goes straight to the
/// live daemon via the shared `live_req` sender — so this carries only the
/// session-lifecycle intents the client-thread owns.
pub(super) enum HostCtl {
    /// The webview page booted / reloaded: re-push the full authoritative state.
    Ready,
    /// Attach to this existing session UUID (a hub `SelectSession` pick).
    Select(String),
    /// Mint a fresh session UUID + attach (the hub `[+ new session]` row). Carries the
    /// folder the GUI's native picker chose as the new session's working dir; `None`
    /// (e.g. a non-GUI/empty-state new) falls back to the host's cwd (base path).
    New(Option<std::path::PathBuf>),
    /// Re-run cross-daemon discovery + push a FRESH `Hub` envelope. Fired when the
    /// React ResumePalette overlay opens (and may re-fire while it stays open).
    /// Handled in BOTH host states: inline in `host_swapper` (nothing renders there,
    /// so the blocking sweep is fine), and OFF the fold thread in `render::push_loop`
    /// while attached (the sweep must not stall the 16ms loop). This keeps the live
    /// session list current instead of frozen at the one cold build-at-boot.
    RefreshHub,
    /// Best-effort CANCEL of a session switch (the React loader's Cancel button): bail to
    /// the hub. An in-flight attach can't be interrupted (the client-thread is blocked in
    /// `attach_session_headless` and never polls this channel then), so this is queued and
    /// acted on once the current/target attach lands — `push_loop` returns
    /// `HostTransition::ToSwapper` and `host_swapper` pushes a fresh `Hub` (clearing the
    /// loader). In the swapper it is a harmless hub re-push.
    ToSwapper,
    /// Apply a config-GLOBAL mutation directly to `~/.koma/config.json` while PRE-SESSION.
    /// The empty-state/onboarding flow runs in the SWAPPER, which holds NO attached daemon
    /// to forward a `ClientRequest` to (the ipc `live_req` slot is `None`), so the theme +
    /// provider + model setters that onboarding drives arrive here instead. Carries the
    /// SAME [`ClientRequest`] the attached path forwards; `host_swapper` loads the config,
    /// applies the config-global subset (provider/model/mcp/theme), saves, and re-pushes a
    /// fresh `Config` (repainting the theme + clearing `needsOnboarding`). Session-scoped
    /// requests (a `SetModel { scope:"local" }`, `SetSessionMain`) are no-ops here — there
    /// is no session to hold a local override yet.
    ConfigMutate(ClientRequest),
    /// UN-ATTACHED live model-id fetch (the GUI Connector ModelForm's model-id picker while
    /// onboarding / in the swapper, where the ipc `live_req` daemon path is `None`). The
    /// swapper resolves `provider` (uuid) from the GLOBAL config, runs the `GET
    /// {endpoint}/models` OFF-thread (a network call must never block the ctl loop), and
    /// pushes the SAME `ModelList` envelope the attached daemon path produces — ALWAYS a
    /// reply (an EMPTY list on an unknown provider or any fetch error), so the React picker's
    /// spinner can never hang.
    ListModels { provider: String },
    /// UN-ATTACHED twin of [`ListModels`](Self::ListModels) for the ROUTE picker: fetch one
    /// model's live provider-route list (`GET {endpoint}/models/{model_id}/endpoints`)
    /// off-thread and push a `RouteList` envelope (echoing `provider` + `model_id`). EMPTY
    /// routes for a non-OpenRouter provider (the endpoints API is OpenRouter-specific) or any
    /// fetch error — again ALWAYS a reply so the form falls back to "Auto" instead of hanging.
    ListRoutes { provider: String, model_id: String },
}

/// The host-relay run-loop's next step, mirroring [`ClientState`] for the headless
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

/// Headless twin of [`attach_session`]: attach + build-skew auto-restart WITHOUT a
/// terminal spinner (the GUI host owns no TTY). Ensures the daemon is up, connects +
/// handshakes, and on a CONFIRMED build mismatch restarts the stale daemon via the
/// SAME silent [`super::manage::restart_daemon`] machinery (`quiet = true`) — at most
/// once — then reconnects. A daemon that sends no `Hello` is never restarted on that
/// absence alone (mirrors [`attach_session`]'s loop guard).
fn attach_session_headless(
    handle: &tokio::runtime::Handle,
    session_id: &str,
    workdir: Option<&std::path::Path>,
) -> Result<Connection> {
    super::manage::ensure_daemon_running(session_id, false, workdir).map_err(|e| {
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
        super::manage::restart_daemon(session_id, true)
            .map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}"))?;

        conn = connect_attach_and_handshake(handle, &sock_path)?;
    }
    Ok(conn)
}

/// Run the host-relay client on a background thread: own a tokio runtime and run the
/// two-state machine (swapper / attached) that PUSHES the shadow state into the
/// webview. The `push` sink hands a ready JSON envelope to the main tao thread;
/// `ctl_rx` carries [`HostCtl`] intents from the ipc handler; `live_req` is the shared
/// slot the ipc handler forwards `SubmitInput` through (updated on every (re)attach).
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
pub(super) fn run_host_relay(
    opts: crate::cli::Opts,
    push: impl Fn(String) + Clone + Send + 'static,
    ctl_rx: std::sync::mpsc::Receiver<HostCtl>,
    live_req: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
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

    let mut push_state = render::PushState::new();
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
                &ctl_rx,
                &mut push_state,
                current_session_id.as_deref(),
            ),
            HostStep::Attach { id, workdir } => host_attached(
                &handle,
                &push,
                &ctl_rx,
                &live_req,
                &live_marks,
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

/// Read the loaded GLOBAL config off disk and push a `Config` envelope so the GUI's
/// Connector + MCP panels show the real providers/models/mcp while the host is in the
/// SWAPPER (bug #3/#4). The swapper holds no daemon connection/snapshot, so the attached
/// `push_loop`'s snapshot-sourced Config push never runs there; without this the panels
/// cold-open EMPTY even though `~/.koma/config.json` has providers/models. `push_config`
/// dedups on `push_state.config_json`, so callers `reset()` first to force a re-emit.
fn push_swapper_config(push: &dyn Fn(String), push_state: &mut render::PushState) {
    let cfg = crate::model::app_config::AppConfig::load();
    let projection = render::ConfigProjection::from_app_config(&cfg);
    render::push_config(Some(&projection), push, push_state);
}

/// Apply a PRE-SESSION config mutation (a [`HostCtl::ConfigMutate`]) directly to
/// `~/.koma/config.json` and re-push a fresh `Config` envelope.
///
/// The swapper/onboarding state has no attached daemon, so the theme + provider + model
/// setters onboarding drives can't ride the normal `live_req` → daemon path. Instead the
/// host loads the on-disk config, applies the config-GLOBAL subset via [`apply_global_config_req`],
/// persists it, and re-pushes so the Connector panels + the live theme repaint and the
/// `needsOnboarding` flag clears the instant a provider + Main model land. `push_config`
/// dedups on the last-pushed JSON, so an unchanged config emits nothing.
fn apply_swapper_config_mutation(
    req: &ClientRequest,
    push: &dyn Fn(String),
    push_state: &mut render::PushState,
) {
    let mut cfg = crate::model::app_config::AppConfig::load();
    if apply_global_config_req(&mut cfg, req) {
        if let Err(e) = cfg.save() {
            eprintln!("[gui] pre-session config save failed: {e}");
        }
        let projection = render::ConfigProjection::from_app_config(&cfg);
        render::push_config(Some(&projection), push, push_state);
    }
}

/// Apply the config-GLOBAL subset of a [`ClientRequest`] to an in-memory [`AppConfig`],
/// returning `true` if it mutated `cfg` (the caller then persists + re-pushes).
///
/// This mirrors — for the PRE-SESSION swapper path — exactly what the daemon's
/// `dispatch_request` does for these variants (see
/// `runtime::event_loop::daemon::hub::requests`), reusing the SAME config-layer setters
/// (`upsert_provider`/`upsert_model`/`upsert_mcp_server`/…) and the SAME MCP arg/env
/// parsers, so the on-disk result is identical whether a setter runs attached or during
/// onboarding. Session-scoped operations (`SetModel { scope:"local" }`, `SetSessionMain`,
/// the MCP live-reconnect) have no session/manager pre-session and are treated as no-ops
/// / config-write-only here. Any non-config request returns `false` untouched.
fn apply_global_config_req(
    cfg: &mut crate::model::app_config::AppConfig,
    req: &ClientRequest,
) -> bool {
    use crate::model::app_config::{McpServerEntry, McpTransport, ModelEntry, ModelRole};
    match req {
        ClientRequest::SetTheme { name } => {
            cfg.palette = name.clone();
            true
        }
        ClientRequest::SetProvider {
            uuid,
            name,
            endpoint,
            api_key,
        } => {
            cfg.upsert_provider(
                uuid.clone(),
                name.trim().to_string(),
                endpoint.trim().to_string(),
                api_key.clone(),
            );
            true
        }
        ClientRequest::DeleteProvider { uuid } => {
            cfg.remove_provider_by_uuid(uuid);
            true
        }
        ClientRequest::SetModel {
            uuid,
            name,
            model_id,
            provider_uuid,
            route,
            roles,
            scope,
        } => {
            // Pre-session there is no foreground session to hold a LOCAL override, so a
            // `local`-scope model can't be applied here — only the GLOBAL catalogue.
            if scope == "local" {
                return false;
            }
            let roles: Vec<ModelRole> = roles
                .iter()
                .filter_map(|r| match r.as_str() {
                    "main" => Some(ModelRole::Main),
                    "awareness" => Some(ModelRole::Awareness),
                    "safeguard" => Some(ModelRole::Safeguard),
                    "compactor" => Some(ModelRole::Compactor),
                    "planner" => Some(ModelRole::Planner),
                    _ => None,
                })
                .collect();
            cfg.upsert_model(ModelEntry {
                uuid: uuid.clone().unwrap_or_default(),
                name: name.trim().to_string(),
                model_id: model_id.trim().to_string(),
                provider_uuid: provider_uuid.clone(),
                route: ModelEntry::normalize_route(route.clone()),
                roles,
                role: None,
            });
            true
        }
        ClientRequest::DeleteModel { uuid, scope } => {
            if scope == "local" {
                return false;
            }
            cfg.remove_model_by_uuid(uuid);
            true
        }
        ClientRequest::SetMcpServer {
            uuid,
            name,
            enabled,
            transport,
            command,
            args,
            env,
            url,
        } => {
            cfg.upsert_mcp_server(McpServerEntry {
                uuid: uuid.clone().unwrap_or_default(),
                name: name.trim().to_string(),
                enabled: *enabled,
                transport: if transport == "http" {
                    McpTransport::Http
                } else {
                    McpTransport::Stdio
                },
                command: command.trim().to_string(),
                args: crate::app::mode::mcp::parse_args(args),
                env: crate::app::mode::mcp::parse_env(env),
                url: url.trim().to_string(),
            });
            true
        }
        ClientRequest::DeleteMcpServer { uuid } => {
            cfg.remove_mcp_server_by_uuid(uuid);
            true
        }
        ClientRequest::EnableMcpServer { uuid, enabled } => {
            cfg.set_mcp_enabled_by_uuid(uuid, *enabled);
            true
        }
        // GUI onboarding "koma free" (pre-session): mint/reuse the keyless Koma Free
        // provider + Main model directly in the on-disk config — the SAME
        // `ensure_koma_free_config` the daemon + TUI paths use, so the entries are
        // identical whether this runs attached or during onboarding. Returning `true`
        // makes the caller persist + re-push `Config`, which clears `firstRun`.
        ClientRequest::SetupKomaFree => {
            crate::service::koma_free::ensure_koma_free_config(cfg);
            true
        }
        _ => false,
    }
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
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    push_state: &mut render::PushState,
    current: Option<&str>,
) -> HostStep {
    // Build + push the hub (discovery blocks briefly; fine — nothing renders here).
    let hub = build_local_hub(current);
    push_state.reset();
    render::push_hub(&hub, push, push_state);
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
                render::push_hub(&hub, push, push_state);
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
                    render::push_model_list(&push2, provider, models);
                });
            }
            Ok(HostCtl::ListRoutes { provider, model_id }) => {
                let push2 = P::clone(push);
                handle.spawn(async move {
                    let routes = fetch_routes_for_provider(&provider, &model_id).await;
                    render::push_route_list(&push2, provider, model_id, routes);
                });
            }
            // A hub pick → attach that session; `[+ new session]` → mint + attach. Fire the
            // swap-START loader signal first (this thread will BLOCK in the attach next, so
            // this push is the last thing the webview hears until the new Snapshot lands).
            Ok(HostCtl::Select(id)) => {
                render::push_switching(push, &id);
                return HostStep::Attach { id, workdir: None };
            }
            // `[+ new session]`: the GUI picker already ran (this only fires after a folder
            // was confirmed — a cancel sends nothing), so mint a fresh session and attach
            // it AT the chosen `workdir`. `None` (empty-state / non-GUI) keeps the host cwd.
            Ok(HostCtl::New(workdir)) => {
                let new_id = uuid::Uuid::new_v4().to_string();
                render::push_switching(push, &new_id);
                return HostStep::Attach { id: new_id, workdir };
            }
            // The ipc side hung up (window gone) — leave the host.
            Err(_) => return HostStep::Done,
        }
    }
}

/// The ATTACHED arm: attach `id` (build-skew safe), publish its request sender for the
/// ipc `Submit`, fold its frames into pushes via [`render::push_loop`], then tear the
/// connection down and translate the loop's [`render::HostTransition`] into the next
/// [`HostStep`]. A failed attach degrades to the swapper rather than crashing.
#[allow(clippy::too_many_arguments)]
fn host_attached(
    handle: &tokio::runtime::Handle,
    push: &dyn Fn(String),
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    live_req: &std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    push_state: &mut render::PushState,
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
        render::push_loop(
            push,
            &conn.frame_rx,
            &conn.req_tx,
            prebuffered,
            ctl_rx,
            push_state,
            current.as_deref(),
            live_marks,
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
    teardown_connection(handle, conn);

    match transition {
        // Carry any GUI-picker workdir (a hub `New` while attached) into the next attach;
        // a daemon `/new` hand-off / a `Select` carries `None` (inherit the host cwd).
        render::HostTransition::Attach { id, workdir } => HostStep::Attach { id, workdir },
        render::HostTransition::ToSwapper => HostStep::Swapper,
        render::HostTransition::Exit => HostStep::Done,
    }
}

/// Run the thin attach client, with the daemon-per-session SWAPPER.
///
/// A two-state run-loop ([`ClientState`]): ATTACHED (render a daemon's frames + forward
/// input) or SWAPPER (the detached `/resume` picker). Startup depends on `opts.resume`:
///
/// - **`--resume` / `koma agents`** (`opts.resume`): start in the SWAPPER with NO daemon
///   connection and nothing to return to. The user picks a session (live, history, or a
///   fresh `[+ new session]`) and the client attaches to it; cancelling with nothing to
///   return to exits cleanly. (`opts.session` is ignored on this path — main does not mint
///   one for `--resume`.)
/// - **plain `koma` / `--session X`** (no resume): attach immediately to `opts.session`
///   (the uuid main minted + spawned a daemon for), exactly as before.
///
/// Swap is detach-then-pick: a `/resume` IN-session makes the daemon send `OpenSwapper`,
/// the render loop returns [`render::ClientTransition::OpenSwapper`], and here we
/// [`teardown_connection`] (leaving that daemon cooking) before running the swapper
/// STANDALONE — so its snapshots can't clobber the local hub. A PICK attaches the chosen
/// daemon; a CANCEL reconnects the one we left. `rt`/`handle`, the terminal, and the
/// [`TerminalGuard`] are owned across the whole loop, so a swap reuses the same runtime +
/// alt-screen with no re-enter flash. A failed attach degrades to the swapper (rebuilt
/// from fresh discovery) rather than crashing.
pub fn client_run(opts: crate::cli::Opts) -> Result<()> {
    // The client needs the config dirs only to resolve socket paths; it owns no sessions
    // and writes no config (lock ownership belongs to the daemon — see the module header).
    store::ensure_dirs()?;

    // A small multi-thread runtime drives the two socket tasks of whatever connection is
    // live. The render/swapper loops run on THIS thread (synchronous), like the local TUI.
    // Owned across the WHOLE state machine so every attach/reattach reuses it.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // Terminal setup — identical to the local TUI (`run`). Guard first so a failure
    // anywhere after still restores the terminal. The guard persists ACROSS the loop so a
    // detach-then-swap re-attaches without re-entering the alt-screen.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // `current_session_id` is the session we are (or are becoming) attached to;
    // `prev_session` is what a swapper CANCEL returns to. On `--resume` both start empty
    // (the swapper opens cold); otherwise `current` is the minted/`--session` id.
    let mut current_session_id: Option<String> = None;
    let mut prev_session: Option<String> = None;

    // Seed the initial state. Build-skew handling + daemon-spawn live in `attach_session`,
    // so an `Err` here (no daemon could be started, or the initial connect failed) is
    // surfaced to the caller exactly as before — BUT only on the non-resume path, where an
    // attach happens up front. The `--resume` path can't fail here (no attach yet).
    let mut state = if opts.resume {
        // `--resume` / `koma agents`: swapper first, no connection, nothing to return to.
        ClientState::Swapper(build_local_hub(None))
    } else {
        // Plain `koma` / `--session X`: attach immediately to the minted/given id (REQUIRED
        // here — without it there is no socket to reach).
        let id = opts.session.clone().ok_or_else(|| {
            anyhow::anyhow!("internal: client_run requires a session id (--session <id>) without --resume")
        })?;
        let conn = attach_session(&mut terminal, &handle, &id)?;
        current_session_id = Some(id);
        ClientState::Attached(conn)
    };

    // The loop's terminal outcome (Ok on a clean exit, or a render error captured so the
    // active connection's teardown still runs before it is returned).
    let mut render_result: Result<()> = Ok(());

    loop {
        state = match state {
            // --- ATTACHED: render the daemon's frames + forward input ---
            ClientState::Attached(mut conn) => {
                // Take the handshake's prebuffered frames OUT for this render pass (the
                // loop consumes them); leaves an empty Vec so `conn` stays intact for the
                // teardown / detach below.
                let prebuffered = std::mem::take(&mut conn.prebuffered);

                // Run the render loop with the runtime context entered on THIS thread,
                // SCOPED so the `EnterGuard` drops the instant it returns — BEFORE any
                // `teardown_connection`'s `block_on` (which panics under an entered
                // context). The context is needed only so a snapshot rebuild can mint the
                // inert `AbortHandle` a reconstructed shadow `SubAgent` carries.
                let transition = {
                    let _rt_ctx = handle.enter();
                    render::render_loop(
                        &mut terminal,
                        &conn.frame_rx,
                        &conn.req_tx,
                        prebuffered,
                    )
                };

                match transition {
                    // Leave the client: tear this connection down (flush the Detach) and
                    // break out of the loop. No connection survives, so the post-loop has
                    // nothing more to detach.
                    Ok(render::ClientTransition::Exit) => {
                        teardown_connection(&handle, conn);
                        break;
                    }
                    // `/resume`: DETACH from this daemon (leaving it cooking) and open the
                    // local swapper STANDALONE. Record where to return on cancel, then
                    // build the hub from fresh cross-daemon discovery (flagging the row we
                    // just left as `is_foreground`).
                    Ok(render::ClientTransition::OpenSwapper) => {
                        teardown_connection(&handle, conn);
                        prev_session = current_session_id.take();
                        ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                    }
                    // `/new` (`kill = false`) / `/new kill` (`kill = true`): DETACH from this
                    // daemon and attach a BRAND-NEW one. In the daemon-per-session world a
                    // daemon owns exactly ONE session, so `/new` makes another DAEMON, not a
                    // tab. On `kill` we queue `QuitDaemon` on the request channel BEFORE
                    // teardown so the writer flushes it (the old daemon releases its lock,
                    // drops its session, and unlinks its socket) ahead of the polite `Detach`;
                    // on plain `/new` we only `Detach` and leave the old daemon cooking
                    // (resumable via the swapper). Then mint a fresh uuid and attach its daemon
                    // (spawned on demand by `attach_session`). `prev_session` is deliberately
                    // LEFT as-is — a `/new` is not a swapper cancel, so there is nothing to
                    // "return to". If the attach of the new session fails we DEGRADE to the
                    // swapper (rebuilt from fresh discovery) rather than crash, matching the
                    // failed-Pick degrade below.
                    Ok(render::ClientTransition::NewSession { kill }) => {
                        if kill {
                            // Reap the old daemon: queue QuitDaemon BEFORE teardown so the
                            // writer drains it (then the Detach) before the socket closes.
                            // The client is its daemon's controller, so QuitDaemon is accepted.
                            let _ = conn.req_tx.send(ClientRequest::QuitDaemon);
                        }
                        teardown_connection(&handle, conn);
                        let new_id = uuid::Uuid::new_v4().to_string();
                        match attach_session(&mut terminal, &handle, &new_id) {
                            Ok(conn) => {
                                current_session_id = Some(new_id);
                                ClientState::Attached(conn)
                            }
                            Err(e) => {
                                eprintln!(
                                    "koma: could not start a new session {new_id}: {e:#}"
                                );
                                // Degrade to the swapper (fresh discovery). Don't disturb
                                // `prev_session`; the old daemon (if not killed) is still in
                                // the discovered list.
                                ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                            }
                        }
                    }
                    // A render error ends the loop like an exit, but the error is carried
                    // out and returned AFTER this connection's teardown so the daemon is
                    // never orphaned by an early return.
                    Err(e) => {
                        teardown_connection(&handle, conn);
                        render_result = Err(e);
                        break;
                    }
                }
            }

            // --- SWAPPER: the detached `/resume` picker ---
            ClientState::Swapper(mut hub) => match run_swapper(&mut terminal, &mut hub, prev_session.as_deref())? {
                // Picked a target session: attach to its daemon (spawning if needed). On
                // success it becomes the foreground; on failure DEGRADE to the swapper
                // rebuilt from fresh discovery (the dead/unreachable session drops out)
                // rather than crash — the user can pick again.
                SwapperOutcome::Pick(target) => match attach_session(&mut terminal, &handle, &target) {
                    Ok(conn) => {
                        current_session_id = Some(target);
                        ClientState::Attached(conn)
                    }
                    Err(e) => {
                        eprintln!("koma: could not attach to session {target}: {e:#}");
                        ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                    }
                },
                // Cancelled: reconnect to the previous session if there was one; otherwise
                // (a `--resume` cold start with nothing to return to) exit cleanly. A
                // failed reconnect to a since-died previous daemon also degrades back to
                // the swapper instead of crashing.
                SwapperOutcome::Cancel => match prev_session.take() {
                    Some(prev) => match attach_session(&mut terminal, &handle, &prev) {
                        Ok(conn) => {
                            current_session_id = Some(prev);
                            ClientState::Attached(conn)
                        }
                        Err(e) => {
                            eprintln!("koma: could not reconnect to session {prev}: {e:#}");
                            ClientState::Swapper(build_local_hub(None))
                        }
                    },
                    None => break,
                },
            },
        };
    }

    // Every live connection was already torn down inside the `Attached` arm it exited from
    // (so there is no double-detach and no connection to clean up here). A break straight
    // out of the swapper has no connection at all. Drop the runtime LAST so the active
    // connection's reader task (if any) is cancelled after exit.
    drop(rt);

    render_result
}

/// Run the `/select` transcript dump on the CLIENT's terminal.
///
/// Re-exported so `runtime/mod.rs` can optionally reference it; the actual
/// implementation lives in `render::client_select_dump` (called from `render_loop`).
pub(super) fn client_select_dump(
    terminal: &mut ratatui::Terminal<CrosstermBackend<std::io::Stdout>>,
    shadow: &crate::app::state::AppState,
) -> Result<()> {
    render::client_select_dump(terminal, shadow)
}

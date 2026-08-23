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
use super::diff::{compute_analytics, compute_file_diff, compute_usage_preview};
use super::git_host;
use super::host_catalogue::{
    build_host_agents_values, build_host_oauth_state, fetch_models_for_provider,
    fetch_routes_for_provider,
};
use super::host_config::{apply_swapper_config_mutation, push_swapper_config};
use super::project::push_hub;
use super::push_proto::{
    push_agents_values, push_analytics, push_file_diff, push_model_list, push_oauth_state,
    push_remote_state, push_route_list, push_settings_values, push_switching, push_usage_preview,
};
use super::store_host;
use super::tutorial_host;
use super::swapper::build_local_hub;
use super::{push_loop, render, HostCtl, StreamView};

/// Resolve a saved/ad-hoc remote target + session into [`HostStep::RemoteAttach`].
/// Used by `koma gui --session <id> remote user@host` for a second window on the
/// same remote session (multi-attach via a second process + ControlMaster).
fn bootstrap_remote_attach_step(
    target_str: &str,
    session_id: &str,
    key: Option<&str>,
    port: Option<u16>,
    cwd: Option<String>,
) -> anyhow::Result<HostStep> {
    if session_id.is_empty() || session_id.contains('\0') {
        anyhow::bail!("invalid session id");
    }
    let mut target = crate::remote::parse_target(target_str)?;
    if let Some(p) = port {
        target.port = Some(p);
    }
    if let Some(k) = key {
        target.key = Some(k.to_string());
    }
    // Prefer saved-host metadata (key path + stable id for password vault).
    let hosts = crate::remote::hosts::load_hosts();
    let matched = hosts.hosts.iter().find(|h| {
        h.address() == target_str
            || format!("{}@{}", h.user, h.host) == target_str
            || (h.user == target.user
                && h.host == target.host
                && h.port == target.port.unwrap_or(22))
    });
    let host_id = if let Some(h) = matched {
        if target.key.is_none() {
            target.key = h.key_path.clone();
        }
        if target.port.is_none() && h.port != 22 {
            target.port = Some(h.port);
        }
        h.id.clone()
    } else {
        // Fall back to address-keyed secrets lookup; do NOT mint a random id
        // (that would never hit the password vault).
        crate::remote::secrets::host_id_for_address(&target.user, &target.host, target.port)
            .unwrap_or_else(|| format!("{}@{}", target.user, target.host))
    };

    let password = crate::remote::secrets::get_remote_password(&host_id);
    let auth = match password.as_ref() {
        Some(pw) => Some(crate::remote::auth::SshAuth::from_password(pw.clone())?),
        None => None,
    };
    crate::remote::bootstrap::ensure_koma_compatible(&target, auth.as_ref())
        .map_err(|e| anyhow::anyhow!("remote bootstrap failed: {e:#}"))?;
    let koma_path = crate::remote::ssh::find_koma(&target, auth.as_ref())
        .map_err(|e| anyhow::anyhow!("cannot find remote koma: {e:#}"))?;
    Ok(HostStep::RemoteAttach {
        ctx: Box::new(super::remote_ctl::RemoteCtx {
            host_id,
            target,
            password,
            koma_path,
        }),
        session_id: session_id.to_string(),
        cwd,
    })
}

/// The host-relay run-loop's next step, mirroring [`super::ClientState`] for the headless
/// GUI host: show the swapper, attach a session, or leave.
enum HostStep {
    /// Show the detached session swapper (the hub) and wait for a pick.
    Swapper,
    /// Host is live (auth+bootstrap done); show remote hub and wait for session/folder pick.
    /// No SSH `koma server` child yet — path listing and session attach use `ctx`.
    RemoteHub {
        ctx: Box<super::remote_ctl::RemoteCtx>,
    },
    /// Attach to this session UUID and fold its frames into pushes. `workdir` is the
    /// folder a GUI `[+ new session]` native-picker chose (the new session's working
    /// dir); `None` for every other attach (existing pick, `--session` boot, daemon
    /// `/new` hand-off) inherits the host's cwd.
    Attach {
        id: String,
        workdir: Option<std::path::PathBuf>,
    },
    /// SSH-attach a remote session using a retained host context.
    RemoteAttach {
        ctx: Box<super::remote_ctl::RemoteCtx>,
        session_id: String,
        cwd: Option<String>,
    },
    /// Live SSH-bridged remote session.
    Remote {
        active: Box<super::remote_ctl::ActiveRemote>,
        session_id: String,
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
    crate::app::runtime::manage::ensure_daemon_running(session_id, false, workdir).map_err(
        |e| anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}"),
    )?;

    let sock_path = store::daemon_sock_path(session_id)?;
    let my_fingerprint = store::build_fingerprint();

    let mut conn = connect_attach_and_handshake(handle, &sock_path, session_id)?;
    let mut already_restarted = false;
    while conn
        .daemon_version
        .as_deref()
        .is_some_and(|v| v != my_fingerprint)
    {
        if already_restarted {
            crate::model::store::append_global_error_log(
                "gui",
                "daemon still reports a different build after a restart; continuing against it",
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

        conn = connect_attach_and_handshake(handle, &sock_path, session_id)?;
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
    push: impl Fn(String) + Clone + Send + Sync + 'static,
    ctl_tx: std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: std::sync::mpsc::Receiver<HostCtl>,
    live_req: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: std::sync::Arc<std::sync::Mutex<StreamView>>,
) {
    // The client owns no sessions; it needs the config dirs only to resolve sockets.
    let _ = store::ensure_dirs();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            crate::model::store::append_global_error_log(
                "gui",
                &format!("could not build the host-relay tokio runtime: {e}"),
            );
            return;
        }
    };
    let handle = rt.handle().clone();

    let mut push_state = push_loop::PushState::new();
    // The session the host is (or was) attached to, so the swapper flags the row it
    // came from as `is_foreground` and a `ToSwapper` fallback remembers it.
    let mut current_session_id: Option<String> = None;

    // Host-side PTY manager for the GUI terminal view. Shared between the
    // swapper and attached control loops so terminal sessions survive state
    // transitions (the React side drives create/kill explicitly).
    let terminal_manager = std::sync::Arc::new(std::sync::Mutex::new(
        super::terminal_host::TerminalManager::new(push.clone()),
    ));
    // Host-spawned language servers for Monaco (completion/hover/definition/diagnostics).
    // Shared across swapper/attached like TerminalManager so open docs survive attach.
    let lsp_manager = std::sync::Arc::new(std::sync::Mutex::new(
        crate::lsp::LspManager::new(push.clone()),
    ));

    // Startup: attach directly to `--session`, else open cold into the swapper.
    // With `remote_target` + `session`, open a second-window remote attach
    // (multi-attach the same remote session from another GUI process).
    let mut step = if let (Some(session_id), Some(target_str)) =
        (opts.session.clone(), opts.remote_target.clone())
    {
        match bootstrap_remote_attach_step(&target_str, &session_id, opts.remote_key.as_deref(), opts.remote_port, opts.cwd.clone()) {
            Ok(step) => step,
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "gui",
                    &format!("remote --session boot failed: {e:#}"),
                );
                // Surface to the webview so the second window is not a silent local hub.
                let envelope = serde_json::json!({
                    "k": "RemoteState",
                    "state": "error",
                    "error": format!("remote attach failed: {e:#}"),
                });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
                HostStep::Swapper
            }
        }
    } else if let Some(id) = opts.session.clone() {
        HostStep::Attach { id, workdir: None }
    } else {
        HostStep::Swapper
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
                &HostLocalManagers {
                    terminal: std::sync::Arc::clone(&terminal_manager),
                    lsp: std::sync::Arc::clone(&lsp_manager),
                },
            ),
            HostStep::RemoteHub { ctx } => host_remote_hub(
                &handle,
                &push,
                &ctl_tx,
                &ctl_rx,
                &mut push_state,
                &mut current_session_id,
                *ctx,
                &terminal_manager,
                &lsp_manager,
            ),
            HostStep::RemoteAttach {
                ctx,
                session_id,
                cwd,
            } => host_remote_attach(
                &handle,
                &push,
                &ctl_tx,
                &ctl_rx,
                &live_req,
                &live_marks,
                &live_view,
                &mut push_state,
                &mut current_session_id,
                *ctx,
                session_id,
                cwd,
                &terminal_manager,
                &lsp_manager,
            ),
            HostStep::Remote { active, session_id } => host_remote(
                &handle,
                &push,
                &ctl_tx,
                &ctl_rx,
                &live_req,
                &live_marks,
                &live_view,
                &mut push_state,
                &mut current_session_id,
                *active,
                session_id,
                &terminal_manager,
                &lsp_manager,
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
                &terminal_manager,
                &lsp_manager,
            ),
        };
    }

    // Kill any remaining terminal sessions before exiting. Must happen before
    // the runtime drop since reader threads hold Arc clones of the push sink.
    {
        let _ = terminal_manager.lock().map(|mut mgr| mgr.cleanup_all());
        let _ = lsp_manager.lock().map(|mut mgr| mgr.cleanup_all());
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

/// Spawn an OFF-THREAD kill of a **remote** session-daemon over SSH, then refresh the hub.
///
/// Uses [`crate::remote::sessions::kill_session_over_ssh`] (`koma daemon kill --session`)
/// so the remote hub Kill button never probes a local socket. Disconnect-from-host is a
/// separate control (`DisconnectRemote`) and must not call this.
pub(super) fn spawn_remote_kill_and_refresh(
    ctl_tx: std::sync::mpsc::Sender<HostCtl>,
    target: crate::remote::RemoteTarget,
    password: Option<String>,
    id: String,
) {
    std::thread::spawn(move || {
        let auth = password
            .as_deref()
            .map(|p| crate::remote::auth::SshAuth::new(p.to_string()))
            .transpose()
            .ok()
            .flatten();
        let _ = crate::remote::sessions::kill_session_over_ssh(&target, auth.as_ref(), &id);
        let _ = ctl_tx.send(HostCtl::RefreshHub);
    });
}

/// Spawn an OFF-THREAD physical delete of a **remote** on-disk history session over SSH,
/// then refresh the hub.
///
/// Uses [`crate::remote::sessions::delete_session_over_ssh`] (`koma daemon delete --session`).
/// Never touches laptop disk — remote history rows use synthetic paths that must not be
/// passed to [`store::delete_session`].
pub(super) fn spawn_remote_delete_and_refresh(
    ctl_tx: std::sync::mpsc::Sender<HostCtl>,
    target: crate::remote::RemoteTarget,
    password: Option<String>,
    id: String,
) {
    std::thread::spawn(move || {
        let auth = password
            .as_deref()
            .map(|p| crate::remote::auth::SshAuth::new(p.to_string()))
            .transpose()
            .ok()
            .flatten();
        let _ = crate::remote::sessions::delete_session_over_ssh(&target, auth.as_ref(), &id);
        let _ = ctl_tx.send(HostCtl::RefreshHub);
    });
}

/// Result of one off-thread remote path listing: (attempt, Ok((path, dirs)) | Err).
type PathListReply = (u64, Result<(String, Vec<String>), String>);

/// Expand `~` / `~/…` against remote `$HOME`. Absolute paths pass through.
fn expand_remote_home(
    target: &crate::remote::RemoteTarget,
    auth: Option<&crate::remote::auth::SshAuth>,
    path: &str,
) -> String {
    let path = path.trim();
    if path.is_empty() || path == "~" {
        return match crate::remote::ssh::exec_remote(target, "printf '%s' \"$HOME\"", auth) {
            Ok(home) if !home.is_empty() => home,
            _ => "/".to_string(),
        };
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let home = match crate::remote::ssh::exec_remote(target, "printf '%s' \"$HOME\"", auth) {
            Ok(home) if !home.is_empty() => home,
            _ => return format!("/{rest}"),
        };
        return format!(
            "{}/{}",
            home.trim_end_matches('/'),
            rest.trim_start_matches('/')
        );
    }
    path.to_string()
}

/// Off-thread `list_dirs` for the remote path picker.
fn spawn_remote_path_list(
    tx: std::sync::mpsc::Sender<PathListReply>,
    attempt: u64,
    target: crate::remote::RemoteTarget,
    password: Option<String>,
    path: String,
) {
    std::thread::spawn(move || {
        let auth = password
            .as_deref()
            .map(|p| crate::remote::auth::SshAuth::new(p.to_string()))
            .transpose()
            .ok()
            .flatten();
        let list_path = expand_remote_home(&target, auth.as_ref(), &path);
        let result = crate::remote::ssh::list_dirs(&target, &list_path, auth.as_ref())
            .map(|dirs| (list_path.clone(), dirs))
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send((attempt, result));
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
        let live: std::collections::HashSet<String> =
            crate::app::runtime::manage::list_live_sessions()
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

// `fetch_models_for_provider`, `fetch_routes_for_provider`, `build_host_agents_values`,
// and `build_host_oauth_state` moved to the sibling `host_catalogue` module (file size) —
// see the `use super::host_catalogue::{...}` import above.

/// Host-local managers shared across the swapper control loop (PTY + LSP).
/// Bundled so `host_swapper` stays under the clippy arg limit without an allow.
struct HostLocalManagers {
    terminal: std::sync::Arc<std::sync::Mutex<super::terminal_host::TerminalManager>>,
    lsp: std::sync::Arc<std::sync::Mutex<crate::lsp::LspManager>>,
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
    managers: &HostLocalManagers,
) -> HostStep {
    let terminal_manager = &managers.terminal;
    let lsp_manager = &managers.lsp;
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

    // The host-local OAuth login flow's abort handle, if one is currently in flight
    // (`HostCtl::StartOAuth` below) — mirrors `AppStateRest::oauth_task` on the daemon
    // side. Local to this swapper invocation: a fresh `host_swapper` call (re-entering
    // the hub after an attach) naturally starts with none in flight, same as a fresh
    // daemon session.
    let mut oauth_task: Option<tokio::task::AbortHandle> = None;
    let (remote_state_tx, remote_state_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::RemoteStateUpdate>();
    let (remote_ready_tx, remote_ready_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::ReadyRemote>();
    let remote_shared = std::sync::Arc::new(super::remote_ctl::RemoteSessionShared::new());

    loop {
        // Push worker state first so `ready` clears the GUI's connection
        // overlay before the remote hub starts rendering.
        while let Ok(update) = remote_state_rx.try_recv() {
            if !remote_shared.is_current(update.attempt_id) {
                continue;
            }
            push_remote_state(
                push,
                &update.state,
                update.host_id.as_deref(),
                update.user.as_deref(),
                update.host.as_deref(),
                update.session_id.as_deref(),
                update.error.as_deref(),
                &update.sessions,
            );
        }
        while let Ok(ready) = remote_ready_rx.try_recv() {
            if !remote_shared.is_current(ready.attempt_id) {
                continue;
            }
            // Re-push ready with sessions in case the last state update raced.
            push_remote_state(
                push,
                "ready",
                Some(&ready.ctx.host_id),
                Some(&ready.ctx.target.user),
                Some(&ready.ctx.target.host),
                None,
                None,
                &ready.sessions,
            );
            return HostStep::RemoteHub {
                ctx: Box::new(ready.ctx),
            };
        }

        match ctl_rx.recv_timeout(std::time::Duration::from_millis(16)) {
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
            Ok(ctl @ HostCtl::FileTree { .. })
            | Ok(ctl @ HostCtl::FileRead { .. })
            | Ok(ctl @ HostCtl::FileSave { .. })
            | Ok(ctl @ HostCtl::FileCreate { .. })
            | Ok(ctl @ HostCtl::FileRename { .. })
            | Ok(ctl @ HostCtl::FileDelete { .. })
            | Ok(ctl @ HostCtl::FileWriteBytes { .. })
            | Ok(ctl @ HostCtl::FileDownloadBytes { .. })
            | Ok(ctl @ HostCtl::FileContentSearch { .. })
            | Ok(ctl @ HostCtl::FileContentReplace { .. }) => {
                let push2 = P::clone(push);
                let workdirs = current
                    .and_then(super::diff::session_workdirs_for)
                    .unwrap_or_default();
                let session = current.map(str::to_string);
                std::thread::spawn(move || {
                    match &ctl {
                        HostCtl::FileContentSearch { .. } | HostCtl::FileContentReplace { .. } => {
                            super::content_search::handle_content_ctl(
                                &ctl,
                                &push2,
                                &workdirs,
                                session.as_deref(),
                            );
                        }
                        _ => {
                            super::file_ops::handle_file_ctl(
                                &ctl,
                                &push2,
                                &workdirs,
                                session.as_deref(),
                            );
                        }
                    }
                });
            }
            #[cfg(feature = "linker")]
            Ok(HostCtl::ImportGraph {
                path,
                depth,
                direction,
                filter_roots,
                filter_languages,
                session_id,
                request_id,
            }) => {
                // Resolve the foreground session's configured workdirs for
                // session-scoped visualisation (never daemon-global).
                let wds = current
                    .and_then(super::diff::session_workdirs_for)
                    .unwrap_or_default();
                let configured_roots = crate::linker::client::canonical_roots(&wds);
                let configured_root_map = crate::linker::client::configured_root_map(&wds);
                let resolved_session = session_id.or_else(|| current.map(str::to_string));
                super::import_graph::spawn_import_graph(
                    P::clone(push),
                    super::import_graph::ImportGraphJob {
                        path,
                        depth,
                        direction,
                        filter_roots,
                        filter_languages,
                        configured_roots,
                        configured_root_map,
                        session_id: resolved_session,
                        request_id,
                    },
                );
            }
            #[cfg(feature = "linker")]
            Ok(HostCtl::ImportGraphImpact {
                path,
                depth,
                request_id,
                session_id,
            }) => {
                // Resolve the foreground session's configured workdirs for
                // session-scoped impact analysis (never daemon-global).
                let configured_roots = current
                    .and_then(super::diff::session_workdirs_for)
                    .map(|wds| crate::linker::client::canonical_roots(&wds))
                    .unwrap_or_default();
                let resolved_session = session_id.or_else(|| current.map(str::to_string));
                super::import_graph::spawn_import_graph_impact(
                    P::clone(push),
                    path,
                    depth,
                    request_id,
                    configured_roots,
                    resolved_session,
                );
            }
            #[cfg(feature = "linker")]
            Ok(HostCtl::ImportGraphReindex { request_id }) => {
                // Manual reindex: reconcile/register the foreground session's
                // current workdirs, issue Rescan, poll until the scan completes,
                // then refresh the scoped visualization. Entirely off-thread.
                let session_id = current.unwrap_or_default().to_string();
                let wds = current
                    .and_then(super::diff::session_workdirs_for)
                    .unwrap_or_default();
                let configured_roots = crate::linker::client::canonical_roots(&wds);
                let configured_root_map = crate::linker::client::configured_root_map(&wds);
                super::import_graph::spawn_import_graph_reindex(
                    P::clone(push),
                    session_id,
                    configured_roots,
                    configured_root_map,
                    None, // All roots after reindex
                    None,
                    request_id,
                );
            }
            // Explore GIT panel + Settings SSH-key vault, all opened/mutated while
            // detached (StartScreen / swapper): git/fs/`ssh-keygen` are blocking, so
            // each runs on a plain OS thread rather than the async runtime, and NEVER
            // touches the daemon in either host state. Bodies live in the sibling
            // `git_host` module (shared with `push_loop`'s attached twin) — see there
            // for the per-op reasoning (mutations push a `GitOp`/`KeyOp` reply THEN a
            // follow-up refreshed `GitStatus`/`KeyList`).
            Ok(HostCtl::GitStatus) => {
                git_host::spawn_git_status(P::clone(push), current.map(str::to_string));
            }
            Ok(HostCtl::GitDiff { path, staged }) => {
                git_host::spawn_git_diff(P::clone(push), current.map(str::to_string), path, staged);
            }
            Ok(HostCtl::GitStage { paths }) => {
                git_host::spawn_git_stage(P::clone(push), current.map(str::to_string), paths);
            }
            Ok(HostCtl::GitUnstage { paths }) => {
                git_host::spawn_git_unstage(P::clone(push), current.map(str::to_string), paths);
            }
            Ok(HostCtl::GitDiscard { paths }) => {
                git_host::spawn_git_discard(P::clone(push), current.map(str::to_string), paths);
            }
            Ok(HostCtl::GitCommit { message }) => {
                git_host::spawn_git_commit(P::clone(push), current.map(str::to_string), message);
            }
            // Commit-graph panel: same host-local reasoning as `GitStatus`/`GitDiff`.
            Ok(HostCtl::GitGraph { limit, skip }) => {
                git_host::spawn_git_graph(P::clone(push), current.map(str::to_string), limit, skip);
            }
            Ok(HostCtl::GitCommitDetail { sha }) => {
                git_host::spawn_commit_detail(P::clone(push), current.map(str::to_string), sha);
            }
            Ok(HostCtl::GitCommitDiff { sha, path }) => {
                git_host::spawn_commit_diff(P::clone(push), current.map(str::to_string), sha, path);
            }
            // Bubble/activity chart (GK5a): same host-local reasoning as
            // `GitStatus`/`GitGraph`.
            Ok(HostCtl::GitActivity { path, limit }) => {
                git_host::spawn_git_activity(
                    P::clone(push),
                    current.map(str::to_string),
                    path,
                    limit,
                );
            }
            Ok(HostCtl::SetGitKey { name }) => {
                git_host::spawn_set_git_key(P::clone(push), current.map(str::to_string), name);
            }
            Ok(HostCtl::GitFetch) => {
                git_host::spawn_git_fetch(P::clone(push), current.map(str::to_string));
            }
            Ok(HostCtl::GitPull) => {
                git_host::spawn_git_pull(P::clone(push), current.map(str::to_string));
            }
            Ok(HostCtl::GitPush { mode, root }) => {
                git_host::spawn_git_push(P::clone(push), current.map(str::to_string), mode, root);
            }
            // Source Control toolbar stash ops (GK4a): same host-local reasoning
            // as `GitStatus`/`GitFetch` above. Bodies live in `git_host`.
            Ok(HostCtl::GitStash) => {
                git_host::spawn_git_stash(P::clone(push), current.map(str::to_string));
            }
            Ok(HostCtl::GitStashPop) => {
                git_host::spawn_git_stash_pop(P::clone(push), current.map(str::to_string));
            }
            Ok(HostCtl::GitStashList) => {
                git_host::spawn_git_stash_list(P::clone(push), current.map(str::to_string));
            }
            // Branch-switcher popover / graph context menu (G4): same host-local
            // reasoning as `GitStatus`/`GitGraph`. Bodies live in the shared
            // `git_branch`/`git_host` modules.
            Ok(HostCtl::GitBranchList { request_id }) => {
                git_host::spawn_git_branch_list(
                    P::clone(push),
                    current.map(str::to_string),
                    request_id,
                );
            }
            // Source Control multi-repo picker (discover + set-active): same host-local
            // reasoning as `GitBranchList`/`SetGitKey` above.
            Ok(HostCtl::GitRepos) => {
                git_host::spawn_git_repos(P::clone(push), current.map(str::to_string));
            }
            Ok(HostCtl::SetActiveRepo { root }) => {
                git_host::spawn_set_active_repo(P::clone(push), current.map(str::to_string), root);
            }
            Ok(HostCtl::GitCheckout { ref_name, root }) => {
                git_host::spawn_git_checkout(
                    P::clone(push),
                    current.map(str::to_string),
                    ref_name,
                    root,
                );
            }
            Ok(HostCtl::GitCreateBranch {
                name,
                start,
                checkout,
                root,
            }) => {
                git_host::spawn_git_create_branch(
                    P::clone(push),
                    current.map(str::to_string),
                    name,
                    start,
                    checkout,
                    root,
                );
            }
            // Commit-graph interactive/destructive ops (G5b): same host-local
            // reasoning as `GitCheckout`/`GitCreateBranch` above. Bodies live in
            // the shared `git_destructive`/`git_host` modules; a mutation's
            // follow-up `GitStatus` re-push carries the fresh `inProgress`/
            // `conflicted` state (see `git::compute_git_status`).
            Ok(HostCtl::GitCherryPick { sha }) => {
                git_host::spawn_git_cherry_pick(P::clone(push), current.map(str::to_string), sha);
            }
            Ok(HostCtl::GitRevert { sha }) => {
                git_host::spawn_git_revert(P::clone(push), current.map(str::to_string), sha);
            }
            Ok(HostCtl::GitReset { sha, mode }) => {
                git_host::spawn_git_reset(P::clone(push), current.map(str::to_string), sha, mode);
            }
            Ok(HostCtl::GitMerge { ref_name }) => {
                git_host::spawn_git_merge(P::clone(push), current.map(str::to_string), ref_name);
            }
            Ok(HostCtl::GitRebase { upstream, branch }) => {
                git_host::spawn_git_rebase(
                    P::clone(push),
                    current.map(str::to_string),
                    upstream,
                    branch,
                );
            }
            Ok(HostCtl::GitOpAbort { kind }) => {
                git_host::spawn_git_op_abort(P::clone(push), current.map(str::to_string), kind);
            }
            Ok(HostCtl::GitOpContinue { kind }) => {
                git_host::spawn_git_op_continue(P::clone(push), current.map(str::to_string), kind);
            }
            Ok(HostCtl::KeyList) => {
                git_host::spawn_key_list(P::clone(push));
            }
            Ok(HostCtl::KeyGenerate { name, comment }) => {
                git_host::spawn_key_generate(P::clone(push), name, comment);
            }
            Ok(HostCtl::KeyImport { name, private_key }) => {
                git_host::spawn_key_import(P::clone(push), name, private_key);
            }
            Ok(HostCtl::KeyDelete { name }) => {
                git_host::spawn_key_delete(P::clone(push), name);
            }
            Ok(HostCtl::KeyReveal { name, private }) => {
                git_host::spawn_key_reveal(P::clone(push), name, private);
            }
            Ok(HostCtl::LspStatus) => {
                super::lsp_host::spawn_lsp_status(P::clone(push));
            }
            Ok(HostCtl::LspInstall { id, all, force }) => {
                super::lsp_host::spawn_lsp_install(P::clone(push), id, all, force);
            }
            Ok(HostCtl::LspUninstall { id }) => {
                super::lsp_host::spawn_lsp_uninstall(P::clone(push), id);
            }

            Ok(HostCtl::LspDidOpen { root, path, language_id, text }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidOpen { root, path, language_id, text },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDidChange { root, path, text }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidChange { root, path, text },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDidSave { root, path, text }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidSave { root, path, text },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDidClose { root, path }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidClose { root, path },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspCompletion { root, path, line, character, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspCompletion { root, path, line, character, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspHover { root, path, line, character, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspHover { root, path, line, character, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDefinition { root, path, line, character, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDefinition { root, path, line, character, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspReferences {
                root,
                path,
                line,
                character,
                include_declaration,
                request_id,
            }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspReferences {
                        root,
                        path,
                        line,
                        character,
                        include_declaration,
                        request_id,
                    },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDocumentSymbol { root, path, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDocumentSymbol { root, path, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            // Extension STORE browse/detail/installed-list opened while detached
            // (StartScreen / swapper, e.g. the Store tab mounting on the home screen
            // with no session): koma.run is a PUBLIC endpoint and the installed list is
            // a local config read, so both NEVER touch the daemon in either host state
            // — see `store_host`. Bodies live in the sibling `store_host` module
            // (shared with `push_loop`'s attached twin).
            Ok(HostCtl::StoreBrowse { query, category }) => {
                store_host::spawn_store_browse(P::clone(push), query, category);
            }
            Ok(HostCtl::TutorialChat { id, messages }) => {
                tutorial_host::spawn_tutorial_chat(P::clone(push), id, messages);
            }
            Ok(HostCtl::StoreDetail { id }) => {
                store_host::spawn_store_detail(P::clone(push), id);
            }
            Ok(HostCtl::ListInstalledExtensions) => {
                store_host::spawn_list_installed(P::clone(push));
            }
            Ok(HostCtl::GetInstalledExtensionDetail { id }) => {
                store_host::spawn_get_installed_detail(P::clone(push), id);
            }
            // Install/uninstall arrived with no session attached (always true in the
            // swapper): run them HOST-LOCAL rather than failing closed — see
            // `store_host::spawn_install`/`spawn_uninstall` for what's covered (and what's
            // intentionally skipped, since it self-heals on the next session start).
            // `InstallExtension` needs the tokio runtime (`fresh_key`/the download are
            // async); `UninstallExtension` is synchronous fs + a config save, so it gets
            // its own plain thread like the browse/detail workers above.
            Ok(HostCtl::InstallExtension { id, version }) => {
                store_host::spawn_install(handle, P::clone(push), id, version);
            }
            Ok(HostCtl::UninstallExtension { id }) => {
                store_host::spawn_uninstall(P::clone(push), id);
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
            // GUI Analytics tab opened while detached (StartScreen / swapper): the
            // ledger is a global file the host reads directly, so this never
            // touches a daemon in either state — see `compute_analytics`. Sqlite
            // I/O is blocking, so it runs on a plain OS thread like
            // `UsagePreview` above. All correlation inputs ride along unchanged
            // so the reply echoes them (the React tab drops a reply whose
            // reqSeq/scope/session/range/metric no longer matches what's current).
            Ok(HostCtl::Analytics {
                req_seq,
                session,
                scope,
                range,
                metric,
            }) => {
                let push2 = P::clone(push);
                std::thread::spawn(move || {
                    let result = compute_analytics(req_seq, scope, session, range, metric);
                    push_analytics(&push2, result);
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
                    d.coding_autosave,
                    d.internet_mode.as_str().to_string(),
                    cfg.palette,
                    String::new(),
                    d.subagent_max_turns,
                );
            }
            // GUI /agents dashboard opened while detached (StartScreen / swapper): there is
            // no foreground session, so answer from `load_registry(None)` (built-in + global
            // only) + the GLOBAL config catalogue. ALWAYS a reply so the dashboard's loading
            // state clears. Cheap, synchronous (a registry + config load), so it runs inline.
            Ok(HostCtl::GetAgents) => {
                let (agents, catalogue_models, catalogue_providers) = build_host_agents_values();
                // The tool-picker options: the SAME shared source the daemon + TUI use, so the
                // un-attached reply offers exactly the same set as the attached one.
                let available_tools = crate::tool::agent_selectable_tools();
                push_agents_values(
                    push,
                    0, // req_seq — no correlation for host-built fallback
                    agents,
                    catalogue_models,
                    catalogue_providers,
                    available_tools,
                );
            }
            // GUI OAuth screen opened while detached (StartScreen / swapper): there is no
            // attached daemon to run a login flow on, so answer from the GLOBAL config — the
            // persisted connections + the provider catalogue, phase "idle". The login FLOW is
            // attached-only; this read populates the screen pre-session. Cheap, synchronous (a
            // config load), so it runs inline.
            Ok(HostCtl::GetOAuthState) => {
                let (conns, providers) = build_host_oauth_state();
                push_oauth_state(
                    push,
                    "idle".to_string(),
                    None,
                    None,
                    None,
                    None,
                    conns,
                    providers,
                );
            }
            // GUI OAuth login START while detached (the home-screen / pre-session Settings
            // "Sign in" buttons): there is no attached daemon to run the flow, so run it
            // HOST-side instead of silently dropping the request (the prior bug this wave
            // fixes). Resolve the wire `provider` string the SAME way the daemon does
            // (`OAuthProvider::from_wire_id`); an unknown string pushes an immediate
            // `failed` rather than hanging. Supersede any flow already in flight — abort
            // its task first, mirroring `handle_oauth_start`'s supersede — before spawning
            // the new one via the SAME `service::oauth::flow::run_flow` dispatcher the
            // daemon's `Action::OAuthStart` spawns, so this is not a second copy of the
            // five provider flows.
            //
            // Two tasks, mirroring the daemon's spawn + tick-drain split: the FLOW task
            // (`run_flow`) is the one whose abort handle is stored for `CancelOAuth` —
            // aborting it stops an in-progress browser-wait/device-poll immediately, same
            // as the daemon's `oauth_task.abort()`. The DRAIN task awaits each `OAuthEvent`
            // and turns it into an `OAuthState` push (`waiting_url`/`waiting_code`, then a
            // terminal `success` — persisting the connection to the GLOBAL config first —
            // or `failed`); it ends on its own once the flow task's `tx` is dropped
            // (aborted or finished), so only the flow task's handle needs tracking here.
            Ok(HostCtl::StartOAuth { provider }) => {
                if let Some(h) = oauth_task.take() {
                    h.abort();
                }
                match crate::model::app_config::OAuthProvider::from_wire_id(&provider) {
                    Some(p) => {
                        let (tx, mut orx) = tokio::sync::mpsc::unbounded_channel();
                        let flow_join = handle.spawn(crate::service::oauth::flow::run_flow(p, tx));
                        oauth_task = Some(flow_join.abort_handle());

                        let push2 = P::clone(push);
                        handle.spawn(async move {
                            while let Some(ev) = orx.recv().await {
                                match ev {
                                    crate::service::oauth::OAuthEvent::CodexUrl { provider: _, url } => {
                                        let (conns, providers) = build_host_oauth_state();
                                        push_oauth_state(
                                            &push2,
                                            "waiting_url".to_string(),
                                            Some(url),
                                            None,
                                            None,
                                            None,
                                            conns,
                                            providers,
                                        );
                                    }
                                    crate::service::oauth::OAuthEvent::KiloCode {
                                        provider: _,
                                        user_code,
                                        verification_url,
                                    } => {
                                        let (conns, providers) = build_host_oauth_state();
                                        push_oauth_state(
                                            &push2,
                                            "waiting_code".to_string(),
                                            None,
                                            Some(user_code),
                                            Some(verification_url),
                                            None,
                                            conns,
                                            providers,
                                        );
                                    }
                                    crate::service::oauth::OAuthEvent::Success { conn } => {
                                        // Seed the token-refresh cache (fire-and-forget,
                                        // mirrors the daemon's `drain_oauth`), then persist
                                        // the connection to the GLOBAL config — there is no
                                        // in-memory `AppConfig` pre-session, so re-load
                                        // fresh rather than risk clobbering a concurrent
                                        // swapper-side config mutation with a stale copy.
                                        crate::service::oauth::manager::seed(&conn).await;
                                        let mut cfg = crate::model::app_config::AppConfig::load();
                                        cfg.oauth_conns.push(conn);
                                        if let Err(e) = cfg.save() {
                                            crate::model::store::append_global_error_log(
                                                "gui",
                                                &format!(
                                                    "pre-session oauth login saved but config write failed: {e}"
                                                ),
                                            );
                                        }
                                        let (conns, providers) = build_host_oauth_state();
                                        push_oauth_state(
                                            &push2,
                                            "success".to_string(),
                                            None,
                                            None,
                                            None,
                                            None,
                                            conns,
                                            providers,
                                        );
                                        break;
                                    }
                                    crate::service::oauth::OAuthEvent::Failed { error } => {
                                        let (conns, providers) = build_host_oauth_state();
                                        push_oauth_state(
                                            &push2,
                                            "failed".to_string(),
                                            None,
                                            None,
                                            None,
                                            Some(error),
                                            conns,
                                            providers,
                                        );
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    None => {
                        let (conns, providers) = build_host_oauth_state();
                        push_oauth_state(
                            push,
                            "failed".to_string(),
                            None,
                            None,
                            None,
                            Some(format!("unknown oauth provider: {provider}")),
                            conns,
                            providers,
                        );
                    }
                }
            }
            // GUI OAuth login CANCEL while detached: abort whatever host-local flow
            // `StartOAuth` above started (a no-op if none is in flight — `take()` on
            // `None`), then re-push a fresh "idle" `OAuthState` so the Cancel button
            // always lands somewhere instead of leaving the wait screen stranded.
            Ok(HostCtl::CancelOAuth) => {
                if let Some(h) = oauth_task.take() {
                    h.abort();
                }
                let (conns, providers) = build_host_oauth_state();
                push_oauth_state(
                    push,
                    "idle".to_string(),
                    None,
                    None,
                    None,
                    None,
                    conns,
                    providers,
                );
            }
            // GUI OAuth connection delete while detached: remove it from `~/.koma/config.json`,
            // persist, evict its token-refresh cache entry OFF-thread (evict is async), then
            // re-push a fresh "idle" `OAuthState`. Reachable pre-session so a connection is
            // removable before any session exists.
            Ok(HostCtl::DeleteOAuthConn { uuid }) => {
                let mut cfg = crate::model::app_config::AppConfig::load();
                cfg.oauth_conns.retain(|c| c.uuid != uuid);
                if let Err(e) = cfg.save() {
                    crate::model::store::append_global_error_log(
                        "gui",
                        &format!("pre-session oauth delete save failed: {e}"),
                    );
                }
                let uuid2 = uuid.clone();
                handle.spawn(async move {
                    crate::service::oauth::manager::evict(&uuid2).await;
                });
                let (conns, providers) = build_host_oauth_state();
                push_oauth_state(
                    push,
                    "idle".to_string(),
                    None,
                    None,
                    None,
                    None,
                    conns,
                    providers,
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
                return HostStep::Attach {
                    id: new_id,
                    workdir,
                };
            }
            // ─── Remote host management (host-local, fast file I/O) ────────
            Ok(ctl @ HostCtl::GetRemoteHosts)
            | Ok(ctl @ HostCtl::AddRemoteHost { .. })
            | Ok(ctl @ HostCtl::EditRemoteHost { .. })
            | Ok(ctl @ HostCtl::DeleteRemoteHost { .. }) => {
                let mut hosts = crate::remote::hosts::load_hosts();
                let mutated = match ctl {
                    HostCtl::AddRemoteHost {
                        name,
                        user,
                        host,
                        port,
                        key_path,
                    } => {
                        let new_id = crate::model::app_config::new_uuid();
                        crate::remote::hosts::upsert_host(
                            &mut hosts,
                            crate::remote::hosts::RemoteHost {
                                id: new_id,
                                name,
                                user,
                                host,
                                port,
                                key_path,
                                last_connected: None,
                                tags: vec![],
                            },
                        );
                        true
                    }
                    HostCtl::EditRemoteHost {
                        id,
                        name,
                        user,
                        host,
                        port,
                        key_path,
                    } => {
                        if let Some(h) = crate::remote::hosts::host_by_id(&hosts, &id) {
                            let updated = crate::remote::hosts::RemoteHost {
                                id: h.id.clone(),
                                name,
                                user,
                                host,
                                port,
                                key_path,
                                last_connected: h.last_connected,
                                tags: h.tags.clone(),
                            };
                            crate::remote::hosts::upsert_host(&mut hosts, updated);
                            true
                        } else {
                            false
                        }
                    }
                    HostCtl::DeleteRemoteHost { id } => {
                        let deleted = crate::remote::hosts::delete_host(&mut hosts, &id);
                        if deleted {
                            let _ = crate::remote::secrets::delete_remote_password(&id);
                        }
                        deleted
                    }
                    _ => false, // GetRemoteHosts — read-only
                };
                if mutated {
                    let _ = crate::remote::hosts::save_hosts(&hosts);
                }
                let wire_hosts: Vec<serde_json::Value> = hosts
                    .hosts
                    .iter()
                    .map(|h| {
                        // Live = current remoteState host; historical last_connected is separate.
                        serde_json::json!({
                            "id": h.id, "name": h.name, "user": h.user, "host": h.host,
                            "port": h.port, "keyPath": h.key_path,
                            "connected": false,
                            "lastConnected": h.last_connected,
                            "tags": h.tags,
                        })
                    })
                    .collect();
                let envelope = serde_json::json!({ "k": "RemoteHosts", "hosts": wire_hosts });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
            }
            // ─── Remote host connect (available from the detached start screen) ───
            Ok(HostCtl::ConnectRemote { host_id }) => {
                let (pw_tx, pw_rx) = std::sync::mpsc::channel::<String>();
                let (attempt_id, cancelled) = remote_shared.begin(pw_tx);
                push_remote_state(
                    push,
                    "resolving",
                    Some(&host_id),
                    None,
                    None,
                    None,
                    None,
                    &[],
                );
                super::remote_ctl::spawn_connect_worker(
                    attempt_id,
                    host_id,
                    remote_state_tx.clone(),
                    remote_ready_tx.clone(),
                    pw_rx,
                    cancelled,
                    std::sync::Arc::clone(&remote_shared),
                    handle.clone(),
                );
            }
            Ok(HostCtl::SubmitRemotePassword { password }) => {
                remote_shared.submit_password(password);
            }
            Ok(HostCtl::DisconnectRemote) | Ok(HostCtl::CancelRemoteConnect) => {
                remote_shared.cancel();
                push_remote_state(push, "disconnected", None, None, None, None, None, &[]);
            }
            // Remote path selection is only meaningful once an SSH transport is active;
            // the detached local hub has no retained remote context to query.
            Ok(HostCtl::RequestRemotePath)
            | Ok(HostCtl::ListRemotePath { .. })
            | Ok(HostCtl::ConfirmRemotePath { .. })
            | Ok(HostCtl::CancelRemotePath) => {
                let envelope = serde_json::json!({
                    "k": "RemotePathPicker",
                    "state": "error",
                    "error": "no active remote session"
                });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
            }
            // ─── GUI terminal view (host-local PTY lifecycle) ───────────
            // Terminal sessions are managed host-side via the shared
            // TerminalManager. These routes delegate to it; the reader
            // threads spawned by `create` push output/exit envelopes.
            Ok(HostCtl::TerminalCreate { id, cwd }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    if let Err(e) = mgr.create(id, cwd) {
                        crate::model::store::append_global_error_log(
                            "terminal",
                            &format!("terminal create failed: {e}"),
                        );
                    }
                }
            }
            Ok(HostCtl::TerminalInput { id, data }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    mgr.input(&id, &data);
                }
            }
            Ok(HostCtl::TerminalResize { id, cols, rows }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    mgr.resize(&id, cols, rows);
                }
            }
            Ok(HostCtl::TerminalKill { id }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    mgr.kill(&id);
                }
            }
            // The ipc side hung up (window gone) — leave the host.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return HostStep::Done,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
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
    terminal_manager: &std::sync::Arc<std::sync::Mutex<super::terminal_host::TerminalManager>>,
    lsp_manager: &std::sync::Arc<std::sync::Mutex<crate::lsp::LspManager>>,
) -> HostStep {
    let mut conn = match attach_session_headless(handle, &id, workdir.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            crate::model::store::append_global_error_log(
                "gui",
                &format!("host-relay could not attach session {id}: {e:#}"),
            );
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
            terminal_manager,
            lsp_manager,
            None, // local attach
            None, // no remote-fs
            None, // no remote-git
            #[cfg(feature = "linker")]
            None, // no remote-linker
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
    transition_to_step(transition)
}

fn transition_to_step(transition: push_loop::HostTransition) -> HostStep {
    match transition {
        push_loop::HostTransition::Attach { id, workdir } => HostStep::Attach { id, workdir },
        push_loop::HostTransition::ToRemoteHub { ctx } => HostStep::RemoteHub { ctx },
        push_loop::HostTransition::RemoteAttach {
            ctx,
            session_id,
            cwd,
        } => HostStep::RemoteAttach {
            ctx,
            session_id,
            cwd,
        },
        push_loop::HostTransition::Remote {
            connection,
            session_id,
        } => HostStep::Remote {
            active: connection,
            session_id,
        },
        push_loop::HostTransition::DisconnectRemote | push_loop::HostTransition::ToSwapper => {
            HostStep::Swapper
        }
        push_loop::HostTransition::Exit => HostStep::Done,
    }
}

/// Wire `RemoteHosts` list. `live_host_id` marks the currently retained remote ctx
/// (ready/connecting/connected) so the green dot means live, not historical.
fn push_remote_hosts_list(push: &dyn Fn(String), live_host_id: Option<&str>) {
    let hosts = crate::remote::hosts::load_hosts();
    let wire_hosts: Vec<serde_json::Value> = hosts
        .hosts
        .iter()
        .map(|h| {
            let live = live_host_id.is_some_and(|id| id == h.id);
            serde_json::json!({
                "id": h.id, "name": h.name, "user": h.user, "host": h.host,
                "port": h.port, "keyPath": h.key_path,
                "connected": live,
                "lastConnected": h.last_connected,
                "tags": h.tags,
            })
        })
        .collect();
    let envelope = serde_json::json!({ "k": "RemoteHosts", "hosts": wire_hosts });
    if let Ok(json) = serde_json::to_string(&envelope) {
        push(json);
    }
}

/// Detached remote hub: host is authenticated, no session SSH child yet.
/// User picks an existing remote session or opens a folder (path picker).
/// Pass `open_path_picker` when arriving from an attached "new session" so the
/// folder dialog opens immediately.
#[allow(clippy::too_many_arguments)]
fn host_remote_hub<P: Fn(String) + Clone + Send + 'static>(
    handle: &tokio::runtime::Handle,
    push: &P,
    ctl_tx: &std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    push_state: &mut push_loop::PushState,
    current: &mut Option<String>,
    ctx: super::remote_ctl::RemoteCtx,
    terminal_manager: &std::sync::Arc<std::sync::Mutex<super::terminal_host::TerminalManager>>,
    lsp_manager: &std::sync::Arc<std::sync::Mutex<crate::lsp::LspManager>>,
) -> HostStep {
    *current = None;
    push_state.reset();

    // Remote hub: cooking sessions on this host, empty local history.
    let hub = super::swapper::build_remote_hub(&ctx.target, ctx.password(), None);
    push_hub(&hub, push, push_state);
    push_swapper_config(push, push_state);
    push_remote_hosts_list(push, Some(&ctx.host_id));

    let (remote_state_tx, remote_state_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::RemoteStateUpdate>();
    // Session attach from hub is handled by returning RemoteAttach; no connected_rx here.
    let remote_shared = std::sync::Arc::new(super::remote_ctl::RemoteSessionShared::new());
    let oauth_task: Option<tokio::task::AbortHandle> = None;
    // Off-thread path-list replies (attempt id ignores races with cancel).
    let (path_tx, path_rx) = std::sync::mpsc::channel::<PathListReply>();
    let mut path_attempt: u64 = 0;
    let _ = remote_state_tx;

    loop {
        while let Ok(update) = remote_state_rx.try_recv() {
            if !remote_shared.is_current(update.attempt_id) {
                continue;
            }
            push_remote_state(
                push,
                &update.state,
                update.host_id.as_deref(),
                update.user.as_deref(),
                update.host.as_deref(),
                update.session_id.as_deref(),
                update.error.as_deref(),
                &update.sessions,
            );
        }

        // Drain remote path listings.
        while let Ok((attempt, result)) = path_rx.try_recv() {
            if attempt != path_attempt {
                continue;
            }
            match result {
                Ok((path, dirs)) => {
                    let envelope = serde_json::json!({
                        "k": "RemotePathPicker",
                        "state": "ready",
                        "path": path,
                        "dirs": dirs,
                    });
                    if let Ok(json) = serde_json::to_string(&envelope) {
                        push(json);
                    }
                }
                Err(error) => {
                    let envelope = serde_json::json!({
                        "k": "RemotePathPicker",
                        "state": "error",
                        "error": error,
                    });
                    if let Ok(json) = serde_json::to_string(&envelope) {
                        push(json);
                    }
                }
            }
        }

        match ctl_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(HostCtl::Ready) | Ok(HostCtl::RefreshHub) | Ok(HostCtl::ToSwapper) => {
                let hub = super::swapper::build_remote_hub(&ctx.target, ctx.password(), None);
                push_state.reset();
                push_hub(&hub, push, push_state);
                push_swapper_config(push, push_state);
                // Keep remoteState = ready so the GUI stays in remote mode.
                push_remote_state(
                    push,
                    "ready",
                    Some(&ctx.host_id),
                    Some(&ctx.target.user),
                    Some(&ctx.target.host),
                    None,
                    None,
                    &[],
                );
                push_remote_hosts_list(push, Some(&ctx.host_id));
            }
            Ok(HostCtl::Select(id)) => {
                // Existing remote cooking session — attach without minting cwd.
                return HostStep::RemoteAttach {
                    ctx: Box::new(ctx),
                    session_id: id,
                    cwd: None,
                };
            }
            Ok(HostCtl::New { kill, .. }) => {
                // Open the remote path picker (same as RequestRemotePath).
                // kill is for attached /new kill; on the hub there is no live session.
                let _ = kill;
                path_attempt = path_attempt.wrapping_add(1);
                let attempt = path_attempt;
                let envelope = serde_json::json!({
                    "k": "RemotePathPicker",
                    "state": "listing",
                    "path": "~",
                    "dirs": [],
                });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
                spawn_remote_path_list(
                    path_tx.clone(),
                    attempt,
                    ctx.target.clone(),
                    ctx.password.clone(),
                    "~".into(),
                );
            }
            Ok(HostCtl::DisconnectRemote) | Ok(HostCtl::CancelRemoteConnect) => {
                remote_shared.cancel();
                // Full host leave — tear down ControlMaster so credentials/sockets
                // don't linger after the user disconnects.
                crate::remote::ssh::exit_multiplex(&ctx.target);
                push_remote_state(push, "disconnected", None, None, None, None, None, &[]);
                push_remote_hosts_list(push, None);
                return HostStep::Swapper;
            }
            Ok(HostCtl::ConnectRemote { host_id }) => {
                // Switch host: drop current ctx, let the local swapper start connect.
                // (Phase 4 may disconnect-then-connect inline; Phase 1 returns to swapper
                // after pushing disconnect so the user can reconnect.)
                if host_id == ctx.host_id {
                    // Already on this host — refresh hub.
                    let hub = super::swapper::build_remote_hub(&ctx.target, ctx.password(), None);
                    push_hub(&hub, push, push_state);
                    continue;
                }
                crate::remote::ssh::exit_multiplex(&ctx.target);
                push_remote_state(push, "disconnected", None, None, None, None, None, &[]);
                push_remote_hosts_list(push, None);
                // Re-queue the connect so host_swapper picks it up.
                let _ = ctl_tx.send(HostCtl::ConnectRemote { host_id });
                return HostStep::Swapper;
            }
            Ok(HostCtl::SubmitRemotePassword { .. }) => {
                // No in-flight password wait on the hub.
            }
            Ok(HostCtl::RequestRemotePath) => {
                path_attempt = path_attempt.wrapping_add(1);
                let attempt = path_attempt;
                let envelope = serde_json::json!({
                    "k": "RemotePathPicker",
                    "state": "listing",
                    "path": "~",
                    "dirs": [],
                });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
                spawn_remote_path_list(
                    path_tx.clone(),
                    attempt,
                    ctx.target.clone(),
                    ctx.password.clone(),
                    "~".into(),
                );
            }
            Ok(HostCtl::ListRemotePath { path }) => {
                path_attempt = path_attempt.wrapping_add(1);
                let attempt = path_attempt;
                let envelope = serde_json::json!({
                    "k": "RemotePathPicker",
                    "state": "listing",
                    "path": path,
                    "dirs": [],
                });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
                spawn_remote_path_list(
                    path_tx.clone(),
                    attempt,
                    ctx.target.clone(),
                    ctx.password.clone(),
                    path,
                );
            }
            Ok(HostCtl::ConfirmRemotePath { path }) => {
                // Expand ~ before attach so the remote daemon gets an absolute cwd.
                let auth = ctx.make_auth().ok().flatten();
                let cwd = expand_remote_home(&ctx.target, auth.as_ref(), &path);
                let new_id = uuid::Uuid::new_v4().to_string();
                // Close the picker BEFORE Switching — without this the GUI keeps
                // remotePath.state at ready/listing (z-70) over the switcher (z-60)
                // and freezes the whole chrome until something else clears it.
                let close = serde_json::json!({
                    "k": "RemotePathPicker",
                    "state": "cancelled",
                });
                if let Ok(json) = serde_json::to_string(&close) {
                    push(json);
                }
                push_switching(push, &new_id);
                return HostStep::RemoteAttach {
                    ctx: Box::new(ctx),
                    session_id: new_id,
                    cwd: Some(cwd),
                };
            }
            Ok(HostCtl::CancelRemotePath) => {
                path_attempt = path_attempt.wrapping_add(1);
                let envelope = serde_json::json!({
                    "k": "RemotePathPicker",
                    "state": "cancelled",
                });
                if let Ok(json) = serde_json::to_string(&envelope) {
                    push(json);
                }
            }
            Ok(HostCtl::ConfigMutate(req)) => {
                apply_swapper_config_mutation(&req, push, push_state);
            }
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
            Ok(HostCtl::GetRemoteHosts)
            | Ok(HostCtl::AddRemoteHost { .. })
            | Ok(HostCtl::EditRemoteHost { .. })
            | Ok(HostCtl::DeleteRemoteHost { .. }) => {
                // Re-emit hosts with live connected flag for this ctx.
                // Mutations intentionally deferred — hub stays on current host.
                push_remote_hosts_list(push, Some(&ctx.host_id));
            }
            Ok(HostCtl::TerminalCreate { id, cwd }) => {
                // Remote hub: always open a shell on the live remote host, never
                // the local machine the GUI is running on.
                if let Ok(mut mgr) = terminal_manager.lock() {
                    if let Err(e) =
                        mgr.create_remote(id, &ctx.target, ctx.password(), cwd.as_deref())
                    {
                        crate::model::store::append_global_error_log(
                            "terminal",
                            &format!("terminal create failed: {e}"),
                        );
                    }
                }
            }
            Ok(HostCtl::TerminalInput { id, data }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    mgr.input(&id, &data);
                }
            }
            Ok(HostCtl::TerminalResize { id, cols, rows }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    mgr.resize(&id, cols, rows);
                }
            }
            Ok(HostCtl::TerminalKill { id }) => {
                if let Ok(mut mgr) = terminal_manager.lock() {
                    mgr.kill(&id);
                }
            }

            Ok(HostCtl::LspDidOpen { root, path, language_id, text }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidOpen { root, path, language_id, text },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDidChange { root, path, text }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidChange { root, path, text },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDidSave { root, path, text }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidSave { root, path, text },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDidClose { root, path }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDidClose { root, path },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspCompletion { root, path, line, character, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspCompletion { root, path, line, character, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspHover { root, path, line, character, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspHover { root, path, line, character, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDefinition { root, path, line, character, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDefinition { root, path, line, character, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspReferences {
                root,
                path,
                line,
                character,
                include_declaration,
                request_id,
            }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspReferences {
                        root,
                        path,
                        line,
                        character,
                        include_declaration,
                        request_id,
                    },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::LspDocumentSymbol { root, path, request_id }) => {
                super::lsp_host::handle_client_ctl(
                    HostCtl::LspDocumentSymbol { root, path, request_id },
                    std::sync::Arc::clone(lsp_manager),
                );
            }
            Ok(HostCtl::KillSession(id)) => {
                // Kill the remote session-daemon; stay on this host hub.
                // Distinct from DisconnectRemote (leave host, daemons keep cooking).
                spawn_remote_kill_and_refresh(
                    ctl_tx.clone(),
                    ctx.target.clone(),
                    ctx.password.clone(),
                    id,
                );
            }
            Ok(HostCtl::DeleteSession(id)) => {
                // Physically delete a remote HISTORY session over SSH; stay on hub.
                spawn_remote_delete_and_refresh(
                    ctl_tx.clone(),
                    ctx.target.clone(),
                    ctx.password.clone(),
                    id,
                );
            }
            // Everything else is no-op on the detached remote hub (local git/store/
            // coding/oauth ctls don't apply until a session is attached).
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return HostStep::Done,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        let _ = &oauth_task;
    }
}

/// Spawn SSH `koma server` for a remote session and enter the attached remote fold.
#[allow(clippy::too_many_arguments)]
fn host_remote_attach(
    handle: &tokio::runtime::Handle,
    push: &dyn Fn(String),
    ctl_tx: &std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    live_req: &std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: &std::sync::Arc<std::sync::Mutex<StreamView>>,
    push_state: &mut push_loop::PushState,
    current: &mut Option<String>,
    ctx: super::remote_ctl::RemoteCtx,
    session_id: String,
    cwd: Option<String>,
    terminal_manager: &std::sync::Arc<std::sync::Mutex<super::terminal_host::TerminalManager>>,
    lsp_manager: &std::sync::Arc<std::sync::Mutex<crate::lsp::LspManager>>,
) -> HostStep {
    let (remote_state_tx, remote_state_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::RemoteStateUpdate>();
    let (remote_connected_tx, remote_connected_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::ActiveRemote>();
    let remote_shared = std::sync::Arc::new(super::remote_ctl::RemoteSessionShared::new());

    // begin() needs a password channel even when unused (key auth).
    let (pw_tx, _pw_rx) = std::sync::mpsc::channel::<String>();
    let (attempt_id, cancelled) = remote_shared.begin(pw_tx);

    push_switching(push, &session_id);
    push_remote_state(
        push,
        "connecting",
        Some(&ctx.host_id),
        Some(&ctx.target.user),
        Some(&ctx.target.host),
        Some(&session_id),
        None,
        &[],
    );

    // Keep a clone so attach failure can return to the remote hub.
    let hub_ctx = ctx.clone();
    super::remote_ctl::spawn_session_worker(
        attempt_id,
        ctx,
        session_id.clone(),
        cwd,
        remote_state_tx,
        remote_connected_tx,
        cancelled,
        std::sync::Arc::clone(&remote_shared),
        handle.clone(),
    );

    // Wait for ActiveRemote or error/cancel. Drain ctl for cancel.
    loop {
        while let Ok(update) = remote_state_rx.try_recv() {
            if !remote_shared.is_current(update.attempt_id) {
                continue;
            }
            push_remote_state(
                push,
                &update.state,
                update.host_id.as_deref(),
                update.user.as_deref(),
                update.host.as_deref(),
                update.session_id.as_deref(),
                update.error.as_deref(),
                &update.sessions,
            );
            if update.state == "error" {
                // Stay on the remote host — user can pick another session.
                push_remote_state(
                    push,
                    "ready",
                    Some(&hub_ctx.host_id),
                    Some(&hub_ctx.target.user),
                    Some(&hub_ctx.target.host),
                    None,
                    None,
                    &[],
                );
                *current = None;
                return HostStep::RemoteHub {
                    ctx: Box::new(hub_ctx),
                };
            }
        }
        while let Ok(mut active) = remote_connected_rx.try_recv() {
            if !remote_shared.is_current(active.attempt_id) {
                // Stale attach race: drop the bridge only (daemon stays up).
                handle.block_on(async {
                    crate::app::runtime::stdio_bridge::reap_bridge_child(&mut active.ssh_child)
                        .await;
                });
                continue;
            }
            let sid = match &active.connection.transport {
                super::connect::TransportKind::Remote { session_id, .. }
                | super::connect::TransportKind::Local { session_id } => session_id.clone(),
            };
            return host_remote(
                handle,
                push,
                ctl_tx,
                ctl_rx,
                live_req,
                live_marks,
                live_view,
                push_state,
                current,
                active,
                sid,
                terminal_manager,
                lsp_manager,
            );
        }

        match ctl_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(HostCtl::DisconnectRemote) | Ok(HostCtl::CancelRemoteConnect) => {
                remote_shared.cancel();
                // Cancel mid-attach: keep host ctx, return to hub.
                push_remote_state(
                    push,
                    "ready",
                    Some(&hub_ctx.host_id),
                    Some(&hub_ctx.target.user),
                    Some(&hub_ctx.target.host),
                    None,
                    None,
                    &[],
                );
                *current = None;
                return HostStep::RemoteHub {
                    ctx: Box::new(hub_ctx),
                };
            }
            Ok(HostCtl::SubmitRemotePassword { password }) => {
                remote_shared.submit_password(password);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return HostStep::Done,
            // Ignore other ctls while attaching.
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn host_remote(
    handle: &tokio::runtime::Handle,
    push: &dyn Fn(String),
    ctl_tx: &std::sync::mpsc::Sender<HostCtl>,
    ctl_rx: &std::sync::mpsc::Receiver<HostCtl>,
    live_req: &std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: &std::sync::Arc<std::sync::Mutex<StreamView>>,
    push_state: &mut push_loop::PushState,
    current: &mut Option<String>,
    mut active: super::remote_ctl::ActiveRemote,
    session_id: String,
    terminal_manager: &std::sync::Arc<std::sync::Mutex<super::terminal_host::TerminalManager>>,
    lsp_manager: &std::sync::Arc<std::sync::Mutex<crate::lsp::LspManager>>,
) -> HostStep {
    *current = Some(session_id);
    if let Ok(mut g) = live_req.lock() {
        *g = Some(active.connection.req_tx.clone());
    }
    let prebuffered = std::mem::take(&mut active.connection.prebuffered);
    push_state.reset();
    let transition = {
        let _rt_ctx = handle.enter();
        push_loop::push_loop(
            push,
            &active.connection.frame_rx,
            &active.connection.req_tx,
            prebuffered,
            ctl_tx,
            ctl_rx,
            push_state,
            current.as_deref(),
            live_marks,
            live_view,
            terminal_manager,
            lsp_manager,
            Some(&active.ctx),
            active.fs.as_ref(),
            active.git.as_ref(),
            #[cfg(feature = "linker")]
            active.linker.as_ref(),
        )
    };
    if let Ok(mut g) = live_req.lock() {
        *g = None;
    }
    if let Ok(mut m) = live_marks.lock() {
        m.clear();
    }
    if let Ok(mut v) = live_view.lock() {
        *v = StreamView::default();
    }
    // Flush Detach/QuitDaemon first, then reap the SSH bridge (not the
    // remote session-daemon). Kill the bridge child only if it does not
    // exit after the stdio close — never treat bridge death as session delete.
    super::teardown_connection(handle, active.connection);
    handle.block_on(async {
        crate::app::runtime::stdio_bridge::reap_bridge_child(&mut active.ssh_child).await;
    });
    // Tear down panel thin clients with the same lifetime as the chat bridge.
    if let Some(mut fs) = active.fs.take() {
        fs.shutdown();
    }
    if let Some(mut git) = active.git.take() {
        git.shutdown();
    }
    #[cfg(feature = "linker")]
    if let Some(mut linker) = active.linker.take() {
        linker.shutdown();
    }

    // Keep RemoteCtx unless the transition is a full disconnect / local swapper / exit.
    match transition {
        push_loop::HostTransition::ToRemoteHub { .. } => {
            // push_loop may not yet pass ctx; reconstruct hub from active.ctx.
            push_remote_state(
                push,
                "ready",
                Some(&active.ctx.host_id),
                Some(&active.ctx.target.user),
                Some(&active.ctx.target.host),
                None,
                None,
                &[],
            );
            *current = None;
            HostStep::RemoteHub {
                ctx: Box::new(active.ctx),
            }
        }
        push_loop::HostTransition::RemoteAttach {
            session_id,
            cwd,
            ..
        } => {
            *current = None;
            HostStep::RemoteAttach {
                ctx: Box::new(active.ctx),
                session_id,
                cwd,
            }
        }
        push_loop::HostTransition::Remote {
            connection,
            session_id,
        } => {
            // Another remote attach completed inside push_loop (rare).
            *current = None;
            HostStep::Remote {
                active: connection,
                session_id,
            }
        }
        push_loop::HostTransition::DisconnectRemote
        | push_loop::HostTransition::ToSwapper
        | push_loop::HostTransition::Exit
        | push_loop::HostTransition::Attach { .. } => {
            // Full leave remote: close ControlMaster, drop ctx, clear state.
            crate::remote::ssh::exit_multiplex(&active.ctx.target);
            drop(active.ctx);
            push_remote_state(push, "disconnected", None, None, None, None, None, &[]);
            *current = None;
            transition_to_step(transition)
        }
    }
}

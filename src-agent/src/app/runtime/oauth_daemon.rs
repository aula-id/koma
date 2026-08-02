//! The GLOBAL OAuth keep-alive daemon (`koma --oauth-daemon`).
//!
//! A SINGLETON headless process that proactively refreshes every configured OAuth
//! token on a schedule, keeping them warm so session-daemons always find a fresh
//! token on disk. Unlike the MCP daemon it has NO heavy children (HTTP only) and
//! does NOT self-reap on idle — the user (or `koma update` / `koma daemon kill`)
//! must kill it.
//!
//! # Lifecycle
//!
//! 1. Load config; if `oauth_conns` is empty → exit 0 (nothing to refresh).
//! 2. Seed the in-memory token cache for every refreshable connection.
//! 3. Bind the singleton socket (`~/.koma/oauth.sock`); bind = liveness oracle.
//! 4. Spawn a background refresh task: every 5 minutes, reload config from disk,
//!    and for each refreshable connection call `fresh_key` (which skips when
//!    non-stale). On 2 consecutive reloads with zero `oauth_conns`, exit.
//! 5. Accept loop: serve `OAuthRequest::{Status, Fingerprint, Shutdown}` over
//!    the same 4-byte-BE-len + JSON frame codec as the MCP daemon.
//! 6. Teardown: drop runtime, unlink socket + pidfile.
//!
//! # Differences from the MCP daemon
//!
//! - No idle reaper (stays alive until killed/updated/empty-oauth).
//! - No build-skew fingerprint probe in v1 (optional future).
//! - No heavy child processes (HTTP-only refresh loop).
//! - Background refresh task instead of per-connection dispatch.

use anyhow::Result;

use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::oauth_proto::{OAuthRequest, OAuthResponse};
use crate::model::{app_config::AppConfig, store};
use super::signals::install_daemon_signals;

/// How often the refresh task reloads config and refreshes tokens.
const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Initial settle delay before the first refresh tick. Gives the daemon time to
/// finish startup (bind socket, write pid) before the first network I/O.
const REFRESH_INITIAL_SETTLE: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a single `accept` waits before we re-check the `shutting_down` flag.
const ACCEPT_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Consecutive empty-oauth reloads required before the daemon exits on its own.
const EMPTY_STREAK_TO_EXIT: u32 = 2;

/// Headless entry point: run the GLOBAL OAuth keep-alive daemon event loop.
///
/// Loads the global config, seeds the token cache, binds `~/.koma/oauth.sock`,
/// spawns the background refresh task, and serves [`OAuthRequest`] frames until
/// signalled. Returns when `shutting_down` is set (SIGTERM / double-SIGTERM /
/// `OAuthRequest::Shutdown`).
pub fn run_oauth_daemon(_opts: crate::cli::Opts) -> Result<()> {
    // Ignore SIGPIPE so a broken-pipe write returns EPIPE instead of terminating.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Ensure the config dirs exist.
    store::ensure_dirs()?;

    // Own tokio runtime.
    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Load config. If no OAuth connections, nothing to do.
    let config = AppConfig::load();
    if config.oauth_conns.is_empty() {
        return Ok(());
    }

    // Seed the in-memory token cache for every refreshable connection.
    let refreshable_conns: Vec<_> = config
        .oauth_conns
        .iter()
        .filter(|c| !c.refresh_token.is_empty())
        .cloned()
        .collect();

    if refreshable_conns.is_empty() {
        // No refreshable connections — still bind the socket for liveness probing,
        // but skip the refresh task.
    }

    let handle_clone = handle.clone();
    if !refreshable_conns.is_empty() {
        handle.spawn(refresh_loop(handle_clone, refreshable_conns));
    }

    // Install signal handling.
    let shutting_down = install_daemon_signals(&handle);

    // Write the advisory pidfile.
    let pid_path = store::oauth_daemon_pid_path()?;
    let _ = store::write_oauth_daemon_pid();

    // Bind the singleton socket.
    let sock_path = store::oauth_daemon_sock_path()?;
    let listener = {
        let _enter = handle.enter();
        crate::ipc::server::bind(&sock_path)?
    };

    // Accept loop.
    handle.block_on(accept_loop(listener, &shutting_down));

    // Teardown.
    drop(rt);
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

/// Background refresh loop: reload config every [`REFRESH_INTERVAL`], refresh
/// every stale token via `fresh_key`. Exits when `shutting_down` is set or
/// when the config has had zero `oauth_conns` for [`EMPTY_STREAK_TO_EXIT`]
/// consecutive reloads.
async fn refresh_loop(
    _handle: tokio::runtime::Handle,
    initial_conns: Vec<crate::model::app_config::OAuthConn>,
) {
    // Settle delay before the first tick.
    tokio::time::sleep(REFRESH_INITIAL_SETTLE).await;

    // Start by seeding the initial connections into the cache.
    for conn in &initial_conns {
        crate::service::oauth::manager::seed(conn).await;
    }

    let mut empty_streak: u32 = 0;
    loop {
        // Reload config from disk to pick up any new connections or deletions.
        let config = AppConfig::load();
        let refreshable: Vec<_> = config
            .oauth_conns
            .iter()
            .filter(|c| !c.refresh_token.is_empty())
            .collect();

        if refreshable.is_empty() {
            empty_streak = empty_streak.saturating_add(1);
            if empty_streak >= EMPTY_STREAK_TO_EXIT {
                crate::model::store::append_global_error_log(
                    "oauth",
                    "oauth daemon: no connections for consecutive reloads, exiting",
                );
                // Can't set the shutting_down flag from here (it lives in the accept loop),
                // but we can force-exit. The teardown runs in the main thread. We signal by
                // making the accept loop's accept fail — but since we're on a different
                // thread, just exit the process. The teardown (unlink sock/pid) is
                // best-effort anyway and the next ensure will rebind.
                std::process::exit(0);
            }
        } else {
            empty_streak = 0;

            // Seed any new connections we haven't cached yet.
            for conn in &refreshable {
                crate::service::oauth::manager::seed(conn).await;
            }

            // Try to refresh each stale token. `fresh_key` handles single-flight,
            // staleness check, network refresh, and persist internally.
            let mut refreshed: u64 = 0;
            for conn in &refreshable {
                let (token, _account) =
                    crate::service::oauth::manager::fresh_key(&conn.uuid, &conn.access_token).await;
                if !token.is_empty() {
                    refreshed += 1;
                }
            }

            crate::model::store::append_global_error_log(
                "oauth",
                &format!(
                    "oauth daemon refresh tick: {} connections, {} refreshed",
                    refreshable.len(),
                    refreshed,
                ),
            );
        }

        tokio::time::sleep(REFRESH_INTERVAL).await;
    }
}

/// Accept connections until `shutting_down` is set, spawning a per-connection
/// task for each. Runs on the tokio runtime (async socket I/O).
async fn accept_loop(
    listener: crate::ipc::IpcListener,
    shutting_down: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return;
        }
        match tokio::time::timeout(ACCEPT_POLL, listener.accept()).await {
            Ok(Ok((stream, _addr))) => {
                let flag = std::sync::Arc::clone(shutting_down);
                tokio::spawn(async move {
                    connection_loop(stream, flag).await;
                });
            }
            Err(_elapsed) => {}
            Ok(Err(_e)) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

/// Serve one client connection: read an [`OAuthRequest`] frame, produce its
/// [`OAuthResponse`], write it back, and repeat until the peer closes or a
/// read/decode/write error ends the connection.
async fn connection_loop(
    mut stream: crate::ipc::IpcStream,
    shutting_down: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut reader = FrameReader::new();
    loop {
        let bytes = match read_frame_from(&mut stream, &mut reader).await {
            Ok(b) => b,
            Err(_) => return,
        };
        let req: OAuthRequest = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                let _ = respond(
                    &mut stream,
                    &OAuthResponse::Error(format!("bad request: {e}")),
                )
                .await;
                return;
            }
        };

        let resp = handle_request(req, &shutting_down);
        if respond(&mut stream, &resp).await.is_err() {
            return;
        }
    }
}

/// Serialise + frame-write one [`OAuthResponse`].
async fn respond(
    stream: &mut crate::ipc::IpcStream,
    resp: &OAuthResponse,
) -> std::io::Result<()> {
    let bytes = match serde_json::to_vec(resp) {
        Ok(b) => b,
        Err(e) => serde_json::to_vec(&OAuthResponse::Error(format!("encode failed: {e}")))
            .unwrap_or_else(|_| b"{\"Error\":\"encode failed\"}".to_vec()),
    };
    write_frame_to(stream, &bytes).await
}

/// Produce the [`OAuthResponse`] for one [`OAuthRequest`].
fn handle_request(
    req: OAuthRequest,
    shutting_down: &std::sync::atomic::AtomicBool,
) -> OAuthResponse {
    match req {
        OAuthRequest::Status => {
            let config = AppConfig::load();
            let oauth_connections = config.oauth_conns.len();
            OAuthResponse::Status {
                oauth_connections,
                refreshed_count: 0, // v1: not tracked yet
            }
        }
        OAuthRequest::Fingerprint => {
            OAuthResponse::Fingerprint(store::build_fingerprint())
        }
        OAuthRequest::Shutdown => {
            shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
            OAuthResponse::Ack
        }
    }
}

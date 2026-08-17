//! Headless daemon mode over stdio (`koma server`).
//!
//! Mirrors [`super::lifecycle::run_daemon`] but speaks the IPC protocol over
//! stdin/stdout instead of a unix socket. Designed for SSH remote development:
//! the SSH channel carries bidirectional length-prefixed JSON frames, and the
//! daemon event loop runs identically to the local case.

use anyhow::Result;

use super::event_loop::daemon::{daemon_loop, DaemonHub};
use super::lifecycle::{build_startup, install_daemon_session, shutdown_runtime};
use super::signals::install_daemon_signals;

/// Headless entry point: run the koma-daemon event loop over stdio.
///
/// Shares [`build_startup`] + [`install_daemon_session`] + [`daemon_loop`] with
/// the unix-socket [`super::lifecycle::run_daemon`], but replaces the unix
/// listener + accept loop with a single stdio transport: `tokio::io::stdin` /
/// `tokio::io::stdout` carry the same length-prefixed JSON frames. When the SSH
/// channel closes (stdin EOF), the read loop signals
/// [`HubInbound::Disconnect`](crate::app::runtime::HubInbound), the hub tears
/// down, and [`daemon_loop`] returns for the shared cleanup.
pub fn run_server(opts: crate::cli::Opts) -> Result<()> {
    // Ignore SIGPIPE — same as daemon. A broken-pipe write returns EPIPE
    // (handled per-write) instead of terminating the process.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Auto-generate session id if not provided.
    let session_id = opts
        .session
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    if let Some(cwd) = opts.cwd.as_deref() {
        if cwd.is_empty() || cwd.contains('\0') {
            anyhow::bail!("invalid remote working directory");
        }
        std::env::set_current_dir(cwd)
            .map_err(|e| anyhow::anyhow!("cannot use remote working directory {cwd:?}: {e}"))?;
    }

    // Shared startup — identical to the TUI/daemon path.
    let (rt, handle, mut state, mut client) = build_startup(&opts)?;

    // Own the session.
    install_daemon_session(&mut state, &mut client, &handle, &session_id);

    // MCP proxy setup (same as daemon).
    if !state.rest.config.mcp_servers.is_empty() {
        let proxy = crate::model::store::mcp_daemon_sock_path().and_then(|sock| {
            super::manage::ensure_mcp_daemon_running()
                .and_then(|()| crate::app::mcp::McpManager::connect_proxy(&handle, sock))
        });
        state.rest.mcp_manager = Some(match proxy {
            Ok(proxy) => proxy,
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "mcp",
                    &format!("global daemon unavailable ({e:#}); using local servers"),
                );
                crate::app::mcp::McpManager::connect_all(&handle, &state.rest.config.mcp_servers)
            }
        });
    }

    // OAuth daemon (same as daemon).
    if !state.rest.config.oauth_conns.is_empty() {
        if let Err(e) = super::manage::ensure_oauth_daemon_running() {
            crate::model::store::append_global_error_log(
                "oauth",
                &format!("failed to start OAuth daemon: {e:#}"),
            );
        }
    }

    // Signal handling.
    let shutting_down = install_daemon_signals(&handle);

    // Hub.
    let (mut hub, req_tx) = DaemonHub::new();

    // Take stdin/stdout as the transport. `tokio::io::Stdin` / `Stdout` both
    // implement `AsyncRead` / `AsyncWrite + Unpin + Send + 'static`.
    {
        let _enter = handle.enter();
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        crate::ipc::conn::spawn_stdio(stdin, stdout, 1, req_tx);
    }

    // Enter the headless loop — identical to the daemon path.
    daemon_loop(&mut state, &mut client, &handle, &mut hub, &shutting_down);

    // Graceful teardown — release locks, drop runtime.
    shutdown_runtime(&mut state, rt);

    Ok(())
}

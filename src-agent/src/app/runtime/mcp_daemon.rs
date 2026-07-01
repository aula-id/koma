//! The GLOBAL MCP daemon (`koma --mcp-daemon`).
//!
//! A SINGLETON headless process that owns every configured MCP server connection so
//! that N session-daemons can proxy to it over IPC instead of each spawning their own
//! copies. The user runs heavyweight MCP servers (e.g. `serena`); today every session
//! daemon builds its own [`McpManager`] and therefore spawns its own copy of every
//! server, so N sessions = N copies. This process centralises them: it holds the ONE
//! real [`McpManager`], and session-daemons ask it (via [`crate::ipc::mcp_proto`]) to
//! list/dispatch/reconnect/report.
//!
//! # This commit builds the daemon + its IPC only
//!
//! Nothing wires session-daemons to it yet — they still build their own `McpManager`
//! in `build_startup` (unchanged). The proxy that makes a session-daemon talk to this
//! process instead lands in the NEXT commit. Here we build: the process entry point,
//! the accept + per-connection request loop, and the four request handlers.
//!
//! # Shape mirrors `run_daemon`
//!
//! Startup/teardown deliberately mirror [`super::lifecycle::run_daemon`]: ignore
//! SIGPIPE, install the SIGHUP-survive + graceful/double-SIGTERM signal task, write a
//! pidfile, bind the unix socket (bind = liveness oracle), run an accept loop on the
//! tokio runtime, and on shutdown drop the runtime + unlink the socket/pidfile. The
//! socket/pidfile are the GLOBAL [`store::mcp_daemon_sock_path`] /
//! [`store::mcp_daemon_pid_path`] (`~/.koma/mcp.sock`, `~/.koma/mcp.pid`) — one
//! instance, NOT keyed by any session.
//!
//! # Request loop (not the session daemon's async fan-out bridge)
//!
//! The session daemon splits every connection into independent read/write tasks
//! because it PUSHES unsolicited snapshot/delta frames. The MCP proxy is strictly
//! request→response: a client sends one [`McpRequest`], gets one [`McpResponse`],
//! repeat. So each accepted connection is a single tokio task running a simple
//! read-request → handle → write-response loop over the SAME frame codec, until the
//! peer closes. Because each connection is its own task, two session-daemons dispatch
//! concurrently; and a slow [`McpManager::execute_blocking`] is moved onto
//! [`tokio::task::spawn_blocking`] so it never pegs a reactor worker (and so a slow
//! call on one connection cannot stall the accept loop or the other connections).

use std::sync::Arc;

use anyhow::Result;

use crate::app::mcp::McpManager;
use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::mcp_proto::{McpRequest, McpResponse};
use crate::model::{app_config::AppConfig, store};

use super::signals::install_daemon_signals;

/// Headless entry point: run the GLOBAL MCP daemon event loop with NO terminal.
///
/// Loads the global config, builds the ONE real [`McpManager`] (which owns every
/// enabled MCP server connection), binds `~/.koma/mcp.sock`, and serves
/// [`crate::ipc::mcp_proto`] requests until signalled. Returns when SIGTERM/SIGINT is
/// observed (via the polled `shutting_down` flag); the teardown then drops the runtime
/// (terminating every MCP child) and unlinks the socket + pidfile.
///
/// LIFECYCLE: for THIS commit the daemon just PERSISTS until killed — there is no
/// auto-exit.
// TODO(mcp-lifecycle): the NEXT commit makes this exit once no session-daemons remain
// (e.g. an idle-timeout sweep of the run dir, or a refcount over proxy connections),
// so a torn-down last session doesn't leave the global MCP daemon running forever.
pub fn run_mcp_daemon(_opts: crate::cli::Opts) -> Result<()> {
    // Critique #10 parity with run_daemon: a dead client write must never kill the
    // daemon. Ignore SIGPIPE process-wide BEFORE any socket IO so a broken-pipe write
    // returns EPIPE (handled per-write) instead of terminating the process.
    // SAFETY: `signal` with SIG_IGN on SIGPIPE is async-signal-safe and the canonical
    // way to opt out of SIGPIPE; it touches no Rust state.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Ensure the config dirs exist (so the socket's parent `~/.koma` is present for
    // bind, and any config read has its dir). Mirrors what build_startup does for the
    // session path.
    store::ensure_dirs()?;

    // Own tokio runtime (the MCP connections live on it) + a cloned handle for the
    // manager and the accept loop.
    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Build the ONE real MCP manager from the configured servers. NON-BLOCKING:
    // `connect_all` returns immediately and connects each enabled server in a
    // background task on the handle. This process now OWNS those connections (dropping
    // the manager / the runtime terminates the stdio children).
    let config = AppConfig::load();
    let manager = McpManager::connect_all(&handle, &config.mcp_servers);

    // Install the SIGHUP-survive + graceful/double-SIGTERM signal handling and get the
    // flag the accept loop polls. Done BEFORE binding so a signal during startup is
    // already accounted for. The daemon ignores SIGHUP, so closing the launching
    // terminal can't kill it.
    let shutting_down = install_daemon_signals(&handle);

    // Record the advisory pidfile (diagnostics / `kill`). Best-effort: a write failure
    // must not stop the daemon (the bound socket, not this file, is the liveness
    // oracle). The teardown unlinks it.
    let pid_path = store::mcp_daemon_pid_path()?;
    let _ = store::write_mcp_daemon_pid();

    // Bind the GLOBAL unix socket (this process becomes THE live MCP daemon — bind is
    // the liveness oracle). `crate::ipc::server::bind` unlinks any stale socket first,
    // so a crashed predecessor's socket file doesn't trip `AddrInUse`.
    let sock_path = store::mcp_daemon_sock_path()?;
    let listener = {
        let _enter = handle.enter();
        crate::ipc::server::bind(&sock_path)?
    };

    // Accept loop: block this thread on it, checking `shutting_down` each iteration so
    // a signal flips the flag and ends the loop. `accept` is awaited inside the
    // runtime with a short timeout so the flag is polled even when no client connects.
    handle.block_on(accept_loop(listener, manager, &shutting_down));

    // Graceful teardown: dropping the runtime cancels the accept loop + every
    // per-connection task AND drops the manager's connections (terminating every stdio
    // MCP child). Then unlink the socket + pidfile so the next spawn binds fresh.
    // (A second SIGTERM during this window hard-exits via the signal task instead.)
    drop(rt);
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

/// How long a single `accept` waits before we re-check the `shutting_down` flag. Short
/// so a SIGTERM ends the loop promptly even with no clients connecting.
const ACCEPT_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Accept connections until `shutting_down` is set, spawning a per-connection request
/// task for each. Runs on the tokio runtime (async socket IO). The `manager` is shared
/// (`Arc`) into every connection task so all connections dispatch against the ONE real
/// [`McpManager`].
///
/// A transient `accept` error is logged-by-ignoring and retried after a short sleep —
/// one bad accept must not tear down the daemon's listener.
async fn accept_loop(
    listener: tokio::net::UnixListener,
    manager: Arc<McpManager>,
    shutting_down: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;

    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return;
        }
        // Bound the accept on the poll interval so the shutdown flag is observed even
        // when idle. A timeout is the "no client this tick" path — just loop.
        match tokio::time::timeout(ACCEPT_POLL, listener.accept()).await {
            Ok(Ok((stream, _addr))) => {
                let mgr = Arc::clone(&manager);
                tokio::spawn(async move {
                    connection_loop(stream, mgr).await;
                });
            }
            // Timed out waiting for a connection — re-check the shutdown flag and retry.
            Err(_elapsed) => {}
            // Transient accept failure: don't kill the listener, just try again.
            Ok(Err(_e)) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

/// Serve one client connection: read an [`McpRequest`] frame, produce its
/// [`McpResponse`], write it back, and repeat until the peer closes or a
/// read/decode/write error ends the connection.
///
/// The frames use the SAME 4-byte-BE-len + JSON codec as the session IPC
/// ([`crate::ipc::frame`]). Because the exchange is strictly request→response, a single
/// non-split [`tokio::net::UnixStream`] is borrowed `&mut` for the read then the write
/// within each cycle — no split into independent halves is needed (unlike the session
/// daemon, which pushes unsolicited frames).
async fn connection_loop(mut stream: tokio::net::UnixStream, manager: Arc<McpManager>) {
    let mut reader = FrameReader::new();
    loop {
        // Read one request frame. EOF / decode error / any read error ends the
        // connection (the client closed, or sent garbage — nothing to serve further).
        let bytes = match read_frame_from(&mut stream, &mut reader).await {
            Ok(b) => b,
            Err(_) => return, // peer closed mid-frame / cap violation / IO error
        };
        let req: McpRequest = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            // A malformed frame is a protocol error on this connection; report it and
            // stop rather than guess at intent.
            Err(e) => {
                let _ = respond(&mut stream, &McpResponse::Error(format!("bad request: {e}"))).await;
                return;
            }
        };

        let resp = handle_request(req, &manager).await;
        if respond(&mut stream, &resp).await.is_err() {
            return; // dead socket (EPIPE etc.) — drop the connection
        }
    }
}

/// Serialise + frame-write one [`McpResponse`]. A serialise failure is a daemon bug,
/// not a transport fault; it is surfaced as an in-band [`McpResponse::Error`] so the
/// client still gets a well-formed frame.
async fn respond(stream: &mut tokio::net::UnixStream, resp: &McpResponse) -> std::io::Result<()> {
    let bytes = match serde_json::to_vec(resp) {
        Ok(b) => b,
        Err(e) => serde_json::to_vec(&McpResponse::Error(format!("encode failed: {e}")))
            .unwrap_or_else(|_| b"{\"Error\":\"encode failed\"}".to_vec()),
    };
    write_frame_to(stream, &bytes).await
}

/// Produce the [`McpResponse`] for one [`McpRequest`] by driving the ONE real
/// [`McpManager`].
///
/// - `List` → `Tools { defs, names }` from the manager's advertise accessors.
/// - `Call` → dispatch on [`tokio::task::spawn_blocking`] so a slow tool never pegs a
///   reactor worker (and concurrent calls on different connections stay parallel);
///   the result — success OR the dispatch error text — comes back as `CallResult` so
///   the model sees a tool error as a tool result, exactly like the in-process path.
/// - `Reconnect` → apply the new server set (background) and `Ack`.
/// - `Status` → per-server tool-count map.
async fn handle_request(req: McpRequest, manager: &Arc<McpManager>) -> McpResponse {
    match req {
        McpRequest::List => McpResponse::Tools {
            defs: manager.tool_defs(),
            names: manager.tool_names(),
        },

        McpRequest::Call {
            server_uuid: _server_uuid,
            tool,
            args,
        } => {
            // Parse the args string into the JSON `Value` `execute_blocking` wants. An
            // un-parseable payload is a PROTOCOL fault (the proxy sent malformed args),
            // so it comes back as `Error`, distinct from a tool-level failure.
            let args_value: serde_json::Value = match serde_json::from_str(&args) {
                Ok(v) => v,
                Err(e) => {
                    return McpResponse::Error(format!(
                        "invalid args JSON for MCP tool '{tool}': {e}"
                    ))
                }
            };

            // Dispatch on a blocking pool thread: `execute_blocking` blocks on an
            // `mpsc::recv_timeout` internally, so running it inside `spawn_blocking`
            // keeps it off the reactor and lets concurrent connections' calls proceed
            // in parallel. The manager is cheaply `Arc`-cloned into the closure.
            let mgr = Arc::clone(manager);
            let result = tokio::task::spawn_blocking(move || {
                mgr.execute_blocking(&tool, &args_value)
            })
            .await;

            match result {
                // Both Ok (tool output) and Err (dispatch error text) become the tool
                // RESULT string — the model surfaces the error as the tool's result,
                // mirroring the in-process session tool path. `Error` is reserved for
                // protocol faults only.
                Ok(Ok(output)) => McpResponse::CallResult(output),
                Ok(Err(err_text)) => McpResponse::CallResult(err_text),
                // The blocking task itself panicked/was cancelled — a genuine internal
                // fault, so surface it in-band as the result rather than dropping the
                // connection.
                Err(join_err) => {
                    McpResponse::CallResult(format!("MCP dispatch task failed: {join_err}"))
                }
            }
        }

        McpRequest::Reconnect { servers } => {
            manager.reconnect(&servers);
            McpResponse::Ack
        }

        McpRequest::Status => McpResponse::Status(manager.server_status()),
    }
}

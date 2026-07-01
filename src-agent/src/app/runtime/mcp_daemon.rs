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
//! # Idle self-reap (lifecycle)
//!
//! Session-daemons now PROXY here (each `koma --daemon` connects a `McpManager::Proxy`
//! to this process), so this daemon can outlive the session that first spawned it. To
//! avoid a lingering process holding heavyweight servers (e.g. `serena`) after every
//! session has closed, a background reaper watches [`store::run_dir`] for live
//! session-daemon sockets (`run/*.sock`): once there are NONE for a sustained grace
//! window it flips the same `shutting_down` flag a SIGTERM would, and the normal
//! teardown drops the runtime (terminating every MCP child) + unlinks the socket/pid.
//! It is deliberately biased toward STAYING ALIVE — an initial grace covers a
//! just-spawned session that hasn't bound its socket yet, and it never exits while any
//! `run/*.sock` exists — because a false exit merely means the next session respawns
//! this daemon, whereas a false STAY is harmless.
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
/// LIFECYCLE: the daemon persists until killed OR until the idle-reaper observes no
/// live session-daemon sockets for a sustained grace window (see [`reaper_loop`]),
/// at which point it flips `shutting_down` and tears down like a SIGTERM would.
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

    // Idle self-reap: watch the run dir for live session-daemon sockets and flip the
    // SAME `shutting_down` flag once none remain for a sustained grace window, so an
    // idle global daemon doesn't linger holding heavy servers after every session
    // closed. Spawned on the runtime; it observes the same flag the accept loop polls,
    // so its trip ends the accept loop and the normal teardown runs. Biased toward
    // staying alive (see `reaper_loop`).
    {
        let flag = std::sync::Arc::clone(&shutting_down);
        handle.spawn(reaper_loop(flag));
    }

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

/// How often the idle-reaper scans the run dir for live session-daemon sockets.
const REAPER_POLL: std::time::Duration = std::time::Duration::from_secs(15);

/// Initial grace before the reaper's FIRST scan. Covers the window where the very
/// session that spawned this daemon has not yet bound its `run/<id>.sock` — without
/// it, a daemon spawned by a starting session could see an empty run dir and reap
/// itself out from under that session. Sized to comfortably exceed the session
/// daemon's spawn→bind latency (sub-second in practice; #142's spawn poll budget is
/// 3s).
const REAPER_INITIAL_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// Consecutive empty scans required before the reaper trips. TWO empty polls
/// (≈`REAPER_POLL` apart) means the run dir has held no session sockets for a
/// sustained window, not just a momentary blip between a client detaching and the
/// next attaching — biasing toward staying alive.
const REAPER_EMPTY_STREAK_TO_EXIT: u32 = 2;

/// Watch the run dir for live session-daemon sockets and flip `shutting_down` once
/// none remain for a sustained grace window, so an idle global MCP daemon reaps
/// itself instead of lingering with heavyweight servers loaded.
///
/// Biased HARD toward staying alive:
/// - an initial [`REAPER_INITIAL_GRACE`] delay before the first scan covers a
///   just-spawned session that has not bound its socket yet;
/// - it requires [`REAPER_EMPTY_STREAK_TO_EXIT`] CONSECUTIVE empty scans, so a brief
///   gap while one client detaches and another attaches does not trip it;
/// - "empty" means the run dir contains ZERO `*.sock` FILES (presence, not a connect
///   probe) — while any session socket exists on disk, this daemon never exits, so a
///   momentarily-wedged session still keeps its shared servers alive.
///
/// A false exit is harmless (the next session's `ensure_mcp_daemon_running` respawns
/// this daemon); a false STAY is likewise harmless (a spare idle process). If the run
/// dir can't be read the scan treats it as NON-empty (streak resets) — never a reason
/// to exit. Returns (ending the task) as soon as it trips the flag; the accept loop
/// observes the same flag and the normal teardown runs.
async fn reaper_loop(shutting_down: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;

    // Initial grace: give the spawning session time to bind its socket before the
    // first scan, and bail early if a signal already asked us to stop.
    tokio::time::sleep(REAPER_INITIAL_GRACE).await;

    let mut empty_streak: u32 = 0;
    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return; // a signal (or a prior trip) already latched shutdown
        }

        // Count session sockets by FILE PRESENCE (bias to stay alive: any socket on
        // disk ⇒ not empty). An unreadable run dir is treated as non-empty so a
        // transient stat error never triggers a reap.
        let has_session_socket = run_dir_has_socket();
        if has_session_socket {
            empty_streak = 0;
        } else {
            empty_streak = empty_streak.saturating_add(1);
            if empty_streak >= REAPER_EMPTY_STREAK_TO_EXIT {
                // Sustained-idle: latch shutdown so the accept loop returns and the
                // normal teardown drops the runtime + unlinks the socket/pid.
                shutting_down.store(true, Ordering::Relaxed);
                return;
            }
        }

        tokio::time::sleep(REAPER_POLL).await;
    }
}

/// Whether the run dir currently contains ANY `*.sock` file (a session-daemon
/// rendezvous socket). Presence-only — it does NOT connect-probe, because a socket
/// file existing at all is enough to keep the shared MCP daemon alive (bias toward
/// staying alive). An unreadable/absent run dir returns `true` (treated as "sessions
/// may exist") so a transient error never causes a reap.
fn run_dir_has_socket() -> bool {
    let dir = match store::run_dir() {
        Ok(d) => d,
        // Can't resolve the run dir → assume sessions may exist; never reap on this.
        Err(_) => return true,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // Unreadable/absent run dir: on a genuinely-absent dir there are no sessions,
        // but a transient read error must NOT trigger a reap, so bias to "not empty".
        Err(_) => return true,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sock") {
            return true;
        }
    }
    false
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

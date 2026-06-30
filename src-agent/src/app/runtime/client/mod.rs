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
//! | `input`     | `local_echo`, `is_detach`, `QuitConfirmKey`, quit overlay keys  |
//! | `bridge`    | `reader_task`, `writer_task`, transport consts                  |

#![allow(unused_imports)]
#![allow(dead_code)]

mod connect;
mod render;
mod shadow;
mod input;
mod bridge;

use std::io::stdout;

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::Mode;
use crate::ipc::proto::ClientRequest;
use crate::model::store;

use connect::{connect_attach_and_handshake, Connection};
use bridge::WRITER_FLUSH_TIMEOUT;

use crate::app::runtime::terminal::TerminalGuard;

/// Attach to a running daemon and run the thin render+forward client.
///
/// Connects to the daemon socket (an `Err` means no daemon is up — surfaced to the
/// caller, which prints it), spawns the reader/writer bridge tasks, sends
/// [`ClientRequest::Attach`], runs the build-skew handshake (task #142), then enters
/// the synchronous render loop. Returns when the user detaches (Ctrl-C) or the
/// daemon's socket closes; the terminal is restored by [`TerminalGuard`]'s drop and
/// the runtime is dropped last.
///
/// # Build-skew auto-restart (task #142)
///
/// The koma daemon outlives a rebuild, so a freshly-built client can attach to a
/// daemon still running OLD code and silently render its stale frames (this already
/// caused a phantom `/agents` bug). On connect the client compares its OWN build
/// fingerprint ([`store::build_fingerprint`], computed fresh now) against the
/// daemon's reported one (the `Hello` value, which the daemon captured AT ITS
/// STARTUP). On a mismatch it restarts the stale daemon via the SAME machinery
/// `koma daemon restart` uses ([`super::manage::restart_daemon`]) and reconnects.
///
/// LOOP GUARD: the auto-restart fires AT MOST ONCE per launch. If the freshly-spawned
/// daemon STILL mismatches (it shouldn't — it was just built from the current binary),
/// the client prints an error and renders against it anyway rather than restart-looping
/// forever. A daemon that sends no `Hello` (predates the handshake, or is slow) is
/// never restarted on that absence alone — only a CONFIRMED mismatch triggers a restart.
pub fn client_run(opts: crate::cli::Opts) -> Result<()> {
    // The client needs the config dirs only to resolve the socket path; it owns no
    // sessions and writes no config. In particular it touches NO session lock here
    // or anywhere downstream (lock ownership belongs to the daemon — see the
    // module header): the only `store` calls are these two lock-free path helpers.
    store::ensure_dirs()?;
    let sock_path = store::daemon_sock_path()?;

    // A small multi-thread runtime drives the two socket tasks. The render loop runs
    // on THIS thread (synchronous), exactly like the local TUI's `run_loop`.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // THIS client's build fingerprint, read fresh now (the on-disk binary as it exists
    // at launch). Compared below to each daemon's reported `Hello` to detect a daemon
    // running stale code.
    let my_fingerprint = store::build_fingerprint();

    // Connect + attach + handshake, restarting a version-skewed daemon AT MOST ONCE
    // (the loop guard). On a confirmed mismatch we restart the stale daemon and
    // reconnect; on the (unexpected) second mismatch we give up and render against it.
    let mut conn = connect_attach_and_handshake(&handle, &sock_path)?;
    let mut already_restarted = false;
    while conn
        .daemon_version
        .as_deref()
        .is_some_and(|v| v != my_fingerprint)
    {
        if already_restarted {
            // The just-restarted daemon STILL reports a different fingerprint. This
            // shouldn't happen (it was spawned from the current binary); don't loop
            // forever — warn and render against it.
            eprintln!(
                "koma: daemon still reports a different build after a restart; \
                 continuing against it"
            );
            break;
        }
        eprintln!("koma: daemon running stale code — restarting...");
        already_restarted = true;

        // Tear down the stale connection's bridge before restarting: drop our request
        // sender (the writer drains + exits) and let the reader task observe the
        // daemon's death as EOF. Both old tasks self-terminate; the runtime persists
        // for the reconnect below.
        drop(conn.req_tx);
        drop(conn.frame_rx);

        // Reuse the EXACT `koma daemon restart` path (kill escalation + spawn-and-
        // confirm). A failure here is fatal — we can't recover a usable daemon.
        super::manage::restart_daemon()
            .map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}"))?;

        // Reconnect to the freshly-spawned daemon and re-handshake.
        conn = connect_attach_and_handshake(&handle, &sock_path)?;
    }

    // Unpack the connection we settled on (fresh-built match, an unverified daemon, or
    // a post-restart daemon we chose to accept).
    let Connection {
        frame_rx,
        req_tx,
        writer_handle,
        prebuffered,
        daemon_version: _,
    } = conn;

    // Terminal setup — identical to the local TUI (`run`). Guard first so a failure
    // anywhere after still restores the terminal.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the synchronous render loop with the runtime context entered on THIS thread,
    // SCOPED so the `EnterGuard` is dropped the instant the loop returns — BEFORE the
    // teardown's `handle.block_on` below (which panics if called while a runtime
    // context is entered). The context is needed only so a snapshot rebuild can mint
    // the inert `AbortHandle` a reconstructed shadow `SubAgent` carries (`tokio::spawn`
    // needs a runtime in scope — see `shadow_subagent`); the loop itself stays sync.
    let result = {
        let _rt_ctx = handle.enter();
        render::render_loop(&mut terminal, &frame_rx, &req_tx, prebuffered, opts.resume)
    };

    // Polite detach so the daemon passes the controller seat promptly (the socket
    // close would also trigger it, but this is cleaner). The `/quit` overlay's `[k]`
    // (close-window) and `[d]` (detach) paths ALREADY queued their own `Detach` (and,
    // for `[k]`, a `QuitSession` ahead of it) before the loop returned; this extra
    // `Detach` is then a harmless no-op (the daemon already deregistered this client by
    // id, so a second Detach finds no matching client and returns). For a plain exit
    // (Ctrl-C / EOF) this is the primary detach. All queued requests MUST reach the
    // daemon or it could be left orphaned (socket open, no controller).
    let _ = req_tx.send(ClientRequest::Detach);

    // Deterministic flush of the final frame(s) before the runtime dies. Dropping
    // `req_tx` closes the outbound channel, which the writer observes as
    // `Disconnected`: it then drains EVERY remaining queued request to the socket
    // and returns (see `writer_task`). We must wait for that drain — previously the
    // runtime was dropped immediately, cancelling the writer mid-`poll.tick()` sleep
    // and LOSING the queued `QuitDaemon`/`Detach` (an orphaned daemon). Drop the
    // sender, then JOIN the writer (bounded, so a wedged socket can't hang exit).
    drop(req_tx);
    let _ = handle.block_on(async {
        tokio::time::timeout(WRITER_FLUSH_TIMEOUT, writer_handle).await
    });

    // Writer is done (or the bound elapsed) — its final frames are flushed to the
    // socket. Drop the runtime LAST so the reader task is cancelled after exit.
    drop(rt);

    result
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

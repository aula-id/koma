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
mod swapper;

use std::io::stdout;

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
fn attach_session(handle: &tokio::runtime::Handle, session_id: &str) -> Result<Connection> {
    // Make sure a daemon owns this session before we connect. No-op when it is already
    // live (the bind-as-oracle probe inside short-circuits); spawns + waits otherwise.
    super::manage::ensure_daemon_running(session_id, false)
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
        eprintln!("koma: daemon running stale code — restarting...");
        already_restarted = true;

        // Tear down the stale connection's bridge before restarting: drop our request
        // sender (the writer drains + exits) and let the reader observe the daemon's
        // death as EOF. Both old tasks self-terminate; the runtime persists.
        drop(conn.req_tx);
        drop(conn.frame_rx);

        super::manage::restart_daemon(session_id)
            .map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}"))?;

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
        let conn = attach_session(&handle, &id)?;
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
                        match attach_session(&handle, &new_id) {
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
                SwapperOutcome::Pick(target) => match attach_session(&handle, &target) {
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
                    Some(prev) => match attach_session(&handle, &prev) {
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

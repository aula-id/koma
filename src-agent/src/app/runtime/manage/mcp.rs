//! GLOBAL MCP daemon spawn/ensure (singleton, not session-keyed). Split out
//! of [`super`] (the `manage` module) for file size — pure code motion, no
//! behaviour change. `ensure_mcp_daemon_running` is re-exported from
//! `manage` (`pub use mcp::ensure_mcp_daemon_running;`) so the existing
//! `crate::app::runtime::manage::ensure_mcp_daemon_running` call site
//! (`lifecycle::run_daemon`) keeps resolving unchanged.
//!
//! `probe_or_clear` and `send_signal` are bumped to `pub(super)` in the
//! parent module (they were private) since this file calls them; `stop_mcp_daemon`,
//! `unlink_mcp_daemon_files`, and `read_mcp_pidfile` are bumped to `pub(super)`
//! HERE since `manage::commands` calls them.

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::ipc::SyncIpcStream;
use crate::model::store;

use super::{StopSignal, SIGNAL_GRACE, SPAWN_CONNECT_TIMEOUT, SPAWN_POLL_INTERVAL};

/// Whether the GLOBAL MCP daemon is currently ALIVE, by the same bind-as-oracle rule
/// as [`super::daemon_alive`]: try to CONNECT to its singleton socket
/// ([`store::mcp_daemon_sock_path`], `~/.koma/mcp.sock`). A successful connect proves
/// a real MCP daemon is accepting; refused / not-found proves it is not. The pidfile
/// is never consulted (PID reuse would make it lie). UNLIKE [`super::daemon_alive`] this
/// takes no session id — the MCP daemon is a singleton owning every MCP connection for
/// every session.
#[allow(dead_code)] // consumed by the session-daemon MCP proxy in the next commit
pub fn mcp_daemon_alive() -> bool {
    let Ok(path) = store::mcp_daemon_sock_path() else {
        return false;
    };
    SyncIpcStream::connect(&path).is_ok()
}

/// Spawn a DETACHED `koma --mcp-daemon` child and return its PID.
///
/// The MCP twin of [`super::spawn_daemon`], detached identically (setsid → own session, stdio
/// → `/dev/null`, not `wait`ed so init reaps it). The ONLY difference is the argv: it
/// passes `--mcp-daemon` and NO `--session` (the MCP daemon is a singleton, not keyed
/// to a session). Liveness is still the socket, via [`mcp_daemon_alive`] /
/// the poll-connect in [`spawn_mcp_and_wait_until_alive`].
fn spawn_mcp_daemon() -> Result<u32> {
    // Re-launch THIS binary with `--mcp-daemon`. `current_exe` is the running koma
    // binary, so a renamed/installed binary still respawns itself correctly.
    let exe = std::env::current_exe().context("cannot resolve current executable path")?;

    let mut cmd = Command::new(exe);
    cmd.arg("--mcp-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: `setsid()` is async-signal-safe and the canonical way to detach a child
    // into its own session; it touches no Rust state and only runs in the forked child
    // between fork and exec. A failure is ignored (best-effort detach) — the daemon
    // still runs; it just shares our process group, which the SIGHUP handler tolerates.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    // Windows detach via creation flags — the exact twin of `manage::spawn_daemon`:
    // DETACHED_PROCESS (no console) + CREATE_NEW_PROCESS_GROUP (own group, immune to our
    // Ctrl+C/Break). stdio is already null'd above (the `/dev/null` analogue).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{
            CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        };
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd.spawn().context("failed to spawn `koma --mcp-daemon`")?;
    Ok(child.id())
}

/// Spawn a detached `koma --mcp-daemon` and POLL the bind-as-oracle liveness until it
/// accepts, up to [`SPAWN_CONNECT_TIMEOUT`]. The MCP twin of
/// [`super::spawn_and_wait_until_alive`], keyed to the singleton [`store::mcp_daemon_sock_path`]
/// instead of a per-session socket.
fn spawn_mcp_and_wait_until_alive(path: &Path) -> Result<()> {
    let pid = spawn_mcp_daemon()?;
    let deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    loop {
        match SyncIpcStream::connect(path) {
            Ok(_stream) => return Ok(()), // accepting — probe stream dropped
            Err(_) if Instant::now() < deadline => std::thread::sleep(SPAWN_POLL_INTERVAL),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "spawned MCP daemon (pid {pid}) did not start accepting on {} within {:?}: {e}",
                    path.display(),
                    SPAWN_CONNECT_TIMEOUT
                ));
            }
        }
    }
}

/// Ensure the GLOBAL MCP daemon is RUNNING and accepting on its singleton socket,
/// spawning a detached one if none is up. The MCP twin of [`super::ensure_daemon_running`]:
/// probe [`store::mcp_daemon_sock_path`]; if a daemon is already live, return (a
/// session-daemon proxy attaches to the existing one); otherwise clear any stale
/// socket and spawn a detached `koma --mcp-daemon`, polling until it accepts. Bounded
/// by [`SPAWN_CONNECT_TIMEOUT`]. Takes no session id — one MCP daemon serves every
/// session.
pub fn ensure_mcp_daemon_running() -> Result<()> {
    let path = store::mcp_daemon_sock_path()?;
    if super::probe_or_clear(&path)? {
        return Ok(()); // already live — proxy attaches to the existing one
    }
    // Nothing live → spawn a detached MCP daemon and wait until it accepts.
    spawn_mcp_and_wait_until_alive(&path)
}

/// Read the advisory PID from the GLOBAL MCP daemon's pidfile, if present + parseable.
/// The MCP twin of [`super::read_pidfile`]; used ONLY for messaging and as the `kill` signal
/// target — never for liveness (that is [`mcp_daemon_alive`]'s job).
///
/// `pub(super)` — called from `manage::commands::cmd_status`.
pub(super) fn read_mcp_pidfile() -> Option<u32> {
    let path = store::mcp_daemon_pid_path().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    contents.trim().parse::<u32>().ok()
}

/// Best-effort unlink of the GLOBAL MCP daemon's socket + pidfile. The MCP twin of
/// [`super::unlink_daemon_files`]; a missing file is ignored.
///
/// `pub(super)` — called from `manage::commands::cmd_kill`.
pub(super) fn unlink_mcp_daemon_files() {
    // Unix-only: the socket is a filesystem object. A Windows named pipe is released
    // when its owning process dies, so there is no socket file to unlink here.
    #[cfg(unix)]
    if let Ok(sock) = store::mcp_daemon_sock_path() {
        let _ = std::fs::remove_file(sock);
    }
    if let Ok(pid) = store::mcp_daemon_pid_path() {
        let _ = std::fs::remove_file(pid);
    }
}

/// Poll the GLOBAL MCP daemon's bind-as-oracle liveness until it stops accepting or
/// `timeout` elapses. The MCP twin of [`super::wait_until_dead`], keyed to the singleton
/// socket. Returns `true` if it went down within the window.
fn mcp_wait_until_dead(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !mcp_daemon_alive() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(SPAWN_POLL_INTERVAL);
    }
}

/// Windows graceful MCP-daemon stop: connect the singleton mcp pipe and frame-write a
/// [`crate::ipc::mcp_proto::McpRequest::Shutdown`] — the additive protocol verb whose
/// daemon-side handler flips the SAME `shutting_down` flag a unix `SIGTERM` / the idle
/// reaper set, so the accept loop returns and the normal teardown drops the runtime
/// (terminating every MCP child) + releases the pipe. Windows has no `SIGTERM`, so this
/// IS the graceful stop; [`stop_mcp_daemon`]'s Kill fallback covers a wedged daemon.
///
/// Best-effort + FIRE-AND-FORGET, mirroring [`super::send_shutdown_request`]: it uses the
/// SAME 4-byte-big-endian-length + JSON frame codec as the rest of the IPC and does NOT
/// read the reply (Windows [`SyncIpcStream`] has no read timeout — a blocking drain could
/// hang the CLI). The written frame is enough for the daemon to observe the request;
/// liveness is re-checked by [`mcp_wait_until_dead`] right after.
#[cfg(windows)]
fn send_mcp_shutdown_request() {
    use std::io::Write;

    let Ok(sock) = store::mcp_daemon_sock_path() else {
        return;
    };
    let Ok(mut stream) = SyncIpcStream::connect(&sock) else {
        return;
    };
    let Ok(payload) = serde_json::to_vec(&crate::ipc::mcp_proto::McpRequest::Shutdown) else {
        return;
    };
    let prefix = (payload.len() as u32).to_be_bytes();
    let _ = stream.write_all(&prefix);
    let _ = stream.write_all(&payload);
    let _ = stream.flush();
}

/// Stop the GLOBAL MCP daemon (best-effort), for `koma daemon kill`. The MCP daemon
/// has NO graceful-quit IPC verb (its request protocol is the MCP proxy, not the
/// session control protocol), so — unlike [`super::stop_session_daemon`] — this goes straight
/// to signalling its pidfile PID: SIGTERM, wait, then SIGKILL. Finally unlinks its
/// socket + pidfile. Prints one outcome line and never fails the caller.
///
/// `pub(super)` — called from `manage::commands::cmd_kill`.
pub(super) fn stop_mcp_daemon() {
    if !mcp_daemon_alive() {
        // Sweep any leftover turds from a previous crash so the next start is clean.
        unlink_mcp_daemon_files();
        println!("koma daemon: MCP daemon not running");
        return;
    }

    // Alive but no graceful-quit channel: signal the pidfile PID. If it's missing we
    // can't signal, so just nuke the files.
    let Some(pid) = read_mcp_pidfile() else {
        unlink_mcp_daemon_files();
        println!(
            "koma daemon: MCP daemon still up but no pidfile to signal; removed stale \
             socket/pidfile. If a process is still running, stop it manually."
        );
        return;
    };

    // Graceful terminate, then wait. Unix: SIGTERM (the signal task runs the orderly
    // teardown). Windows has no SIGTERM, so send the `McpRequest::Shutdown` IPC message to
    // the mcp pipe (the daemon flips the SAME `shutting_down` flag a signal / the idle
    // reaper set); best-effort — the Kill fallback below covers a wedged daemon.
    #[cfg(unix)]
    super::send_signal(pid, StopSignal::Term);
    #[cfg(windows)]
    send_mcp_shutdown_request();
    if mcp_wait_until_dead(SIGNAL_GRACE) {
        unlink_mcp_daemon_files();
        println!("koma daemon: stopped MCP daemon (SIGTERM to pid {pid})");
        return;
    }

    // SIGKILL (last resort), then wait.
    super::send_signal(pid, StopSignal::Kill);
    let died = mcp_wait_until_dead(SIGNAL_GRACE);
    unlink_mcp_daemon_files();
    if died {
        println!("koma daemon: killed MCP daemon (SIGKILL to pid {pid})");
    } else {
        println!(
            "koma daemon: sent SIGKILL to pid {pid} (MCP daemon) but the socket is still up; \
             removed socket/pidfile. The process may be unkillable (zombie/stuck IO)."
        );
    }
}

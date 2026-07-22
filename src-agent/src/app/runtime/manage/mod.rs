//! Daemon management CLI + discovery/spawn machinery (`koma daemon …`, #118).
//!
//! This module is the operator-facing control surface for the headless
//! `koma --daemon` process plus the reusable spawn-or-attach mechanism that a
//! later default-launch flip will sit on top of. It deliberately does NOT change
//! the default launch path — `main` still drops into the local TUI by default; the
//! `daemon` subcommand and these functions are the MECHANISM only.
//!
//! # Discovery is bind-as-oracle, NOT a PID check (critique #2)
//!
//! Whether a daemon is "alive" is decided by whether the unix socket
//! ([`store::daemon_sock_path`]) ACCEPTS a connection — never by reading the pidfile
//! and probing `/proc`. PIDs get reused, so a pidfile-driven liveness test could
//! wedge spawn-or-attach into talking to (or trying to kill) an unrelated process.
//! [`daemon_alive`] therefore just tries to `connect`: success means a real daemon
//! is accepting; `ECONNREFUSED`/`ENOENT` means it is not. The pidfile is read ONLY
//! for human-facing messaging and as the LAST-RESORT signal target in `kill`, never
//! as the source of truth for liveness.
//!
//! # Sync, blocking, std-only
//!
//! The management CLI runs BEFORE the TUI and owns no tokio runtime, so all socket
//! I/O here is blocking [`std::os::unix::net::UnixStream`] with read/write timeouts —
//! NOT the async [`crate::ipc::client`] path. The wire codec is the SAME
//! length-prefixed framing the rest of the daemon speaks (4-byte big-endian length +
//! JSON payload); the read side reuses [`crate::ipc::frame::FrameReader`] (pure
//! buffer reassembly, no async) so there is no second hand-rolled framer to drift.
//!
//! # Robustness contract
//!
//! Every one of `status`/`kill`/`restart`/`clean` must work even when the TUI can't
//! start, must never panic, and treats every unlink as best-effort. They print what
//! they did in plain language and return `Ok(())` on a clean outcome.
//!
//! # Module layout
//!
//! Split into themed submodules for file size (pure code motion, no behaviour
//! change): [`mcp`] carries the GLOBAL MCP daemon spawn/ensure; [`commands`]
//! carries the `koma daemon <verb>` subcommand bodies + the stale-file sweep;
//! [`os`] carries the Linux-only `/proc` orphan-process sweep (+ its
//! non-Linux stub); [`probe`] carries the live-session discovery (no-attach
//! socket probe + full sweep). This file keeps the core spawn/discovery/liveness
//! primitives + the sync wire codec every submodule calls back into.

mod commands;
mod doctor;
mod mcp;
mod os;
mod probe;

// Re-exported so the existing `crate::app::runtime::manage::{print_daemon_usage,
// ensure_mcp_daemon_running, list_live_sessions}` paths (used by the `runtime`-level
// re-export chain, `lifecycle::run_daemon`, and the client-side swapper respectively)
// keep resolving unchanged after the split.
pub use commands::print_daemon_usage;
// `run_doctor` is the `koma doctor` entry point (read-only readiness report), re-exported
// so the `crate::app::runtime::manage::run_doctor` path (through `runtime`/`app`, consumed
// by `main`) resolves the same way `print_daemon_usage` does.
pub use doctor::run_doctor;
// `stop_mcp_daemon` is re-exported for the detached extension-uninstall path (bounce the
// global MCP daemon so the next ensure respawns it off the just-saved config).
pub use mcp::{ensure_mcp_daemon_running, stop_mcp_daemon};
// `spawn_into_session` + `SpawnIntoReply` are the extension `sessions.spawn_into`
// cross-process transport (W7), consumed by the grant broker outside this module tree.
// `broadcast_unload_extension` is the extension-uninstall in-memory fan-out (step 3),
// called by both uninstall paths to unload the extension from every OTHER live daemon.
pub use probe::{
    broadcast_unload_extension, list_live_sessions, spawn_into_session, SpawnIntoReply,
};

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use crate::cli::DaemonSub;
use crate::ipc::frame::FrameReader;
use crate::ipc::proto::{ClientRequest, DaemonFrame};
use crate::ipc::SyncIpcStream;
use crate::model::store;

/// How long to wait for a freshly-spawned daemon's socket to start accepting before
/// giving up (the bind + accept-loop spin-up is sub-second in practice).
const SPAWN_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Poll interval while waiting for a spawned daemon's socket to come up.
const SPAWN_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Read/write timeout on the blocking management socket so a wedged daemon can never
/// hang the CLI (e.g. `status` waiting forever for a snapshot that never comes).
const SOCKET_IO_TIMEOUT: Duration = Duration::from_secs(3);

/// How long `kill` waits for a graceful `QuitDaemon` to actually bring the socket
/// down before escalating to signals.
const KILL_GRACE: Duration = Duration::from_secs(3);

/// How long `kill` waits after a SIGTERM (then again after a SIGKILL) for the
/// process to die / the socket to disappear.
const SIGNAL_GRACE: Duration = Duration::from_secs(2);

/// Stop the SESSION-daemon for `session_id`, then spawn a fresh one — the PUBLIC restart
/// primitive the thin client uses to recover from a build-skew (task #142).
///
/// Daemon-per-session: this restarts ONLY the daemon owning `session_id` (the one the
/// client is attached to), not every daemon. It reuses the full
/// graceful→SIGTERM→SIGKILL stop escalation ([`stop_session_daemon`]) AND the
/// [`ensure_daemon_and_connect`] spawn-and-confirm (poll-connect until the new daemon
/// accepts), so the auto-restart never reinvents the kill or the spawn. It returns
/// `Ok(())` once a fresh daemon is confirmed accepting (or surfaces the spawn error);
/// the caller then reconnects to the new daemon.
///
/// When `quiet` is `true`, ALL terminal output (the outcome lines and the stop-phase
/// warning) is suppressed. Pass `true` from any caller that is inside or entering the
/// alt-screen TUI — a lost log line beats a corrupted screen.
pub fn restart_daemon(session_id: &str, quiet: bool) -> Result<()> {
    // Stop whatever owns this session (prints its own outcome). A kill error shouldn't
    // block the restart — surface it but continue to the spawn.
    if let Err(e) = stop_session_daemon(session_id, quiet) {
        if !quiet {
            crate::model::store::append_global_error_log(
                "daemon restart warning",
                &format!("warning during stop phase of restart: {e:#}"),
            );
        }
    }

    // Spawn + wait for it to accept (false: we just killed it). Confirm-only stream.
    let stream = ensure_daemon_and_connect(session_id)
        .context("failed to start the new daemon")?;
    drop(stream); // we only needed to confirm it is accepting

    if !quiet {
        match read_pidfile(session_id) {
            Some(pid) => println!("koma daemon: restarted session {session_id} (pid {pid})"),
            None => println!(
                "koma daemon: restarted session {session_id} (pid unknown — pidfile not yet written)"
            ),
        }
    }
    Ok(())
}

/// Entry point for `koma daemon <verb>` — dispatch to the matching handler.
///
/// Called from `main` (short-circuited BEFORE the TUI). Each handler prints its
/// outcome and returns `Ok(())` on success; an `Err` is surfaced by `main` as a
/// `error: …` line + non-zero exit. None of these touch the terminal, so they work
/// even when the TUI can't start.
pub fn run_daemon_subcommand(sub: DaemonSub) -> Result<()> {
    match sub {
        DaemonSub::Status => commands::cmd_status(),
        DaemonSub::Kill => commands::cmd_kill(),
        DaemonSub::Restart => commands::cmd_restart(),
        DaemonSub::Clean => commands::cmd_clean(),
    }
}

// ─── discovery (bind-as-oracle) ──────────────────────────────────────────────

/// Whether the SESSION-daemon for `session_id` is currently ALIVE, decided by the
/// bind-as-oracle rule (critique #2): try to CONNECT to its keyed socket
/// (`run/<session_id>.sock`). A successful connect proves a real daemon is accepting;
/// `ECONNREFUSED` (stale socket file, nobody listening) or `ENOENT` (no socket at all)
/// proves it is not. The pidfile is NEVER consulted here — PID reuse would make it lie.
pub fn daemon_alive(session_id: &str) -> bool {
    let Ok(path) = store::daemon_sock_path(session_id) else {
        return false;
    };
    SyncIpcStream::connect(&path).is_ok()
}

/// Whether ANY session-daemon is currently alive, by probing every `*.sock` in the run
/// dir (bind-as-oracle per socket). Used by the explicit `--local` guard, which refuses
/// to run a standalone TUI while any daemon owns a session (a second writer would
/// corrupt that session's locks). A missing/empty run dir ⇒ `false`.
pub fn any_daemon_alive() -> bool {
    live_session_sockets()
        .map(|live| !live.is_empty())
        .unwrap_or(false)
}

/// Probe whether ONE socket file currently accepts a connection (bind-as-oracle for a
/// specific path). `true` only on a successful connect; any error (refused / not-found /
/// permissions) reads as not-live.
fn sock_path_alive(path: &Path) -> bool {
    SyncIpcStream::connect(path).is_ok()
}

/// Enumerate the session sockets under [`store::run_dir`], returning each `*.sock`
/// path paired with whether a daemon is currently accepting on it.
///
/// Best-effort: an unreadable/absent run dir yields an empty list (no daemons), never an
/// error from the directory walk itself — only [`store::run_dir`] resolution can fail.
/// The `session_id` is the socket file stem (`<id>.sock`), the same key the daemon was
/// spawned with. Drives the `koma daemon …` admin verbs (which now act over ALL
/// sessions) without consulting any pidfile for liveness.
///
/// `pub(super)` — called from `manage::commands::{cmd_clean, sweep_stale_files}`.
#[cfg(unix)]
pub(super) fn list_session_sockets() -> Result<Vec<(String, std::path::PathBuf, bool)>> {
    let dir = store::run_dir()?;
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // No run dir yet ⇒ no daemons. Not an error for the admin verbs.
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sock") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let alive = sock_path_alive(&path);
        out.push((id.to_string(), path, alive));
    }
    Ok(out)
}

/// Windows twin of the unix [`list_session_sockets`] above.
///
/// There is no `run_dir` to scan for `*.sock` files here (a named pipe is not a
/// filesystem object), so session ids instead come from
/// [`store::list_koma_session_pipes`] — the pipe-namespace enumeration. Each id's
/// pipe path is rebuilt via [`store::daemon_sock_path`] and probed with the SAME
/// bind-as-oracle connect-probe ([`sock_path_alive`]) the unix arm uses, so a
/// dead-but-still-visible pipe name (a wedged/crashed daemon whose pipe handle
/// hasn't fully released) is verified exactly like a stale unix socket file is —
/// presence in the namespace is only a CANDIDATE, never assumed alive on its own.
/// Returns the same `(session_id, path, alive)` shape as the unix arm, so every
/// downstream consumer (`live_session_sockets`, `any_daemon_alive`,
/// `list_live_sessions`, the `cmd_status`/`kill`/`restart`/`clean` verbs) needs no
/// platform split of its own.
#[cfg(windows)]
pub(super) fn list_session_sockets() -> Result<Vec<(String, std::path::PathBuf, bool)>> {
    let mut out = Vec::new();
    for id in store::list_koma_session_pipes() {
        let Ok(path) = store::daemon_sock_path(&id) else {
            continue;
        };
        let alive = sock_path_alive(&path);
        out.push((id, path, alive));
    }
    Ok(out)
}

/// The subset of [`list_session_sockets`] whose daemon is currently ACCEPTING, as
/// `(session_id, socket_path)` pairs. Stale sockets (dead daemon) are dropped.
///
/// `pub(super)` — called from `manage::commands::{cmd_status, cmd_kill, cmd_restart}`.
pub(super) fn live_session_sockets() -> Result<Vec<(String, std::path::PathBuf)>> {
    Ok(list_session_sockets()?
        .into_iter()
        .filter_map(|(id, path, alive)| alive.then_some((id, path)))
        .collect())
}

/// Spawn a DETACHED `koma --daemon` child and return its PID.
///
/// The child is fully detached so it survives this short-lived CLI process:
/// - `pre_exec(setsid)` puts it in its own session (no controlling terminal), so a
///   closed terminal can't SIGHUP it and it is not in our process group.
/// - stdio is redirected to `/dev/null` (the daemon is headless; it must not write to
///   our terminal or hold our fds open).
///
/// We do NOT `wait()` on the child: this CLI exits almost immediately, at which point
/// the now-orphaned daemon is reparented to and reaped by init — so it never lingers
/// as a zombie. The returned PID is advisory (for messaging); liveness is still the
/// socket, via [`daemon_alive`] / the poll-connect in [`ensure_daemon_and_connect`].
///
/// When `resume` is `true`, the `--resume` flag is forwarded to the spawned daemon
/// so `build_startup()` opens the session picker instead of eagerly creating a
/// session.
///
/// `session_id` is passed through as `--daemon --session <id>`: the daemon binds the
/// matching keyed socket (`run/<id>.sock`) and create-or-loads exactly that session, so
/// the spawning client (which minted `id`) and the daemon agree on the session/socket.
fn spawn_daemon(session_id: &str, resume: bool, workdir: Option<&Path>) -> Result<u32> {
    // Re-launch THIS binary with `--daemon`. `current_exe` is the running koma binary,
    // so a renamed/installed binary still respawns itself correctly.
    let exe = std::env::current_exe().context("cannot resolve current executable path")?;

    let mut cmd = Command::new(exe);
    cmd.arg("--daemon")
        .arg("--session")
        .arg(session_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if resume {
        cmd.arg("--resume");
    }
    // When the caller picked a folder (GUI "+ New session" native picker), spawn the
    // daemon WITH that folder as its cwd so `install_daemon_session`'s `current_dir()`
    // buckets the new session's workspace there. `None` inherits our cwd = old behavior.
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }

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

    // Windows has no `pre_exec`/`setsid`. Detach via creation flags instead:
    // DETACHED_PROCESS gives the daemon no console (headless — a closing terminal can't
    // SIGHUP it), and CREATE_NEW_PROCESS_GROUP roots it in its own process group so a
    // Ctrl+C/Ctrl+Break delivered to OUR group never reaches it. stdio is already
    // null'd above, matching the unix `/dev/null` redirect. Best-effort like `setsid`:
    // even if a flag is ignored the daemon still runs, just less isolated.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{
            CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        };
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd.spawn().context("failed to spawn `koma --daemon`")?;
    Ok(child.id())
}

/// Probe the daemon socket and, if nothing is accepting, clear any stale socket file
/// so a fresh `bind` won't trip over `AddrInUse`.
///
/// Returns `Ok(true)` when a daemon is ALREADY live (the caller should NOT spawn),
/// `Ok(false)` when nothing is accepting and the path is now clear to spawn into, and
/// `Err` on an unexpected probe failure (permissions, etc.) — in which case the caller
/// must NOT blindly spawn a second daemon on top of an unknown condition. Logic:
/// - connect succeeds → a daemon is live (`Ok(true)`).
/// - `ECONNREFUSED` with a socket file still present → a CRASHED daemon left a stale
///   socket; unlink it (best-effort) and report not-live (`Ok(false)`).
/// - `ENOENT` (no socket) → nothing running (`Ok(false)`).
///
/// `pub(super)` — called from `manage::mcp::ensure_mcp_daemon_running`.
pub(super) fn probe_or_clear(path: &Path) -> Result<bool> {
    match SyncIpcStream::connect(path) {
        Ok(_stream) => Ok(true), // a daemon is already live (probe stream dropped)
        Err(e) => match e.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                // Stale socket from a crashed daemon: remove it so the spawn's bind
                // doesn't trip over `AddrInUse`. Best-effort (it may have just gone).
                // Unix-only — a Windows named pipe has no stale file to unlink (a dead
                // daemon's pipe is already gone, surfacing as `NotFound` below, not
                // `ConnectionRefused`).
                #[cfg(unix)]
                let _ = std::fs::remove_file(path);
                Ok(false)
            }
            // No socket at all — nothing running. Clear to spawn.
            std::io::ErrorKind::NotFound => Ok(false),
            // Any other error (permissions, etc.): surface it rather than blindly
            // spawning a second daemon on top of an unknown condition.
            _ => Err(anyhow!("cannot probe daemon socket {}: {e}", path.display())),
        },
    }
}

/// Ensure a koma daemon is RUNNING and accepting on the socket, spawning a detached
/// one if none is up. Returns once a daemon is confirmed accepting; does NOT return a
/// stream (the caller connects itself — e.g. the thin client opens its own async
/// connection right after).
///
/// This is the default-launch primitive (`koma` with no flags = ensure-then-attach):
/// 1. Probe the socket. Live already → return `Ok(())` (attach to the existing one).
/// 2. Not live → (stale socket cleared by [`probe_or_clear`]) spawn a detached
///    `koma --daemon`, then POLL the bind-as-oracle liveness up to
///    [`SPAWN_CONNECT_TIMEOUT`] until it accepts — or return a clear error if it never
///    came up (the default path turns that into "could not start the koma daemon …
///    try `koma --local`", NEVER a silent fallback to a local TUI).
///
/// Bounded by the spawn timeout, so it can never hang forever waiting on a daemon that
/// fails to come up.
///
/// When `resume` is `true`, the spawned daemon receives `--resume` so
/// `build_startup()` opens the session picker instead of eagerly creating a session.
///
/// `session_id` keys the whole operation: the probe + spawn target the session's keyed
/// socket (`run/<session_id>.sock`), and the spawned daemon is told `--session
/// <session_id>` so it owns exactly that session. For a fresh `koma` the id was just
/// minted (so the socket never exists yet and this always spawns); the probe branch
/// still matters for a resume that re-targets an already-live session-daemon later.
pub fn ensure_daemon_running(session_id: &str, resume: bool, workdir: Option<&Path>) -> Result<()> {
    let path = store::daemon_sock_path(session_id)?;
    if probe_or_clear(&path)? {
        return Ok(()); // already live — attach to the existing one
    }
    // Nothing live → spawn a detached daemon and wait until it accepts.
    spawn_and_wait_until_alive(session_id, &path, resume, workdir)
}

/// Spawn-or-attach: return a connected blocking [`SyncIpcStream`] to a LIVE daemon,
/// spawning one first if none is up.
///
/// The blocking-stream variant of [`ensure_daemon_running`], used by `koma daemon
/// restart` to CONFIRM the freshly-spawned daemon is accepting (it drops the stream
/// immediately). Logic:
/// 1. Try to connect. Success → a daemon is live; return the stream.
/// 2. `ECONNREFUSED` with a socket file still present → a CRASHED daemon left a stale
///    socket (bind would fail with `AddrInUse` until it's gone). Unlink it, then spawn.
/// 3. `ENOENT` (no socket) → nothing is running; spawn.
/// 4. After spawning, POLL-connect up to [`SPAWN_CONNECT_TIMEOUT`] until the new
///    daemon's accept loop is up, returning the connected stream — or a clear error if
///    it never came up.
///
/// Note: the daemon's own `server::bind` ALSO unlinks a stale socket before binding,
/// so step 2's unlink is belt-and-suspenders; doing it here too keeps the contract
/// explicit and avoids racing a bind that hasn't happened yet.
///
/// `session_id` keys the socket + the spawned daemon's `--session`, so this confirms a
/// live session-daemon for exactly that session.
pub fn ensure_daemon_and_connect(session_id: &str) -> Result<SyncIpcStream> {
    let path = store::daemon_sock_path(session_id)?;

    if probe_or_clear(&path)? {
        // Already live — reconnect for the caller (the probe stream was dropped).
        return SyncIpcStream::connect(&path)
            .with_context(|| format!("connect to live daemon socket {}", path.display()));
    }

    // Nothing live → spawn + wait until it accepts, then connect for the caller.
    spawn_and_wait_until_alive(session_id, &path, false, None)?;
    SyncIpcStream::connect(&path)
        .with_context(|| format!("connect to spawned daemon socket {}", path.display()))
}

/// Spawn a detached `koma --daemon` and POLL the bind-as-oracle liveness until it
/// accepts, up to [`SPAWN_CONNECT_TIMEOUT`]. Returns `Ok(())` once the socket accepts,
/// or a clear `Err` (naming the advisory PID + the timeout) if it never came up.
///
/// Shared by [`ensure_daemon_running`] (default launch) and
/// [`ensure_daemon_and_connect`] (restart): both need "spawn, then wait until alive";
/// only the latter additionally returns a connected stream. The wait is the SAME
/// connect-probe as [`daemon_alive`], so "alive" means exactly "the socket accepts".
fn spawn_and_wait_until_alive(
    session_id: &str,
    path: &Path,
    resume: bool,
    workdir: Option<&Path>,
) -> Result<()> {
    let pid = spawn_daemon(session_id, resume, workdir)?;
    let deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    loop {
        match SyncIpcStream::connect(path) {
            Ok(_stream) => return Ok(()), // accepting — probe stream dropped
            Err(_) if Instant::now() < deadline => std::thread::sleep(SPAWN_POLL_INTERVAL),
            Err(e) => {
                return Err(anyhow!(
                    "spawned daemon (pid {pid}) did not start accepting on {} within {:?}: {e}",
                    path.display(),
                    SPAWN_CONNECT_TIMEOUT
                ));
            }
        }
    }
}

// ─── blocking framed request/reply ───────────────────────────────────────────

/// Send one [`ClientRequest`] on `stream` as a length-prefixed JSON frame (4-byte
/// big-endian length + payload — the SAME wire codec as [`crate::ipc::frame`]).
///
/// `pub(super)` — called from `manage::commands::daemon_session_count`.
pub(super) fn send_request(stream: &mut SyncIpcStream, req: &ClientRequest) -> Result<()> {
    let payload = serde_json::to_vec(req).context("serialise ClientRequest")?;
    let prefix = (payload.len() as u32).to_be_bytes();
    stream.write_all(&prefix).context("write frame prefix")?;
    stream.write_all(&payload).context("write frame payload")?;
    stream.flush().context("flush frame")?;
    Ok(())
}

/// Block until ONE complete [`DaemonFrame`] arrives on `stream`, reassembling via the
/// shared [`FrameReader`] (so a frame split across reads — or coalesced with the next —
/// is handled identically to the async path). The stream's read timeout bounds the
/// wait so a wedged daemon can't hang the CLI.
///
/// `pub(super)` — called from `manage::commands::daemon_session_count`.
pub(super) fn recv_frame(stream: &mut SyncIpcStream, reader: &mut FrameReader) -> Result<DaemonFrame> {
    loop {
        // A previous read may have buffered a whole frame already.
        if let Some(bytes) = reader.next_frame().context("frame reassembly")? {
            return serde_json::from_slice(&bytes).context("decode DaemonFrame");
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).context("read from daemon socket")?;
        if n == 0 {
            return Err(anyhow!("daemon closed the connection mid-frame"));
        }
        reader.push(&chunk[..n]);
    }
}

/// Connect to the live daemon with the blocking management socket, applying the I/O
/// timeouts. Returns an error if no daemon is accepting (the bind-as-oracle signal).
///
/// `pub(super)` — called from `manage::commands::daemon_session_count`.
pub(super) fn connect_managed(path: &Path) -> Result<(SyncIpcStream, FrameReader)> {
    let stream = SyncIpcStream::connect(path)
        .with_context(|| format!("connect to daemon socket {}", path.display()))?;
    // Bound every blocking read/write so a stuck daemon can't wedge the CLI.
    stream
        .set_read_timeout(Some(SOCKET_IO_TIMEOUT))
        .context("set socket read timeout")?;
    stream
        .set_write_timeout(Some(SOCKET_IO_TIMEOUT))
        .context("set socket write timeout")?;
    Ok((stream, FrameReader::new()))
}

/// Read the advisory PID from the pidfile for `session_id`, if present and parseable.
/// Used ONLY for human-facing messaging and as the last-resort `kill` target — NEVER for
/// liveness (that is the socket's job, per the bind-as-oracle rule).
///
/// `pub(super)` — called from `manage::commands::cmd_status`.
pub(super) fn read_pidfile(session_id: &str) -> Option<u32> {
    let path = store::daemon_pid_path(session_id).ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    contents.trim().parse::<u32>().ok()
}

/// Best-effort unlink of one session's socket + pidfile (the "turds" a crash can leave).
/// Each removal ignores a missing file; any other IO error is swallowed — these are
/// cleanup, never a hard failure.
fn unlink_daemon_files(session_id: &str) {
    // Unix-only: the socket is a filesystem object. A Windows named pipe is released
    // when its owning process dies, so there is no socket file to unlink here.
    #[cfg(unix)]
    if let Ok(sock) = store::daemon_sock_path(session_id) {
        let _ = std::fs::remove_file(sock);
    }
    if let Ok(pid) = store::daemon_pid_path(session_id) {
        let _ = std::fs::remove_file(pid);
    }
}

/// Stop the SESSION-daemon owning `session_id`, escalating only if it won't go. The
/// per-session stop PRIMITIVE shared by `koma daemon kill` (looped over every live
/// session) and [`restart_daemon`]. Prints exactly one outcome line tagged with the
/// session id, and is best-effort throughout (it never fails the caller).
///
/// 1. Not alive: sweep any stale socket/pidfile for this session and report.
/// 2. Alive: connect + send `QuitDaemon` (graceful — the daemon releases its lock,
///    unlinks its own socket/pidfile, exits). Wait up to [`KILL_GRACE`].
/// 3. Still up: fall back to this session's pidfile PID — SIGTERM, wait, then SIGKILL.
/// 4. Finally unlink this session's socket + pidfile if present, and report.
///
/// When `quiet` is `true`, ALL terminal output (`println!`/`eprintln!`) is suppressed.
/// Pass `true` from any caller that is inside or entering the alt-screen TUI — a lost
/// log line beats a corrupted screen.
///
/// `pub(super)` — called from `manage::commands::cmd_kill`.
pub(super) fn stop_session_daemon(session_id: &str, quiet: bool) -> Result<()> {
    if !daemon_alive(session_id) {
        // Sweep any leftover turds from a previous crash so the next start is clean.
        unlink_daemon_files(session_id);
        if !quiet {
            println!("koma daemon: session {session_id} not running");
        }
        return Ok(());
    }

    let sock = store::daemon_sock_path(session_id)?;

    // --- graceful: QuitDaemon ---
    // A connect/send failure here is non-fatal: it just means we go straight to the
    // signal fallback below (the daemon may have died between the liveness check and
    // now, or wedged its accept loop).
    let graceful_sent = match connect_managed(&sock) {
        Ok((mut stream, mut reader)) => {
            if send_request(&mut stream, &ClientRequest::QuitDaemon).is_ok() {
                // Best-effort: drain a couple of frames so the Ack is consumed (and the
                // daemon sees our read side stay open until it tears down). Ignore errors.
                for _ in 0..4 {
                    if recv_frame(&mut stream, &mut reader).is_err() {
                        break; // socket closed (daemon tearing down) — expected
                    }
                }
                true
            } else {
                false
            }
        }
        Err(_) => false,
    };

    if graceful_sent && wait_until_dead(session_id, KILL_GRACE) {
        // The daemon's own teardown unlinks the socket + pidfile; sweep defensively in
        // case it didn't get that far, then report.
        unlink_daemon_files(session_id);
        if !quiet {
            println!("koma daemon: stopped session {session_id} (graceful QuitDaemon)");
        }
        return Ok(());
    }

    // --- fallback: signal the pidfile PID ---
    // Bind-as-oracle says it's still accepting (or graceful failed). Use the pidfile
    // ONLY as the signal target — if it's missing we can't signal, so just nuke files.
    let Some(pid) = read_pidfile(session_id) else {
        unlink_daemon_files(session_id);
        if !quiet {
            println!(
                "koma daemon: session {session_id} still up but no pidfile to signal; removed \
                 stale socket/pidfile. If a daemon is still running, stop it manually."
            );
        }
        return Ok(());
    };

    // Graceful terminate, then wait. Unix: SIGTERM. Windows has no SIGTERM, so re-send
    // the QuitDaemon IPC message to the session's pipe (the daemon latches the SAME
    // shutdown flag SIGTERM flips); best-effort — the Kill fallback below covers a
    // wedged/attached daemon that rejects or ignores it.
    #[cfg(unix)]
    send_signal(pid, StopSignal::Term);
    #[cfg(windows)]
    send_shutdown_request(&sock);
    if wait_until_dead(session_id, SIGNAL_GRACE) {
        unlink_daemon_files(session_id);
        if !quiet {
            println!("koma daemon: stopped session {session_id} (SIGTERM to pid {pid})");
        }
        return Ok(());
    }

    // SIGKILL (last resort), then wait.
    send_signal(pid, StopSignal::Kill);
    let died = wait_until_dead(session_id, SIGNAL_GRACE);
    unlink_daemon_files(session_id);
    if !quiet {
        if died {
            println!("koma daemon: killed session {session_id} (SIGKILL to pid {pid})");
        } else {
            println!(
                "koma daemon: sent SIGKILL to pid {pid} (session {session_id}) but the socket is \
                 still up; removed socket/pidfile. The process may be unkillable (zombie/stuck IO)."
            );
        }
    }
    Ok(())
}

/// SILENTLY kill the SESSION-daemon owning `session_id`, escalating until it is dead, and
/// return whether it is gone afterwards. The alt-screen-safe / TTY-less kill PRIMITIVE
/// shared by the client-side swapper's `Ctrl+X` nuke AND the GUI host-relay's session
/// lifecycle (KillSession / New-with-kill / delete guard).
///
/// Both callers run in a context that must not print — the swapper owns the alternate
/// screen (a stray `println!` smears the picker) and the GUI host owns no TTY at all — so
/// this wraps [`stop_session_daemon`] with `quiet = true`, reusing its FULL escalation
/// (graceful `QuitDaemon` → [`wait_until_dead`] → SIGTERM → wait → SIGKILL) rather than the
/// old graceful-only fire-and-forget reap, which returned BEFORE the daemon died (so an
/// immediate discovery sweep still saw the dying socket answering) and could never remove a
/// wedged daemon at all. It BLOCKS until the daemon is dead or the escalation budget is
/// spent (up to [`KILL_GRACE`] + two [`SIGNAL_GRACE`] windows), so a caller that can't stall
/// (the GUI fold loop, the swapper's input loop) must run it OFF-thread.
///
/// Returns `true` when the keyed socket no longer accepts (dead), `false` if it somehow
/// survived every stage — letting a caller refresh its view only once death is confirmed.
/// Best-effort throughout (it never fails the caller): a dead/unreachable daemon is an
/// immediate `true`, and every I/O error inside `stop_session_daemon` is already swallowed.
pub(crate) fn kill_session_daemon(session_id: &str) -> bool {
    // Reuse the full graceful→SIGTERM→SIGKILL stop escalation, silenced (a lost log line
    // beats a corrupted alt-screen / a TTY-less host). The `Result` is always `Ok` in
    // practice (the stop path is best-effort); liveness is the real signal, so re-probe it.
    let _ = stop_session_daemon(session_id, true);
    !daemon_alive(session_id)
}

// ─── legacy-daemon migration ──────────────────────────────────────────────────

/// Reap a pre-0.2.0 global daemon left over from an upgrade, if one exists.
///
/// Pre-0.2.0 koma ran a single global daemon that bound `<base_dir>/daemon.sock`
/// and recorded its PID in `<base_dir>/daemon.pid`. 0.2.0 switched to
/// daemon-per-session (`run/<id>.sock`); it NEVER writes a bare `daemon.sock`, so
/// the presence of that file is unambiguous proof of a pre-0.2.0 leftover.
///
/// Behavior (entirely best-effort — never panics, never blocks startup):
/// - If neither `daemon.sock` nor `daemon.pid` exists → return immediately.
/// - Read and parse `daemon.pid` → SIGTERM the old daemon (it has a graceful SIGTERM
///   handler that releases locks and unlinks its own files). Poll up to ~1 s for the
///   socket to disappear.
/// - Unlink both files regardless (the old daemon may have already done so).
/// - Print ONE line to stderr only when something was actually reaped; silent otherwise.
pub fn migrate_legacy_daemon() {
    let base = match crate::model::store::base_dir() {
        Ok(b) => b,
        Err(_) => return,
    };
    let legacy_sock = base.join("daemon.sock");
    let legacy_pid  = base.join("daemon.pid");

    // Fast-path: nothing to do (the common case on 0.2.0-only installs).
    if !legacy_sock.exists() && !legacy_pid.exists() {
        return;
    }

    // Try to signal the old daemon via its pidfile.
    let signalled = legacy_pid
        .exists()
        .then(|| std::fs::read_to_string(&legacy_pid).ok())
        .flatten()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .inspect(|&pid| {
            send_signal(pid, StopSignal::Term);
        });

    // Poll up to ~1 s for the socket to disappear (the old daemon's SIGTERM handler
    // unlinks it). Non-fatal if it lingers — we'll unlink it below anyway.
    if signalled.is_some() && legacy_sock.exists() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while legacy_sock.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Best-effort cleanup: unlink whatever survived.
    let _ = std::fs::remove_file(&legacy_sock);
    let _ = std::fs::remove_file(&legacy_pid);

    crate::model::store::append_global_error_log(
        "legacy daemon reaped",
        "reaped a pre-0.2.0 daemon (upgrade cleanup)",
    );
}

// ─── signal + wait helpers ───────────────────────────────────────────────────

/// Platform-neutral stop signal for [`send_signal`]. Unix maps each variant to the
/// matching libc signal number; Windows has no equivalent yet (TODO below).
pub(super) enum StopSignal {
    /// Graceful terminate request (unix: `SIGTERM`).
    Term,
    /// Forceful kill (unix: `SIGKILL`).
    Kill,
}

/// Send `sig` to `pid`, best-effort. A failure (ESRCH = already gone, EPERM = not
/// ours) is ignored — `kill` re-checks liveness via the socket afterwards, so a
/// failed signal just means the follow-up `wait_until_dead` decides the outcome.
///
/// `pub(super)` — called from `manage::mcp::stop_mcp_daemon` and
/// `manage::os::kill_orphan_daemon_processes`.
#[cfg(unix)]
pub(super) fn send_signal(pid: u32, sig: StopSignal) {
    let sig = match sig {
        StopSignal::Term => libc::SIGTERM,
        StopSignal::Kill => libc::SIGKILL,
    };
    // SAFETY: `kill(2)` with a real signal number has no memory-safety preconditions
    // and the FFI types match libc's signature. We intentionally ignore the result.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

/// Windows `send_signal`. There is no `kill(2)`, so only the two expressible-from-a-
/// bare-pid outcomes live here:
/// - [`StopSignal::Kill`] → `OpenProcess(PROCESS_TERMINATE)` + `TerminateProcess` (the
///   `SIGKILL` analogue), via [`crate::model::proc_win::terminate_process`].
/// - [`StopSignal::Term`] → a DELIBERATE no-op. A graceful terminate on Windows is an IPC
///   message (`QuitDaemon` / `McpRequest::Shutdown`) that needs the daemon's PIPE PATH
///   (not just a pid) and is protocol-specific, so it is sent by [`send_shutdown_request`]
///   (session) / `mcp::send_mcp_shutdown_request` (mcp) at the call sites that hold the
///   path. From a bare pid there is nothing graceful to do; the caller escalates to
///   `Kill` if the daemon does not stop (the legacy-daemon sweep, which has no live
///   Windows target, relies on exactly this no-op).
#[cfg(windows)]
pub(super) fn send_signal(pid: u32, sig: StopSignal) {
    match sig {
        StopSignal::Kill => crate::model::proc_win::terminate_process(pid),
        StopSignal::Term => {}
    }
}

/// Windows graceful session-daemon stop: connect the session's named pipe and send the
/// SAME [`ClientRequest::QuitDaemon`] a controller sends. The daemon latches the SAME
/// shutdown flag a unix `SIGTERM` flips, then tears down normally (release locks, drop
/// runtime, release the pipe). Windows has no `SIGTERM`, so this IS the graceful stop;
/// [`stop_session_daemon`]'s Kill fallback ([`send_signal`] → `TerminateProcess`, made
/// tree-safe by the daemon's Job Object) covers a wedged or client-attached daemon that
/// rejects the request.
///
/// Best-effort + FIRE-AND-FORGET: any connect/send error just returns (the caller
/// escalates). It intentionally does NOT read the reply — [`SyncIpcStream`] has no read
/// timeout on Windows, so a blocking drain could hang the CLI against a wedged daemon;
/// the written frame is enough for the daemon to observe the request, and liveness is
/// re-checked by [`wait_until_dead`] right after.
#[cfg(windows)]
pub(super) fn send_shutdown_request(sock: &Path) {
    if let Ok(mut stream) = SyncIpcStream::connect(sock) {
        let _ = send_request(&mut stream, &ClientRequest::QuitDaemon);
    }
}

/// Poll the bind-as-oracle liveness of the SESSION-daemon for `session_id` until it stops
/// accepting or `timeout` elapses. Returns `true` if it went down within the window,
/// `false` if it is still accepting when time ran out. Uses the SAME connect-probe as
/// [`daemon_alive`], so "dead" here means exactly "the keyed socket no longer accepts".
fn wait_until_dead(session_id: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !daemon_alive(session_id) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(SPAWN_POLL_INTERVAL);
    }
}

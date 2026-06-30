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

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use crate::cli::DaemonSub;
use crate::ipc::frame::FrameReader;
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame};
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
/// the caller then reconnects to the new daemon. It prints a couple of plain status
/// lines, which is harmless here — the client's handshake runs BEFORE the alt-screen is
/// entered, so the lines scroll away when the TUI opens.
pub fn restart_daemon(session_id: &str) -> Result<()> {
    // Stop whatever owns this session (prints its own outcome). A kill error shouldn't
    // block the restart — surface it but continue to the spawn.
    if let Err(e) = stop_session_daemon(session_id) {
        eprintln!("koma daemon: warning during stop phase of restart: {e:#}");
    }

    // Spawn + wait for it to accept (false: we just killed it). Confirm-only stream.
    let stream = ensure_daemon_and_connect(session_id)
        .context("failed to start the new daemon")?;
    drop(stream); // we only needed to confirm it is accepting

    match read_pidfile(session_id) {
        Some(pid) => println!("koma daemon: restarted session {session_id} (pid {pid})"),
        None => println!(
            "koma daemon: restarted session {session_id} (pid unknown — pidfile not yet written)"
        ),
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
        DaemonSub::Status => cmd_status(),
        DaemonSub::Kill => cmd_kill(),
        DaemonSub::Restart => cmd_restart(),
        DaemonSub::Clean => cmd_clean(),
    }
}

/// Print usage for the `daemon` subcommand (bare/unknown verb). Returns the process
/// exit code the caller should use (non-zero — a malformed invocation is an error).
pub fn print_daemon_usage() -> i32 {
    eprintln!(
        "usage: koma daemon <status|kill|restart|clean>\n\
         \n\
         \x20 status   show whether the koma daemon is running (PID, socket, sessions)\n\
         \x20 kill     gracefully stop the running daemon (escalates to signals if needed)\n\
         \x20 restart  stop the daemon (if any) then start a fresh one\n\
         \x20 clean    remove a stale socket/pidfile when NO daemon is running"
    );
    2
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
    UnixStream::connect(&path).is_ok()
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
    UnixStream::connect(path).is_ok()
}

/// Enumerate the session sockets under [`store::run_dir`], returning each `*.sock`
/// path paired with whether a daemon is currently accepting on it.
///
/// Best-effort: an unreadable/absent run dir yields an empty list (no daemons), never an
/// error from the directory walk itself — only [`store::run_dir`] resolution can fail.
/// The `session_id` is the socket file stem (`<id>.sock`), the same key the daemon was
/// spawned with. Drives the `koma daemon …` admin verbs (which now act over ALL
/// sessions) without consulting any pidfile for liveness.
fn list_session_sockets() -> Result<Vec<(String, std::path::PathBuf, bool)>> {
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

/// The subset of [`list_session_sockets`] whose daemon is currently ACCEPTING, as
/// `(session_id, socket_path)` pairs. Stale sockets (dead daemon) are dropped.
fn live_session_sockets() -> Result<Vec<(String, std::path::PathBuf)>> {
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
fn spawn_daemon(session_id: &str, resume: bool) -> Result<u32> {
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

    // SAFETY: `setsid()` is async-signal-safe and the canonical way to detach a child
    // into its own session; it touches no Rust state and only runs in the forked child
    // between fork and exec. A failure is ignored (best-effort detach) — the daemon
    // still runs; it just shares our process group, which the SIGHUP handler tolerates.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
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
fn probe_or_clear(path: &Path) -> Result<bool> {
    match UnixStream::connect(path) {
        Ok(_stream) => Ok(true), // a daemon is already live (probe stream dropped)
        Err(e) => match e.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                // Stale socket from a crashed daemon: remove it so the spawn's bind
                // doesn't trip over `AddrInUse`. Best-effort (it may have just gone).
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
pub fn ensure_daemon_running(session_id: &str, resume: bool) -> Result<()> {
    let path = store::daemon_sock_path(session_id)?;
    if probe_or_clear(&path)? {
        return Ok(()); // already live — attach to the existing one
    }
    // Nothing live → spawn a detached daemon and wait until it accepts.
    spawn_and_wait_until_alive(session_id, &path, resume)
}

/// Spawn-or-attach: return a connected blocking [`UnixStream`] to a LIVE daemon,
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
pub fn ensure_daemon_and_connect(session_id: &str) -> Result<UnixStream> {
    let path = store::daemon_sock_path(session_id)?;

    if probe_or_clear(&path)? {
        // Already live — reconnect for the caller (the probe stream was dropped).
        return UnixStream::connect(&path)
            .with_context(|| format!("connect to live daemon socket {}", path.display()));
    }

    // Nothing live → spawn + wait until it accepts, then connect for the caller.
    spawn_and_wait_until_alive(session_id, &path, false)?;
    UnixStream::connect(&path)
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
fn spawn_and_wait_until_alive(session_id: &str, path: &Path, resume: bool) -> Result<()> {
    let pid = spawn_daemon(session_id, resume)?;
    let deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(path) {
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
fn send_request(stream: &mut UnixStream, req: &ClientRequest) -> Result<()> {
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
fn recv_frame(stream: &mut UnixStream, reader: &mut FrameReader) -> Result<DaemonFrame> {
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
fn connect_managed(path: &Path) -> Result<(UnixStream, FrameReader)> {
    let stream = UnixStream::connect(path)
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
fn read_pidfile(session_id: &str) -> Option<u32> {
    let path = store::daemon_pid_path(session_id).ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    contents.trim().parse::<u32>().ok()
}

/// Best-effort unlink of one session's socket + pidfile (the "turds" a crash can leave).
/// Each removal ignores a missing file; any other IO error is swallowed — these are
/// cleanup, never a hard failure.
fn unlink_daemon_files(session_id: &str) {
    if let Ok(sock) = store::daemon_sock_path(session_id) {
        let _ = std::fs::remove_file(sock);
    }
    if let Ok(pid) = store::daemon_pid_path(session_id) {
        let _ = std::fs::remove_file(pid);
    }
}

// ─── subcommands ─────────────────────────────────────────────────────────────

/// `koma daemon status` — report liveness for EVERY session-daemon via the bind-as-oracle
/// probe.
///
/// Daemon-per-session: enumerates the live `run/<id>.sock` daemons and prints one block
/// per session — its session id, advisory PID (from that session's pidfile), socket
/// path, and a best-effort session count from the daemon's own `ListSessions` snapshot
/// (failure to get the count never fails the command — liveness is already established).
/// With no live daemons it prints "no daemons running".
fn cmd_status() -> Result<()> {
    let live = live_session_sockets()?;
    if live.is_empty() {
        println!("koma daemon: no daemons running");
        return Ok(());
    }

    println!("koma daemon: {} session daemon(s) running", live.len());
    for (id, sock) in live {
        // PID is advisory (the pidfile may be missing/stale even while the socket is up —
        // they are written/removed at slightly different moments), so word it as such.
        let pid_str = match read_pidfile(&id) {
            Some(pid) => format!("pid {pid}"),
            None => "pid unknown (no pidfile)".to_string(),
        };
        println!("  session {id} ({pid_str})");
        println!("    socket: {}", sock.display());

        // Best-effort session count from THIS daemon's snapshot. (At this commit a
        // daemon owns exactly one session, so this is normally 1; kept because the wire
        // reply still carries the full set.) Any failure is non-fatal.
        match daemon_session_count(&sock) {
            Ok(n) => println!("    sessions: {n}"),
            Err(e) => println!("    sessions: unknown ({e})"),
        }
    }

    Ok(())
}

/// Ask the live daemon for its session count via `ListSessions` → `Snapshot`.
///
/// Bounded: it reads at most a handful of frames (skipping any interleaved Ack/Error)
/// before giving up, and the socket's read timeout caps the wait, so a daemon that
/// never answers surfaces as an `Err` (rendered as "unknown") rather than a hang.
fn daemon_session_count(sock: &Path) -> Result<usize> {
    let (mut stream, mut reader) = connect_managed(sock)?;
    send_request(&mut stream, &ClientRequest::ListSessions)?;

    // The reply we want is a Snapshot; tolerate a few non-Snapshot frames first.
    for _ in 0..8 {
        let frame = recv_frame(&mut stream, &mut reader)?;
        if let DaemonEvent::Snapshot(snap) = frame.event {
            return Ok(snap.sessions.len());
        }
    }
    Err(anyhow!("no snapshot in reply"))
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
fn stop_session_daemon(session_id: &str) -> Result<()> {
    if !daemon_alive(session_id) {
        // Sweep any leftover turds from a previous crash so the next start is clean.
        unlink_daemon_files(session_id);
        println!("koma daemon: session {session_id} not running");
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
        println!("koma daemon: stopped session {session_id} (graceful QuitDaemon)");
        return Ok(());
    }

    // --- fallback: signal the pidfile PID ---
    // Bind-as-oracle says it's still accepting (or graceful failed). Use the pidfile
    // ONLY as the signal target — if it's missing we can't signal, so just nuke files.
    let Some(pid) = read_pidfile(session_id) else {
        unlink_daemon_files(session_id);
        println!(
            "koma daemon: session {session_id} still up but no pidfile to signal; removed \
             stale socket/pidfile. If a daemon is still running, stop it manually."
        );
        return Ok(());
    };

    // SIGTERM (graceful at the OS level), then wait.
    send_signal(pid, libc::SIGTERM);
    if wait_until_dead(session_id, SIGNAL_GRACE) {
        unlink_daemon_files(session_id);
        println!("koma daemon: stopped session {session_id} (SIGTERM to pid {pid})");
        return Ok(());
    }

    // SIGKILL (last resort), then wait.
    send_signal(pid, libc::SIGKILL);
    let died = wait_until_dead(session_id, SIGNAL_GRACE);
    unlink_daemon_files(session_id);
    if died {
        println!("koma daemon: killed session {session_id} (SIGKILL to pid {pid})");
    } else {
        println!(
            "koma daemon: sent SIGKILL to pid {pid} (session {session_id}) but the socket is \
             still up; removed socket/pidfile. The process may be unkillable (zombie/stuck IO)."
        );
    }
    Ok(())
}

/// `koma daemon kill` — stop EVERY live session-daemon, escalating per session only if
/// one won't go.
///
/// Daemon-per-session: enumerates the live `run/<id>.sock` daemons and calls
/// [`stop_session_daemon`] on each (which prints its own per-session outcome). Each stop
/// is best-effort — one wedged session never blocks stopping the rest. A run dir with no
/// live daemons reports "no daemons running" (and still sweeps any stale turds).
fn cmd_kill() -> Result<()> {
    let live = live_session_sockets()?;
    if live.is_empty() {
        println!("koma daemon: no daemons running");
        // Sweep any stale socket/pidfiles left by crashed daemons.
        sweep_stale_files();
        return Ok(());
    }
    for (id, _path) in live {
        let _ = stop_session_daemon(&id);
    }
    Ok(())
}

/// `koma daemon restart` — stop EVERY live session-daemon, then respawn one per session
/// (each on its own keyed socket) and report the new PIDs.
///
/// Reuses [`restart_daemon`] (the per-session graceful→signal stop + spawn-and-confirm)
/// for each currently-live session, so "restart" is "a working daemon is up afterwards
/// for every session that was running", not just "children were forked". A restart error
/// for one session is surfaced but never blocks the others. With nothing live there is
/// nothing to restart (a fresh `koma` is how you start a daemon — restart only re-spawns
/// sessions that were already running).
fn cmd_restart() -> Result<()> {
    let live = live_session_sockets()?;
    if live.is_empty() {
        println!("koma daemon: no daemons running to restart (start one with `koma`)");
        return Ok(());
    }
    for (id, _path) in live {
        if let Err(e) = restart_daemon(&id) {
            eprintln!("koma daemon: failed to restart session {id}: {e:#}");
        }
    }
    Ok(())
}

/// `koma daemon clean` — the "OS shit happened, nuke the turds" escape hatch.
///
/// Daemon-per-session: scans every `run/<id>.sock`. For each socket whose daemon is DEAD
/// (bind-as-oracle probe refused) it unlinks that socket + its pidfile; sockets with a
/// LIVE daemon are left untouched (removing a live daemon's socket would orphan it). Then
/// it also sweeps any orphan `*.pid` whose `*.sock` is already gone. Reports exactly
/// which files it removed, and — when some daemons are still live — names them so the
/// user can `koma daemon kill` instead.
fn cmd_clean() -> Result<()> {
    let socks = list_session_sockets()?;
    let live: Vec<String> = socks
        .iter()
        .filter(|(_, _, alive)| *alive)
        .map(|(id, _, _)| id.clone())
        .collect();

    let mut removed: Vec<String> = Vec::new();
    // Remove every DEAD session's socket + pidfile; never touch a live one.
    for (id, path, alive) in &socks {
        if *alive {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            removed.push(path.display().to_string());
        }
        if let Ok(pid) = store::daemon_pid_path(id) {
            if std::fs::remove_file(&pid).is_ok() {
                removed.push(pid.display().to_string());
            }
        }
    }

    // Sweep orphan pidfiles whose socket is already gone (a crash that lost the socket
    // but left the pid). Skip any pid that belongs to a still-live session.
    for (id, pid) in orphan_pidfiles()? {
        if live.contains(&id) {
            continue;
        }
        if std::fs::remove_file(&pid).is_ok() {
            removed.push(pid.display().to_string());
        }
    }

    if !live.is_empty() {
        println!(
            "koma daemon: {} session daemon(s) still running ({}); left their files in place — \
             use `koma daemon kill` to stop them",
            live.len(),
            live.join(", ")
        );
    }
    if removed.is_empty() {
        println!("koma daemon: nothing to clean (no stale socket/pidfile)");
    } else {
        println!("koma daemon: removed stale file(s):");
        for f in removed {
            println!("  {f}");
        }
    }
    Ok(())
}

/// Best-effort sweep of stale socket/pidfiles for sessions whose daemon is DEAD, across
/// the whole run dir. Used by `cmd_kill`'s no-live-daemons branch to clean crash turds.
/// Live daemons are never touched. Errors are swallowed (pure cleanup).
fn sweep_stale_files() {
    if let Ok(socks) = list_session_sockets() {
        for (id, path, alive) in socks {
            if alive {
                continue;
            }
            let _ = std::fs::remove_file(&path);
            if let Ok(pid) = store::daemon_pid_path(&id) {
                let _ = std::fs::remove_file(pid);
            }
        }
    }
    if let Ok(orphans) = orphan_pidfiles() {
        for (_id, pid) in orphans {
            let _ = std::fs::remove_file(pid);
        }
    }
}

/// Enumerate `*.pid` files in the run dir whose matching `*.sock` does NOT exist (an
/// orphan pidfile from a crash that lost its socket), as `(session_id, pid_path)` pairs.
/// Best-effort: an unreadable run dir yields an empty list.
fn orphan_pidfiles() -> Result<Vec<(String, std::path::PathBuf)>> {
    let dir = store::run_dir()?;
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pid") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Only an orphan if there is no corresponding socket file.
        let sock_gone = store::daemon_sock_path(id)
            .map(|s| !s.exists())
            .unwrap_or(true);
        if sock_gone {
            out.push((id.to_string(), path));
        }
    }
    Ok(out)
}

// ─── signal + wait helpers ───────────────────────────────────────────────────

/// Send `sig` to `pid`, best-effort. A failure (ESRCH = already gone, EPERM = not
/// ours) is ignored — `kill` re-checks liveness via the socket afterwards, so a
/// failed signal just means the follow-up `wait_until_dead` decides the outcome.
fn send_signal(pid: u32, sig: libc::c_int) {
    // SAFETY: `kill(2)` with a real signal number has no memory-safety preconditions
    // and the FFI types match libc's signature. We intentionally ignore the result.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
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

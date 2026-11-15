//! Live-session discovery (no attach): probe one socket for its
//! [`SessionStatus`], and sweep every socket in the run dir. Split out of
//! [`super`] (the `manage` module) for file size — pure code motion, no
//! behaviour change.
//!
//! Both functions were already `pub fn`; `list_live_sessions` is re-exported
//! from `manage` (`pub use probe::list_live_sessions;`) so the existing
//! `manage::list_live_sessions()` call sites (the client-side swapper) keep
//! resolving unchanged. `probe_status` has no external caller today, so no
//! re-export is needed for it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ipc::frame::FrameReader;
use crate::ipc::proto::{ClientRequest, DaemonEvent, SessionStatus};
use crate::ipc::SyncIpcStream;
use crate::model::store;

/// Read timeout on the discovery probe socket. Short, because a discovery sweep may
/// run synchronously in front of the picker and must stay snappy; a daemon that does
/// not answer `Status` within this window is treated as un-probeable (→ `None`), never
/// waited on. Independent of [`super::SOCKET_IO_TIMEOUT`] (the admin verbs' 3s budget)
/// so discovery can be tightened without loosening the admin path.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Defensive cap on how many frames the probe will read before giving up, so a daemon
/// that streams an unbounded run of non-`Status` frames can never wedge discovery even
/// if each individual read keeps beating the timeout.
const PROBE_MAX_FRAMES: usize = 16;

/// Probe ONE session-daemon socket for its [`SessionStatus`] WITHOUT attaching.
///
/// Opens a fresh blocking [`SyncIpcStream`] (runtime-free — discovery may run before any
/// tokio runtime exists), sets a short read timeout ([`PROBE_TIMEOUT`]) so a wedged
/// daemon can never hang the sweep, writes a single [`ClientRequest::Status`] frame
/// using the SAME 4-byte-big-endian-length + JSON codec the rest of the wire speaks
/// (via [`super::send_request`] / [`super::recv_frame`], the shared sync framing — no
/// second codec), then reads frames until a [`DaemonEvent::Status`] arrives. Any other
/// frame type (Ack / Hello / a stray Snapshot) is ignored defensively. Returns
/// `Some(status)` on a clean reply, or `None` on ANY connect / timeout / decode failure
/// — and on the read cap. The stream is dropped (closed) on every path.
///
/// Side-effect-free on the daemon: `Status` is a pure metadata read (the daemon does
/// NOT attach this connection, change its foreground, or stream a snapshot for it), so a
/// transient connect→Status→close leaves the daemon's session state untouched.
#[allow(dead_code)] // consumed by the hub swapper in the next commit
pub fn probe_status(sock_path: &Path) -> Option<SessionStatus> {
    // Connect (blocking). Anything refused / missing ⇒ not probeable.
    let mut stream = SyncIpcStream::connect(sock_path).ok()?;
    // Bound every read so a daemon that accepts but never answers can't hang us. The
    // write side is naturally bounded (the Status frame is tiny); a missing write
    // timeout can't wedge a sweep, but set it too for symmetry with the read budget.
    stream.set_read_timeout(Some(PROBE_TIMEOUT)).ok()?;
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));

    // Ask for the one-shot metadata frame using the shared sync framing.
    super::send_request(&mut stream, &ClientRequest::Status).ok()?;

    // Read until we see Status, ignoring any other frame defensively. Bounded by BOTH
    // the per-read timeout (a quiet daemon trips `recv_frame`'s read error) and the
    // frame cap (a chatty daemon can't keep us reading forever).
    let mut reader = FrameReader::new();
    for _ in 0..PROBE_MAX_FRAMES {
        match super::recv_frame(&mut stream, &mut reader) {
            Ok(frame) => {
                if let DaemonEvent::Status(status) = frame.event {
                    return Some(status); // stream dropped on return
                }
                // Some other frame (e.g. Ack) — keep reading for the Status reply.
            }
            // Timeout / EOF / decode error: give up on this daemon (→ None).
            Err(_) => return None,
        }
    }
    None // hit the frame cap without a Status reply
}

/// Discover EVERY live session-daemon and collect each one's [`SessionStatus`], WITHOUT
/// attaching to any of them.
///
/// Enumerates the `run/<id>.sock` sockets via the same scan the admin verbs use, then
/// [`probe_status`]-es each in turn, collecting the successful replies. This is the data
/// source the hub/swapper consumes to render the live-session picker. Sockets that fail
/// to probe (dead daemon that left a stale socket, or a wedged one) are dropped from the
/// result AND, if any failed, swept from disk via the shared
/// [`super::commands::sweep_stale_files`] cleanup — which re-checks liveness with a
/// fresh connect and only unlinks sockets that are genuinely dead, so a live-but-slow
/// daemon's socket is never removed.
///
/// Best-effort: an unreadable/absent run dir yields an empty list (no daemons). The
/// probe order follows directory order (unspecified); the caller sorts for display.
#[allow(dead_code)] // consumed by the hub swapper in the next commit
pub fn list_live_sessions() -> Vec<SessionStatus> {
    let socks = match super::list_session_sockets() {
        Ok(s) => s,
        // No run dir / unreadable ⇒ no live sessions to report.
        Err(_) => return Vec::new(),
    };

    let (out, any_failed) = probe_listed(&socks);

    // Clean up any dead/stale sockets we ran into. `sweep_stale_files` re-probes each
    // socket's liveness itself and never touches a live daemon, so it is safe even
    // against a daemon that merely answered slowly this pass.
    if any_failed {
        super::commands::sweep_stale_files();
    }

    out
}

/// Discover every live session-daemon exactly like [`list_live_sessions`], but never
/// clean up: no socket and no pidfile is removed, whatever the probes find.
///
/// The cleanup is left out because it also removes every pidfile that has no socket
/// beside it, and a daemon that is still starting writes its pidfile before it binds
/// its socket. A caller that polls (a dashboard refreshing every few seconds) would
/// otherwise delete a starting daemon's pidfile, and a later stop would have no PID to
/// signal. Dead and wedged daemons are left out of the result and their files stay for
/// `koma daemon clean` or the next [`list_live_sessions`].
#[allow(dead_code)] // read by session listings that must not change the run dir
pub fn probe_live_sessions() -> Vec<SessionStatus> {
    match super::list_session_sockets() {
        Ok(socks) => probe_listed(&socks).0,
        Err(_) => Vec::new(),
    }
}

/// The probe loop shared by [`list_live_sessions`] and [`probe_live_sessions`]: `Status`
/// replies from every socket in `socks`, plus whether any socket was dead or failed to
/// answer. Touches no file.
fn probe_listed(socks: &[(String, PathBuf, bool)]) -> (Vec<SessionStatus>, bool) {
    let mut out = Vec::new();
    let mut any_failed = false;
    for (_id, path, alive) in socks {
        // Skip sockets the cheap connect-probe already shows as dead (saves a Status
        // round-trip), and mark them as failed.
        if !alive {
            any_failed = true;
            continue;
        }
        match probe_status(path) {
            Some(status) => out.push(status),
            // Accepted the connect but failed to answer Status (wedged / mid-shutdown):
            // exclude it and report the failure so a sweeping caller re-decides its fate.
            None => any_failed = true,
        }
    }
    (out, any_failed)
}

/// FIRE-AND-FORGET fan-out of a [`ClientRequest::UnloadExtension`] to EVERY live
/// session-daemon — the uninstall broadcast (step 3) that makes OTHER daemons drop a
/// just-uninstalled extension's in-memory footprint (contributed MCP tools, running child,
/// ext-agent registry, published context, buffered prompts) WITHOUT waiting for a restart.
///
/// Best-effort throughout, mirroring the Windows `send_shutdown_request` precedent: each
/// live socket is connected, the ONE frame is written, and the stream is DROPPED without
/// reading a reply — a `connect → write → close` contract (the tiny frame is delivered to
/// the kernel buffer before close, so the daemon reads it even though we never wait for its
/// Ack). A daemon too old to know the verb error-replies (never read) or drops the
/// connection — ignored, exactly like the additive MCP `Fingerprint` probe. Every
/// connect/write failure is logged and the sweep CONTINUES; this never fails, and never
/// aborts the caller's uninstall.
///
/// It sends to EVERY live session (the sender's own daemon INCLUDED, when it has one — the
/// receiver's unload is idempotent, so a self-send is harmless): the caller runs this OFF
/// the event loop (a bare OS thread), so even a self-connect can never wedge the loop.
/// Enumerates targets via [`list_live_sessions`], mapping each `session_id` to its keyed
/// socket.
pub fn broadcast_unload_extension(ext_id: &str) {
    let req = ClientRequest::UnloadExtension {
        id: ext_id.to_string(),
    };
    for status in list_live_sessions() {
        let sock = match store::daemon_sock_path(&status.session_id) {
            Ok(p) => p,
            Err(e) => {
                store::append_global_error_log(
                    "ext-uninstall",
                    &format!(
                        "unload fan-out: no socket path for session {}: {e}",
                        status.session_id
                    ),
                );
                continue;
            }
        };
        if let Err(e) = send_unload_frame(&sock, &req) {
            store::append_global_error_log(
                "ext-uninstall",
                &format!(
                    "unload fan-out to session {} failed: {e}",
                    status.session_id
                ),
            );
        }
    }
}

/// How a fan-out went: `reached` daemons were written the frame, `failed` live
/// daemons could not be (no socket path, refused connect, or a failed write).
/// Delivery is fire-and-forget, so `reached` means written, not acknowledged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BroadcastReport {
    pub reached: usize,
    pub failed: usize,
}

/// FIRE-AND-FORGET fan-out of a [`ClientRequest::ReloadGlobalCatalogue`] to
/// every live session-daemon — the config-change broadcast that makes OTHER
/// daemons re-read `~/.koma/config.json` + global agents from disk so their
/// in-memory catalogue stays current without a restart.
///
/// Best-effort throughout, identical contract to [`broadcast_unload_extension`]:
/// connect → write one frame → drop. Called on a background OS thread by
/// [`super::save_config_and_broadcast`] so the saving daemon's event loop
/// never blocks on peer connects. Callers that need the outcome use
/// [`broadcast_reload_global_catalogue_report`], which sends the same frames.
pub fn broadcast_reload_global_catalogue() {
    let _ = broadcast_reload_global_catalogue_report();
}

/// [`broadcast_reload_global_catalogue`] that also counts the daemons it
/// reached and the ones it could not. Failures are logged exactly as before.
pub fn broadcast_reload_global_catalogue_report() -> BroadcastReport {
    let targets = list_live_sessions().into_iter().map(|status| {
        let sock = store::daemon_sock_path(&status.session_id);
        (status.session_id, sock)
    });
    send_reload_to(targets, &mut |body| {
        store::append_global_error_log("config-reload", &body)
    })
}

/// Write one [`ClientRequest::ReloadGlobalCatalogue`] frame to each target,
/// passing every failure's log line to `log`. Split from the session sweep so
/// the counting can be exercised against sockets a test controls.
fn send_reload_to(
    targets: impl IntoIterator<Item = (String, anyhow::Result<PathBuf>)>,
    log: &mut dyn FnMut(String),
) -> BroadcastReport {
    let req = ClientRequest::ReloadGlobalCatalogue;
    let mut report = BroadcastReport::default();
    for (session_id, sock) in targets {
        let sock = match sock {
            Ok(p) => p,
            Err(e) => {
                report.failed += 1;
                log(format!(
                    "reload fan-out: no socket path for session {session_id}: {e}"
                ));
                continue;
            }
        };
        match send_unload_frame(&sock, &req) {
            Ok(()) => report.reached += 1,
            Err(e) => {
                report.failed += 1;
                log(format!("reload fan-out to session {session_id} failed: {e}"));
            }
        }
    }
    report
}

/// Search preferences need no catalogue or modal rebuild.
pub fn broadcast_reload_web_search() {
    for status in list_live_sessions() {
        let Ok(sock) = store::daemon_sock_path(&status.session_id) else {
            continue;
        };
        let _ = send_unload_frame(&sock, &ClientRequest::ReloadWebSearch);
    }
}

/// Connect ONE session socket, WRITE the fan-out frame, then drop it (fire-and-forget — no
/// reply is read). A short write timeout ([`PROBE_TIMEOUT`]) bounds a wedged daemon; the
/// tiny frame lands in the kernel buffer before close, so the daemon reads it even though we
/// never wait for its Ack. Runtime-free blocking IO (same sync framing as the `Status`
/// probe), so it is safe to call from a plain OS thread with no tokio runtime.
fn send_unload_frame(sock: &Path, req: &ClientRequest) -> std::io::Result<()> {
    let mut stream = SyncIpcStream::connect(sock)?;
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    super::send_request(&mut stream, req).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            format!("write unload frame: {e:#}"),
        )
    })?;
    Ok(())
}

/// The terminal reply of a cross-daemon [`spawn_into_session`] once the target daemon
/// answered on the wire. A transport failure (connect / write / read / decode / EOF /
/// timeout) is surfaced as the outer `Err(io::Error)` instead, with the ErrorKind
/// preserved so the caller can map it (connect-refused/not-found vs. everything else).
#[derive(Debug)]
pub enum SpawnIntoReply {
    /// The target daemon replied [`DaemonEvent::Ack`] — the sub-agent was spawned or queued
    /// in its session.
    Accepted,
    /// The target daemon replied [`DaemonEvent::Error`] — it rejected the spawn (carries its
    /// human-readable reason, e.g. "no live session").
    Rejected(String),
}

/// Fire ONE [`ClientRequest::SpawnAgent`] at another session-daemon's keyed socket and read
/// its one-shot [`DaemonEvent::Ack`]/[`DaemonEvent::Error`] reply — the transport half of the
/// extension `sessions.spawn_into` cross-process branch (W7).
///
/// Blocking + runtime-free (it runs inside the broker's `spawn_blocking`), speaking the SAME
/// length-prefixed sync codec ([`super::send_request`]/[`super::recv_frame`]) as the `Status`
/// discovery probe. It NEVER sends `Attach`, so the target enrolls the connection but never
/// streams it a snapshot — a pure connect→SpawnAgent→read-Ack→close, exactly the connectionless
/// contract [`probe_status`] relies on. Bounded by the same short read timeout
/// ([`PROBE_TIMEOUT`]) + frame cap ([`PROBE_MAX_FRAMES`]) as the probe, so a wedged/chatty
/// target can never hang the caller (worst case ≈ `PROBE_TIMEOUT × PROBE_MAX_FRAMES`, well
/// under the broker's 25s inner budget). No retries, no daemon auto-spawn.
///
/// Returns `Ok(Accepted)`/`Ok(Rejected(reason))` on a clean reply, or an `Err(io::Error)`
/// (kind preserved) on any connect/write/read/decode/EOF/timeout/cap failure — the caller
/// maps the kind to its structured error (connect-refused/ENOENT ⇒ "session not live";
/// everything else ⇒ "target daemon incompatible or unavailable").
pub fn spawn_into_session(
    sock_path: &Path,
    req: &ClientRequest,
) -> std::io::Result<SpawnIntoReply> {
    // Connect (blocking). A refused / missing socket propagates its ErrorKind verbatim so the
    // caller can distinguish "not live" from an incompatible/unavailable daemon.
    let mut stream = SyncIpcStream::connect(sock_path)?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));

    // Send the spawn request. `send_request` yields an anyhow error on a write failure; a
    // failed write is a post-connect transport fault, so collapse it to an io error whose
    // kind lands on the caller's "unavailable" arm (never "not live").
    super::send_request(&mut stream, req)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "spawn write failed"))?;

    // Read until Ack/Error, ignoring any other frame defensively. Bounded by BOTH the per-read
    // timeout (a quiet target trips `recv_frame`'s read error) and the frame cap (a chatty one
    // can't keep us reading forever). Any read/decode/EOF fault collapses to an io error whose
    // kind lands on the caller's "unavailable" arm.
    let mut reader = FrameReader::new();
    for _ in 0..PROBE_MAX_FRAMES {
        match super::recv_frame(&mut stream, &mut reader) {
            Ok(frame) => match frame.event {
                DaemonEvent::Ack => return Ok(SpawnIntoReply::Accepted),
                DaemonEvent::Error(msg) => return Ok(SpawnIntoReply::Rejected(msg)),
                // Some other frame (e.g. a stray Hello) — keep reading for the real reply.
                _ => continue,
            },
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "spawn reply read/decode failed",
                ))
            }
        }
    }
    // Hit the frame cap without an Ack/Error — treat as an unavailable/incompatible target.
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "no spawn ack within frame cap",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::frame::{read_frame, FrameReader};

    fn unique() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    /// A socket path nobody listens on, kept short for the unix path limit.
    fn sock_path(tag: &str) -> PathBuf {
        let nanos = unique();
        #[cfg(unix)]
        {
            let dir = std::env::temp_dir().join(format!("koma-bc-{tag}-{nanos}"));
            std::fs::create_dir_all(&dir).expect("mkdir");
            dir.join("d.sock")
        }
        #[cfg(windows)]
        {
            PathBuf::from(format!(r"\\.\pipe\koma-bc-{tag}-{nanos}"))
        }
    }

    fn cleanup(sock: &Path) {
        #[cfg(unix)]
        if let Some(dir) = sock.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        #[cfg(windows)]
        let _ = sock;
    }

    #[test]
    fn a_refused_socket_and_a_missing_path_count_as_failed() {
        let refused = sock_path("refused");
        let targets = vec![
            ("refused".to_string(), Ok(refused.clone())),
            ("pathless".to_string(), Err(anyhow::anyhow!("no run dir"))),
        ];
        let mut lines = Vec::new();
        let report = send_reload_to(targets, &mut |line| lines.push(line));
        cleanup(&refused);

        assert_eq!(
            report,
            BroadcastReport {
                reached: 0,
                failed: 2
            }
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("reload fan-out to session refused failed: "));
        assert_eq!(
            lines[1],
            "reload fan-out: no socket path for session pathless: no run dir"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_live_socket_is_reached_with_one_reload_frame() {
        let live = sock_path("live");
        let refused = sock_path("dead");
        let listener = crate::ipc::server::bind(&live).expect("bind");
        let server = tokio::spawn(async move {
            let mut conn = crate::ipc::server::accept(&listener).await.expect("accept");
            let mut reader = FrameReader::new();
            read_frame(&mut conn, &mut reader).await.expect("read frame")
        });

        let targets = vec![
            ("live".to_string(), Ok(live.clone())),
            ("dead".to_string(), Ok(refused.clone())),
        ];
        let report = tokio::task::spawn_blocking(move || {
            let mut lines = Vec::new();
            let report = send_reload_to(targets, &mut |line| lines.push(line));
            (report, lines)
        })
        .await
        .expect("join fan-out");
        let frame = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("frame within timeout")
            .expect("join server");
        cleanup(&live);
        cleanup(&refused);

        let (report, lines) = report;
        assert_eq!(
            report,
            BroadcastReport {
                reached: 1,
                failed: 1
            }
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(
            frame,
            serde_json::to_vec(&ClientRequest::ReloadGlobalCatalogue).expect("encode")
        );
    }

    /// A fresh directory under the OS temp root, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "koma-probe-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A socket path nothing listens on: a bound-then-dropped unix socket file, or on
    /// Windows a plain file whose path no pipe server owns.
    fn dead_socket(dir: &Path, id: &str) -> PathBuf {
        let path = dir.join(format!("{id}.sock"));
        #[cfg(unix)]
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        #[cfg(windows)]
        std::fs::write(&path, b"").unwrap();
        path
    }

    #[test]
    fn probe_listed_removes_no_socket_or_pidfile() {
        let tmp = TempDir::new("keep");
        let dead = dead_socket(&tmp.0, "dead");
        let dead_pid = tmp.0.join("dead.pid");
        let orphan_pid = tmp.0.join("starting.pid");
        std::fs::write(&dead_pid, "1").unwrap();
        std::fs::write(&orphan_pid, "1").unwrap();

        // Once as the cheap connect-probe reports it, once as a socket that accepted
        // the connect check but then fails the Status probe.
        let socks = vec![
            ("dead".to_string(), dead.clone(), false),
            ("dead".to_string(), dead.clone(), true),
        ];
        let (live, any_failed) = probe_listed(&socks);

        assert!(live.is_empty());
        assert!(any_failed);
        assert!(dead.exists(), "dead socket was removed");
        assert!(dead_pid.exists(), "pidfile of a dead socket was removed");
        assert!(orphan_pid.exists(), "orphan pidfile was removed");
    }
}

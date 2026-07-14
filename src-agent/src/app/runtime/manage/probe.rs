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

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::ipc::frame::FrameReader;
use crate::ipc::proto::{ClientRequest, DaemonEvent, SessionStatus};

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
/// Opens a fresh blocking [`UnixStream`] (runtime-free — discovery may run before any
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
    let mut stream = UnixStream::connect(sock_path).ok()?;
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

    let mut out = Vec::new();
    let mut any_failed = false;
    for (_id, path, alive) in &socks {
        // Skip sockets the cheap connect-probe already shows as dead (saves a Status
        // round-trip), and mark them for the sweep below.
        if !alive {
            any_failed = true;
            continue;
        }
        match probe_status(path) {
            Some(status) => out.push(status),
            // Accepted the connect but failed to answer Status (wedged / mid-shutdown):
            // exclude it and let the stale-sweep re-decide its fate by a fresh connect.
            None => any_failed = true,
        }
    }

    // Clean up any dead/stale sockets we ran into. `sweep_stale_files` re-probes each
    // socket's liveness itself and never touches a live daemon, so it is safe even
    // against a daemon that merely answered slowly this pass.
    if any_failed {
        super::commands::sweep_stale_files();
    }

    out
}

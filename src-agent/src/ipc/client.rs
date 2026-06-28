//! Client-side socket helpers: connect to the daemon and exchange frames.
//!
//! The TUI client uses [`connect`] to reach the daemon's unix socket, then drives
//! the connection with the thin [`send_frame`] / [`recv_frame`] wrappers over the
//! shared [`super::frame`] codec. A successful [`connect`] is also the client half
//! of the liveness oracle (critique #3): if the connection is refused / there is
//! no listener, no daemon is live and the caller should spawn one (that spawn-or-
//! attach decision lands in a later stage — this module only provides the
//! primitives).

use std::path::Path;

use tokio::net::UnixStream;

use super::frame::{self, FrameReader};
use crate::model::store;

/// Connect to the daemon's unix socket at `path` (typically
/// [`store::daemon_sock_path`]).
///
/// A returned `Ok` stream means a daemon is live and listening. An `Err`
/// (`NotFound` / `ConnectionRefused`) means no daemon is up — the signal the
/// spawn-or-attach logic uses to decide it must spawn one.
pub async fn connect(path: &Path) -> std::io::Result<UnixStream> {
    UnixStream::connect(path).await
}

/// Convenience over [`store::daemon_sock_path`] + [`connect`]: resolve the
/// canonical daemon socket path and connect to it.
#[allow(dead_code)] // wired in daemon stage 3+ (spawn-or-attach)
pub async fn connect_default() -> anyhow::Result<UnixStream> {
    let path = store::daemon_sock_path()?;
    Ok(connect(&path).await?)
}

/// Send one length-prefixed frame (the raw JSON payload bytes) to the daemon.
pub async fn send_frame(stream: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    frame::write_frame(stream, bytes).await
}

/// Receive one complete length-prefixed frame from the daemon, blocking until it
/// fully arrives. `reader` carries the reassembly buffer across calls (a single
/// socket read may deliver more than one frame), so the SAME [`FrameReader`] must
/// be reused for the lifetime of the connection.
pub async fn recv_frame(
    stream: &mut UnixStream,
    reader: &mut FrameReader,
) -> std::io::Result<Vec<u8>> {
    frame::read_frame(stream, reader).await
}

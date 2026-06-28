//! Daemon-side socket setup: bind the unix listener and accept client connections.
//!
//! # Bind is the liveness oracle (critique #3)
//!
//! Whether the daemon is "alive" is decided by who currently HOLDS the bound unix
//! socket — NOT by reading a PID file and probing `/proc`. PIDs get reused, which
//! would wedge spawn-or-attach into talking to an unrelated process. So the real
//! liveness test (added in the spawn-or-attach stage) is: try to `connect` to the
//! socket — success means a daemon is live; connection refused / no listener means
//! it is not, and this process may [`bind`] and become the daemon. [`bind`] here
//! is the other half of that contract: it removes any stale socket file left by a
//! crashed daemon, then binds fresh. The spawn/attach decision logic that uses it
//! lands in a later stage; this module only provides the primitives.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use tokio::net::{UnixListener, UnixStream};

use crate::app::runtime::HubInbound;
use crate::ipc::conn;
use crate::model::store;

/// Bind the daemon's unix-domain listener at `path` (typically
/// [`store::daemon_sock_path`]).
///
/// Steps:
/// 1. Ensure the parent dir (`~/.koma`) exists — `bind` fails if it does not.
/// 2. Unlink any stale socket file already at `path`. A leftover socket from a
///    crashed daemon would otherwise make `bind` fail with `AddrInUse` even though
///    nobody is listening. (Removing it is safe precisely because bind — not the
///    file's existence — is the liveness oracle; a still-live daemon keeps the
///    socket and the caller would have connected to it instead of binding.)
/// 3. Bind and return the listener. Holding it is what makes this process the live
///    daemon.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    // 1. ~/.koma (or whatever parent the path has) must exist before bind.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 2. Remove a stale socket file; ignore "not found" (nothing to clean up).
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // 3. Bind fresh — this is the liveness oracle.
    UnixListener::bind(path)
}

/// Convenience over [`store::daemon_sock_path`] + [`bind`]: resolve the canonical
/// daemon socket path and bind it.
#[allow(dead_code)] // wired in daemon stage 3+ (spawn-or-attach)
pub fn bind_default() -> anyhow::Result<UnixListener> {
    let path = store::daemon_sock_path()?;
    Ok(bind(&path)?)
}

/// Await the next client connection on `listener`, returning just the stream
/// (the peer's anonymous unix address is not needed — clients are identified by
/// the [`super::proto::ClientRequest::Attach`] handshake, not their socket addr).
pub async fn accept(listener: &UnixListener) -> std::io::Result<UnixStream> {
    let (stream, _addr) = listener.accept().await?;
    Ok(stream)
}

/// The daemon's accept loop (daemon stage 5): accept connections forever and spawn
/// a per-client [`conn`] task for each, handing it a clone of the hub's request
/// channel `hub_tx`.
///
/// Each accepted stream gets a fresh, monotonically increasing `client_id` (the
/// stable handle the hub keys its registry on — index-agnostic, like the session
/// UUIDs). The loop OWNS the `listener` (so it lives as long as the loop) and runs
/// as a tokio task spawned by the daemon runner; the synchronous `daemon_loop` runs
/// on the main thread in parallel and never blocks on accept.
///
/// A transient `accept` error (e.g. EMFILE) is logged-by-ignoring and retried —
/// one bad accept must not tear down the whole daemon's listener. The loop only
/// ends when the runtime is dropped at shutdown.
pub async fn accept_loop(listener: UnixListener, hub_tx: Sender<HubInbound>) {
    let next_id = AtomicU64::new(1);
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let client_id = next_id.fetch_add(1, Ordering::Relaxed);
                conn::spawn(stream, client_id, hub_tx.clone());
            }
            // Transient accept failure: don't kill the listener, just try again.
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

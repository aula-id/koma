//! Stdio ↔ session-daemon socket bridge for `koma server`.
//!
//! Bidirectional length-prefixed frame proxy. Does **not** own a session: the
//! durable agent is the remote `koma --daemon` process on `run/<id>.sock`. This
//! process only ensures that daemon is up, dials it, and copies frames between
//! SSH stdio and the socket. EOF on either side stops the proxy; QuitDaemon is
//! never injected.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::ipc::frame::{self, FrameReader};
use crate::ipc::{self, IpcStream};

/// How long to wait for a wedged bridge child after stdin is closed before kill.
pub(crate) const BRIDGE_CHILD_WAIT: Duration = Duration::from_secs(2);

/// Ensure a session-daemon is accepting, dial its socket, and proxy
/// length-prefixed IPC frames between `stdin`/`stdout` and the socket until
/// either side hits EOF.
///
/// `workdir` is passed to [`super::manage::ensure_daemon_running`] only when a
/// daemon must be spawned — a live daemon is never chdir'd.
pub async fn run_stdio_sock_bridge<R, W>(
    session_id: &str,
    workdir: Option<&Path>,
    mut stdin: R,
    mut stdout: W,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    super::manage::ensure_daemon_running(session_id, false, workdir)
        .with_context(|| format!("ensure session-daemon for {session_id}"))?;

    let sock_path = crate::model::store::daemon_sock_path(session_id)?;
    let stream = ipc::client::connect(&sock_path)
        .await
        .with_context(|| format!("connect to session-daemon socket {}", sock_path.display()))?;

    proxy_frames(stream, &mut stdin, &mut stdout).await
}

/// Bidirectional raw frame proxy: stdin→sock and sock→stdout.
///
/// Stops when either direction EOF/errors. Does not interpret or inject frames.
pub async fn proxy_frames<R, W>(
    stream: IpcStream,
    stdin: &mut R,
    stdout: &mut W,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let (mut sock_r, mut sock_w) = ipc::split_stream(stream);

    let to_sock = async {
        let mut reader = FrameReader::new();
        while let Ok(payload) = frame::read_frame_from(stdin, &mut reader).await {
            if frame::write_frame_to(&mut sock_w, &payload).await.is_err() {
                break;
            }
        }
        // Half-close the write side so the daemon sees client disconnect.
        let _ = sock_w.shutdown().await;
    };

    let to_stdout = async {
        let mut reader = FrameReader::new();
        while let Ok(payload) = frame::read_frame_from(&mut sock_r, &mut reader).await {
            if frame::write_frame_to(stdout, &payload).await.is_err() {
                break;
            }
        }
    };

    // Either direction finishing ends the bridge. The sibling task is dropped
    // (cancels its I/O) when this select completes.
    tokio::select! {
        _ = to_sock => {}
        _ = to_stdout => {}
    }

    Ok(())
}

/// Reap an SSH bridge child after the client has flushed Detach/QuitDaemon.
///
/// Order: wait briefly for a clean exit; kill only if still running. Killing
/// the bridge must never be treated as "delete the remote session" — that is
/// QuitDaemon's job on the session-daemon.
pub async fn reap_bridge_child(child: &mut tokio::process::Child) {
    match tokio::time::timeout(BRIDGE_CHILD_WAIT, child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

#[cfg(test)]
#[path = "stdio_bridge_test.rs"]
mod tests;

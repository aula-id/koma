//! Client bridge for remote `koma server` connections.
//!
//! Similar to [`super::connect`] + [`super::bridge`] but works over generic
//! `AsyncRead + AsyncWrite` streams (SSH channel stdin/stdout) instead of a
//! unix socket. The remote peer is the stdio bridge (`koma server`), which
//! proxies frames to the durable session-daemon on `run/<id>.sock`.

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};

use crate::ipc::frame::{self, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame};

use super::bridge::REQ_POLL;
use super::connect::{Connection, TransportKind, HELLO_HANDSHAKE_TIMEOUT};

/// Connect to a remote koma server over generic async streams (stdin/stdout),
/// send Attach, and run the Hello handshake.
///
/// This is the remote equivalent of [`super::connect::connect_attach_and_handshake`].
/// Instead of connecting to a unix socket, it takes ownership of the SSH channel's
/// stdin/stdout and bridges them to the same TUI client loop.
pub(crate) fn connect_remote<R, W>(
    handle: &tokio::runtime::Handle,
    reader: R,
    writer: W,
    host_id: String,
    session_id: String,
) -> Result<Connection>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // Bridge channels — same as the local client.
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();
    let (req_tx, req_rx) = std::sync::mpsc::channel::<ClientRequest>();

    // Spawn reader/writer tasks that work with generic streams.
    // We can't reuse the unix-socket-typed `reader_task`/`writer_task` from
    // `bridge.rs`, so we implement equivalent logic here with generic bounds.
    let writer_handle = {
        let _enter = handle.enter();

        // Reader task: read frames from the remote server and push to frame_tx.
        handle.spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut frame_reader = FrameReader::new();
            while let Ok(bytes) = frame::read_frame_from(&mut reader, &mut frame_reader).await {
                match serde_json::from_slice::<DaemonFrame>(&bytes) {
                    Ok(frame) => {
                        if frame_tx.send(frame).is_err() {
                            break;
                        }
                    }
                    // A malformed frame from the daemon is a protocol fault; stop.
                    Err(_) => break,
                }
            }
            // Dropping frame_tx signals disconnection.
        });

        // Writer task: drain req_rx and write frames to the remote server.
        handle.spawn(async move {
            let mut writer = writer;
            let mut poll = tokio::time::interval(REQ_POLL);
            loop {
                poll.tick().await;
                let mut batch: Vec<ClientRequest> = Vec::new();
                let mut closed = false;
                loop {
                    match req_rx.try_recv() {
                        Ok(req) => batch.push(req),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            closed = true;
                            break;
                        }
                    }
                }
                for req in &batch {
                    if let Ok(json) = serde_json::to_vec(&req) {
                        if frame::write_frame_to(&mut writer, &json).await.is_err() {
                            return; // dead socket
                        }
                    }
                }
                // Channel closed AND the final drained batch is flushed: safe to
                // return (the shutdown frame(s) are on the wire).
                if closed && batch.is_empty() {
                    break;
                }
            }
        })
    };

    // Send the Attach handshake.
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let _ = req_tx.send(ClientRequest::Attach {
        foreground_id: None,
        cwd,
    });

    // Wait for Hello (same as local).
    let deadline = std::time::Instant::now() + HELLO_HANDSHAKE_TIMEOUT;
    let mut prebuffered = Vec::new();
    let mut daemon_version: Option<String> = None;

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match frame_rx.recv_timeout(remaining) {
            Ok(frame) => match frame.event {
                DaemonEvent::Hello { version } => {
                    daemon_version = Some(version);
                    break;
                }
                // A non-Hello frame arrived first: keep it for the render loop.
                _ => prebuffered.push(frame),
            },
            // Timed out or the reader task dropped its sender.
            Err(_) => break,
        }
    }

    Ok(Connection {
        frame_rx,
        req_tx,
        writer_handle,
        prebuffered,
        daemon_version,
        transport: TransportKind::Remote {
            host_id,
            session_id,
        },
    })
}

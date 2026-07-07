use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use crate::ipc::frame::{self, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonFrame};

/// How often the writer task polls its (sync) request queue. 4ms matches the
/// daemon conn's `FRAME_POLL` so a typed key reaches the daemon within one tick.
pub(super) const REQ_POLL: Duration = Duration::from_millis(4);

/// Upper bound on how long the client teardown waits for the writer task to flush
/// its final queued frame(s) (the shutdown `QuitDaemon`/`Detach`) before the tokio
/// runtime is dropped. The writer drains-and-returns the instant its channel closes
/// (well under one `REQ_POLL`), so this is only a safety ceiling against a wedged
/// socket — exit must never hang on a misbehaving daemon write half.
pub(super) const WRITER_FLUSH_TIMEOUT: Duration = Duration::from_millis(200);

/// Reader task: decode framed [`DaemonFrame`]s off the socket and push them onto the
/// loop's incoming channel. On socket EOF / cap violation / decode error it returns,
/// dropping `frame_tx` — the loop's `try_recv` then observes `Disconnected` and exits.
/// `read_frame_from` enforces [`crate::ipc::proto::MAX_FRAME_BYTES`] on every prefix.
pub(super) async fn reader_task(
    mut read_half: tokio::net::unix::OwnedReadHalf,
    frame_tx: Sender<DaemonFrame>,
) {
    let mut reader = FrameReader::new();
    // `while let Ok(..)` ends the loop on EOF / cap violation / read error (the
    // daemon closed or misbehaved); a malformed-frame decode or a gone loop breaks.
    while let Ok(bytes) = frame::read_frame_from(&mut read_half, &mut reader).await {
        match serde_json::from_slice::<DaemonFrame>(&bytes) {
            // Forward the frame; a send error means the loop is gone (client
            // exiting) -> stop reading.
            Ok(frame) => {
                if frame_tx.send(frame).is_err() {
                    break;
                }
            }
            // A malformed frame from the daemon is a protocol fault; stop the
            // connection rather than guess (the loop sees the dropped sender).
            Err(_) => break,
        }
    }
    // Dropping `frame_tx` here signals the loop the connection is gone.
}

/// Writer task: drain the loop's outbound [`ClientRequest`] queue to the socket on a
/// short interval until the queue closes (the loop dropped its sender at exit) or a
/// write fails.
///
/// The `req_rx` borrow is confined to the synchronous collect step (no `.await` while
/// it is held), then the batch is written — the same collect-then-write that keeps
/// the future `Send` despite `std::sync::mpsc::Receiver` being `!Sync` (see
/// `conn::write_loop`).
///
/// # Drain-on-close (final-frame guarantee)
///
/// When `try_recv` reports `Disconnected` the loop has dropped `req_tx` at teardown,
/// after queuing the shutdown frame(s) (`Detach`, and — on `/quit` `[k]` — a
/// `QuitDaemon` ahead of it). Those frames may still be sitting in the channel, so
/// this task does NOT bail on close: it collects EVERY remaining request in the same
/// pass (the `Disconnected` arm only stops the collect, it does not discard what was
/// already drained) and writes the full batch — `write_frame_to` flushes each frame —
/// BEFORE returning. The teardown joins this task (bounded) so the runtime is not
/// dropped until this final flush completes, which is what guarantees the daemon
/// actually receives `QuitDaemon` instead of being orphaned.
pub(super) async fn writer_task(
    mut write_half: tokio::net::unix::OwnedWriteHalf,
    req_rx: Receiver<ClientRequest>,
) {
    let mut poll = tokio::time::interval(REQ_POLL);
    loop {
        poll.tick().await;

        // Collect every queued request WITHOUT awaiting while `req_rx` is borrowed.
        // On `Disconnected` keep everything drained so far (the final shutdown
        // frames) and write them below — closing the channel must never drop a
        // queued request, only end the polling loop after this last flush.
        let mut batch: Vec<ClientRequest> = Vec::new();
        let mut closed = false;
        loop {
            match req_rx.try_recv() {
                Ok(req) => batch.push(req),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true; // loop exited and dropped its sender
                    break;
                }
            }
        }

        // Write the batch (does not touch `req_rx`). Stop on a dead socket.
        // `write_frame_to` flushes each frame, so a successful write is on the wire.
        for req in &batch {
            let bytes = match serde_json::to_vec(req) {
                Ok(b) => b,
                // A request that can't serialise is a client bug, not a transport
                // fault — skip it rather than tear down the connection.
                Err(_) => continue,
            };
            if frame::write_frame_to(&mut write_half, &bytes).await.is_err() {
                return; // dead socket
            }
        }
        // Channel closed AND the final drained batch is flushed: the shutdown
        // frame(s) are on the wire, so it is safe to return (the teardown join then
        // completes and the runtime is dropped).
        if closed {
            break;
        }
    }
}

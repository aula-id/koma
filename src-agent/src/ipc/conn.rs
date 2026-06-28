//! Per-client connection task — the critique #6 bridge between async socket I/O
//! and the SYNCHRONOUS [`daemon_loop`](crate::app::runtime).
//!
//! `daemon_loop` is sync (it `try_recv`s, it `thread::sleep`s) and MUST stay sync.
//! All per-client socket I/O therefore lives here in a tokio task that talks to the
//! loop over plain `std::sync::mpsc` channels carried by the
//! [`DaemonHub`](crate::app::runtime): inbound `ClientRequest`s are forwarded as
//! [`HubInbound`](crate::app::runtime) messages on the hub's request channel (which
//! the loop drains each tick), and outbound seq-tagged [`DaemonFrame`]s arrive on
//! this client's own frame channel (which the loop fans out onto).
//!
//! # Lifecycle
//!
//! [`spawn`] is called by the accept loop on each accepted [`UnixStream`]. It:
//! 1. creates this client's `std::sync::mpsc::channel::<DaemonFrame>()`,
//! 2. hands the loop the matching [`HubInbound::Register`] (frame *sender*) BEFORE
//!    spawning the task, so the client is enrolled before any frame could be sent,
//! 3. spawns a tokio task that splits the stream into independent read/write halves
//!    and drives both concurrently:
//!    - **read half**: `read_frame_from` (which enforces [`MAX_FRAME_BYTES`] on every
//!      length prefix, critique #4) -> decode `ClientRequest` -> forward as
//!      [`HubInbound::Request`]. On socket EOF / decode error it stops and signals
//!      [`HubInbound::Disconnect`] so the hub deregisters this client (+ passes the
//!      controller seat).
//!    - **write half**: owns this client's frame *receiver* and polls it on a short
//!      tokio interval, writing each [`DaemonFrame`] to the socket. The sync→async
//!      handoff is the interval poll: the sender side lives on the sync loop, so a
//!      blocking `recv()` here would peg a tokio worker — instead we `try_recv` on a
//!      tick and yield between ticks.
//!
//! # Two independent tasks (not one `select!`)
//!
//! The read and write halves run as SEPARATE `tokio::spawn`ed tasks rather than two
//! `select!` branches of one task. The reason is `Send`: the outbound queue is a
//! `std::sync::mpsc::Receiver` (its sender lives on the sync loop), which is `!Sync`,
//! so a `&Receiver` held across an `.await` makes a future non-`Send` and
//! `tokio::spawn` rejects it. A `select!` keeps both branch futures (and thus the
//! receiver borrow) alive across the other branch's await; two independent tasks do
//! not. The write task OWNS the receiver (a `Receiver` is `Send`) and only borrows
//! it inside a non-`await` drain phase, so its future is `Send`.
//!
//! The two tasks coordinate through the hub: when the peer closes the read half, the
//! read task signals [`HubInbound::Disconnect`]; the hub deregisters the client and
//! DROPS its frame sender; the write task's next `try_recv` then returns
//! `Disconnected` and it exits. So a peer hangup tears down both halves with a single
//! `Disconnect`.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use crate::app::runtime::HubInbound;
use crate::ipc::frame::{self, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonFrame};

/// How often the write half polls its (sync) frame receiver. 4ms keeps streamed
/// tokens flushing to the socket at >=60fps (matching the loop's busy 8ms cadence)
/// while a tick between polls yields the tokio worker so an idle client costs ~0.
const FRAME_POLL: Duration = Duration::from_millis(4);

/// Spawn the per-client connection task for an accepted `stream`.
///
/// `client_id` is the loop-assigned connection id (the accept loop's monotonic
/// counter). `hub_tx` is the hub's request channel (cloned per client). Registers
/// this client with the hub (handing it the frame sender) BEFORE returning, so the
/// task is enrolled the instant it starts; then spawns the I/O task on the ambient
/// tokio runtime (the caller runs inside the daemon runtime).
pub fn spawn(stream: UnixStream, client_id: u64, hub_tx: Sender<HubInbound>) {
    // This client's outbound frame channel: the hub holds the sender (enrolled via
    // Register below), this task owns the receiver and writes frames to the socket.
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    // Enrol BEFORE spawning so no frame can be produced for an unknown client. A
    // send error means the loop is already gone (daemon shutting down) — then there
    // is nothing to serve, so drop the connection without spawning.
    if hub_tx
        .send(HubInbound::Register {
            client_id,
            frame_tx,
        })
        .is_err()
    {
        return;
    }

    // Split into independent halves: a single non-split `UnixStream` can't be
    // `&mut`-borrowed for read and write at once, and the two tasks must run
    // concurrently. Each half owns its end for the connection's lifetime.
    let (read_half, write_half) = stream.into_split();

    // Read task: forward requests, signal Disconnect on EOF/error.
    let read_hub_tx = hub_tx.clone();
    tokio::spawn(async move {
        read_loop(read_half, client_id, read_hub_tx).await;
    });

    // Write task: drain this client's frame queue to the socket until the queue
    // closes (hub dropped the sender) or a write fails.
    tokio::spawn(async move {
        write_loop(write_half, frame_rx).await;
    });
}

/// Read framed [`ClientRequest`]s from `read_half` and forward each as a
/// [`HubInbound::Request`]. On socket EOF / cap violation / decode error, signal
/// [`HubInbound::Disconnect`] exactly once and end. `read_frame_from` enforces
/// [`MAX_FRAME_BYTES`](crate::ipc::proto::MAX_FRAME_BYTES) on every length prefix.
async fn read_loop(mut read_half: OwnedReadHalf, client_id: u64, hub_tx: Sender<HubInbound>) {
    use std::ops::ControlFlow;

    let mut reader = FrameReader::new();
    // Loop until a frame read / decode / forward signals it's done. Kept as an
    // explicit `loop` (not `while let`) because the post-loop `Disconnect` MUST fire
    // on every exit path, including the malformed-frame and loop-gone cases.
    loop {
        let step: ControlFlow<()> =
            match frame::read_frame_from(&mut read_half, &mut reader).await {
                Ok(bytes) => match serde_json::from_slice::<ClientRequest>(&bytes) {
                    // Forward the request; stop only if the loop is gone.
                    Ok(req) => {
                        if hub_tx
                            .send(HubInbound::Request { client_id, req })
                            .is_ok()
                        {
                            ControlFlow::Continue(())
                        } else {
                            ControlFlow::Break(())
                        }
                    }
                    // A malformed frame is a protocol error on this connection; drop
                    // it rather than guess at intent (the Disconnect below cleans up).
                    Err(_) => ControlFlow::Break(()),
                },
                // EOF, cap violation (MAX_FRAME_BYTES), or any read error: done.
                Err(_) => ControlFlow::Break(()),
            };
        if step.is_break() {
            break;
        }
    }
    // Single deregister signal. The hub treats an unknown id (already removed via an
    // explicit Detach) as a harmless no-op, so racing a Detach with EOF is safe.
    let _ = hub_tx.send(HubInbound::Disconnect { client_id });
}

/// Drain this client's [`DaemonFrame`] queue to `write_half` on a short interval,
/// until the queue closes (the hub dropped the sender — e.g. the client was
/// deregistered) or a socket write fails.
///
/// The `frame_rx` borrow is confined to the synchronous collect step (no `.await`
/// while it is held); the collected batch is then written in [`write_batch`], which
/// never touches `frame_rx`. This is what keeps `write_loop`'s future `Send`: a
/// `std::sync::mpsc::Receiver` is `!Sync`, so a `&Receiver` held across ANY await
/// (including one nested inside a sub-future captured across this loop's await)
/// makes the spawned future non-`Send`. Collect-then-write keeps the receiver off
/// every await point while still flushing a whole token burst within one tick.
async fn write_loop(mut write_half: OwnedWriteHalf, frame_rx: Receiver<DaemonFrame>) {
    let mut poll = tokio::time::interval(FRAME_POLL);
    loop {
        poll.tick().await;

        // Collect every queued frame WITHOUT awaiting while `frame_rx` is borrowed.
        let mut batch: Vec<DaemonFrame> = Vec::new();
        let mut closed = false;
        loop {
            match frame_rx.try_recv() {
                Ok(frame) => batch.push(frame),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true; // hub dropped the sender (client deregistered)
                    break;
                }
            }
        }

        // Write the batch (does not touch `frame_rx`). Stop on a dead socket.
        if !write_batch(&mut write_half, &batch).await {
            break;
        }
        if closed {
            break;
        }
    }
}

/// Serialise + write each frame in `batch`. Returns `false` on the first socket
/// write error (dead client). A frame that can't serialise is a daemon bug, not a
/// transport fault — skip it rather than killing the connection.
async fn write_batch(write_half: &mut OwnedWriteHalf, batch: &[DaemonFrame]) -> bool {
    for frame in batch {
        let bytes = match serde_json::to_vec(frame) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if frame::write_frame_to(write_half, &bytes).await.is_err() {
            return false;
        }
    }
    true
}

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame};

use super::bridge::{reader_task, writer_task};

/// How long the pre-render build-skew handshake waits for the daemon's first
/// [`DaemonEvent::Hello`] frame before giving up and proceeding UNVERIFIED (task
/// #142). Generous relative to the daemon's sub-ms attach reply, but bounded so a
/// wedged / pre-Hello daemon can never hang the client before it even paints. On a
/// timeout the client renders against whatever daemon answered (it never restarts on
/// a mere absence — only on a CONFIRMED mismatch).
pub(super) const HELLO_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// One live daemon connection: the bridge channels + writer join handle the render
/// loop and teardown drive, plus the frames the pre-render handshake already pulled
/// off the wire and the daemon version it observed.
pub(super) struct Connection {
    /// Incoming daemon frames (reader task -> render loop).
    pub(super) frame_rx: std::sync::mpsc::Receiver<DaemonFrame>,
    /// Outgoing client requests (render loop -> writer task).
    pub(super) req_tx: Sender<ClientRequest>,
    /// Writer task handle, joined at teardown so the final `Detach`/`QuitDaemon`
    /// flushes before the runtime is dropped.
    pub(super) writer_handle: tokio::task::JoinHandle<()>,
    /// Frames the handshake read off `frame_rx` while hunting for `Hello` (normally
    /// none — `Hello` is the first frame — but any that arrived first are carried here
    /// so the render loop applies them BEFORE its own drain and no frame/seq is lost).
    pub(super) prebuffered: Vec<DaemonFrame>,
    /// The daemon's reported build fingerprint, or `None` if no `Hello` arrived within
    /// the handshake window (a daemon predating the handshake, or a slow one).
    pub(super) daemon_version: Option<String>,
}

/// Connect to the daemon, spawn the I/O bridge, send `Attach`, and run the pre-render
/// build-skew handshake (task #142): read frames until the daemon's first
/// [`DaemonEvent::Hello`] (bounded by [`HELLO_HANDSHAKE_TIMEOUT`]), recording its
/// reported fingerprint. Returns a live [`Connection`]; the CALLER compares
/// `daemon_version` to its own fingerprint and decides whether to restart+reconnect.
///
/// The handshake is synchronous and runs BEFORE any terminal setup so a stale-daemon
/// restart happens cleanly on the normal screen. Frames that arrive ahead of `Hello`
/// (defensive — the daemon emits `Hello` first) are stashed in `prebuffered` for the
/// render loop to apply first, so the seq stream the loop sees stays gap-free.
pub(super) fn connect_attach_and_handshake(
    handle: &tokio::runtime::Handle,
    sock_path: &std::path::Path,
) -> Result<Connection> {
    // Connect first so a missing daemon fails BEFORE we touch the terminal (no
    // alt-screen flash on "no daemon"). The connected stream is split into the two
    // task halves below.
    let stream = handle
        .block_on(async { crate::ipc::client::connect(sock_path).await })
        .map_err(|e| {
            anyhow::anyhow!("could not reach koma daemon at {}: {e}", sock_path.display())
        })?;

    // Bridge channels: incoming frames (daemon -> loop) and outgoing requests
    // (loop -> daemon). Mirrors the daemon hub's bridge, client-side.
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();
    let (req_tx, req_rx) = std::sync::mpsc::channel::<ClientRequest>();

    // Split + spawn the two I/O tasks on the runtime (a tokio reactor must be in
    // scope for `into_split` + `spawn`). The writer's `JoinHandle` is kept so the
    // teardown can WAIT for it to flush its final frame(s) (the shutdown
    // `QuitDaemon`/`Detach`) before the runtime is dropped — see below.
    let writer_handle = {
        let _enter = handle.enter();
        let (read_half, write_half) = stream.into_split();
        handle.spawn(reader_task(read_half, frame_tx));
        handle.spawn(writer_task(write_half, req_rx))
    };

    // Send the Attach handshake; the daemon answers with a `Hello` (build-skew
    // fingerprint) FOLLOWED by the initial full Snapshot. Carry THIS client's launch
    // cwd so the daemon does pwd-aware session selection (stage 3): launching from a
    // NEW dir foregrounds/loads/creates a session for THAT dir, not the daemon's last
    // one. `current_dir` failing is non-fatal — `None` just keeps the daemon's foreground.
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let _ = req_tx.send(ClientRequest::Attach {
        foreground_id: None,
        cwd,
    });

    // Pre-render handshake: pull frames until the daemon's `Hello` (bounded). `Hello`
    // is normally the very first frame, so this typically reads exactly one. Any
    // non-`Hello` frame seen first is buffered for the render loop (so nothing is lost
    // and the seq stays monotonic). A timeout / closed socket ends the wait with
    // `daemon_version = None` — the caller proceeds unverified rather than restarting.
    let mut prebuffered: Vec<DaemonFrame> = Vec::new();
    let mut daemon_version: Option<String> = None;
    let deadline = Instant::now() + HELLO_HANDSHAKE_TIMEOUT;
    // Loop until the Hello arrives, the socket closes, or the window elapses
    // (`checked_duration_since` returns `None` once `deadline` is in the past).
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match frame_rx.recv_timeout(remaining) {
            Ok(frame) => match frame.event {
                DaemonEvent::Hello { version } => {
                    daemon_version = Some(version);
                    break;
                }
                // A non-Hello frame arrived first: keep it for the render loop to apply
                // before its own drain, then keep waiting for the Hello.
                _ => prebuffered.push(frame),
            },
            // Timed out, or the reader task dropped its sender (socket closed): stop
            // waiting. `None` daemon_version => unverified; the caller won't restart.
            Err(_) => break,
        }
    }

    Ok(Connection {
        frame_rx,
        req_tx,
        writer_handle,
        prebuffered,
        daemon_version,
    })
}

//! Per-extension host transport: bind the socket, spawn + handshake the child, and
//! the reader/writer tasks over the DUPLEX unix-socket connection.
//!
//! Split out of [`super`] (the `ext` module) for file size — the connection lifecycle
//! and the two long-lived I/O tasks live here; the manager's bookkeeping lives in
//! `mod.rs`. Mirrors the security daemon's `wire` module, but the frames flow over a
//! [`tokio::net::UnixStream`] (koma binds, the child connects) instead of the child's
//! stdio, and the link is DUPLEX: koma `Invoke`s the extension AND the extension
//! `Call`s back into koma on the same connection.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};

use koma_extension::protocol::{ExtMsg, KomaMsg, PROTOCOL_VERSION};

use super::install;
use super::{ExtCallRequest, ExtHostManager, ExtNotify, PendingMap, CONNECT_TIMEOUT};

/// Hard cap on a single newline-delimited frame — the handshake `Hello` line AND
/// every steady-state `ExtMsg`/`KomaMsg` line. Both read sites buffer one line
/// before parsing it; without a cap a runaway or hostile child could force an
/// unbounded in-memory allocation (memory DoS) just by never sending a `\n`. 4 MiB
/// comfortably covers any real manifest/result payload while still being a hard
/// stop.
pub(super) const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// How long the reader task waits for the event loop's grant broker to answer one
/// `agents.*` `Call` before replying an error to the extension (so a stalled/absent
/// drain can never leave the extension's `call()` hanging). Comfortably shorter than
/// the extension SDK's own 120s `host_call` timeout, and generous versus the broker's
/// real cost (one event-loop tick — the broker itself never blocks).
const EXT_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Why [`read_capped_line`] failed: distinguishes "the frame is too big" (fatal —
/// the caller should kill the connection/child rather than keep reading a desynced
/// stream) from any other I/O error.
#[derive(Debug)]
pub(super) enum FrameReadError {
    /// The frame exceeded the cap before a `\n` was seen.
    TooLarge,
    /// Any other read failure (I/O error, EOF mid-frame, invalid UTF-8).
    Other(anyhow::Error),
}

impl std::fmt::Display for FrameReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameReadError::TooLarge => write!(f, "frame exceeds {MAX_FRAME_BYTES} bytes"),
            FrameReadError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Read one newline-delimited frame from `reader`, capped at `cap` bytes.
///
/// Accumulates bytes ONE AT A TIME (no unbounded internal buffer) until a `\n` is
/// seen or `cap` is exceeded, so a line longer than `cap` is caught the moment it
/// crosses the limit rather than after however much of it happened to already be
/// buffered. Returns `Ok(None)` on a clean EOF before any bytes (the normal
/// "child closed the connection" case); `Ok(Some(line))` (the line WITHOUT its
/// trailing `\n`/`\r\n`) on success; [`FrameReadError::TooLarge`] if the cap is
/// exceeded; [`FrameReadError::Other`] for any other I/O failure (including EOF
/// mid-frame, which is always a protocol violation, never a clean close).
pub(super) async fn read_capped_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<Option<String>, FrameReadError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = reader
            .read(&mut byte)
            .await
            .map_err(|e| FrameReadError::Other(anyhow!("frame read failed: {e}")))?;
        if n == 0 {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(FrameReadError::Other(anyhow!(
                    "frame read failed: EOF mid-frame"
                )))
            };
        }
        if byte[0] == b'\n' {
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            let s = String::from_utf8(buf)
                .map_err(|e| FrameReadError::Other(anyhow!("frame is not valid utf8: {e}")))?;
            return Ok(Some(s));
        }
        buf.push(byte[0]);
        if buf.len() > cap {
            return Err(FrameReadError::TooLarge);
        }
    }
}

/// The product of a successful bind + spawn + handshake: the live child, the read
/// half wrapped in a `BufReader` (continued into the reader task so any bytes buffered
/// past the `Hello` line are not lost), and the write half for the writer task.
pub(super) struct Handshaked {
    pub(super) child: tokio::process::Child,
    pub(super) reader: BufReader<OwnedReadHalf>,
    pub(super) write_half: OwnedWriteHalf,
}

/// Bind `sock_path`, spawn the extension child (env `KOMA_EXT_SOCKET` +
/// `KOMA_EXT_TOKEN`, `kill_on_drop`), accept its one inbound connection, read + validate
/// its `Hello`, and reply `Welcome`. On ANY failure the child (if spawned) is
/// `start_kill`'d before returning `Err`.
///
/// `exec_rel` is resolved against `install_dir` (the unpacked `extensions/<id>/`), which
/// is also the child's working directory. The accept + `Hello` read are each bounded by
/// [`CONNECT_TIMEOUT`], so a child that never connects can't hang the caller.
pub(super) async fn connect_and_handshake(
    sock_path: &Path,
    install_dir: &Path,
    exec_rel: &str,
    token: &str,
) -> Result<Handshaked> {
    // Bind FIRST (removes a stale socket file, creates the parent dir) so the child has
    // something to connect to. Reuses the daemon's bind helper (bind-is-liveness-oracle).
    let listener = crate::ipc::server::bind(sock_path)
        .map_err(|e| anyhow!("bind ext socket {}: {e}", sock_path.display()))?;

    // Spawn the child. exec is relative to the install dir; cwd = install dir. stdout is
    // discarded and stderr is drained to the global error log (TUI-safe — no eprintln).
    //
    // `exec_rel` is re-read from the persisted `config.json` at every boot (unlike the
    // install-time check in `install::unpack`), so it is validated again here —
    // defense in depth against a hand-edited/corrupted registry entry pointing the
    // spawn anywhere on disk via an absolute path or a `..` escape.
    let exec_path = match install::safe_exec_rel(exec_rel, install_dir) {
        Ok(p) => p,
        Err(e) => bail!("ext exec path rejected: {e:#}"),
    };
    let mut child = tokio::process::Command::new(&exec_path)
        .current_dir(install_dir)
        .env("KOMA_EXT_SOCKET", sock_path)
        .env("KOMA_EXT_TOKEN", token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow!("spawn extension {}: {e}", exec_path.display()))?;

    if let Some(stderr) = child.stderr.take() {
        let id = install_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        tokio::spawn(stderr_log_task(stderr, id));
    }

    // Accept exactly one connection from the child.
    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, listener.accept()).await {
        Ok(Ok((stream, _addr))) => stream,
        Ok(Err(e)) => {
            let _ = child.start_kill();
            bail!("ext accept failed: {e}");
        }
        Err(_) => {
            let _ = child.start_kill();
            bail!(
                "extension did not connect within {}s",
                CONNECT_TIMEOUT.as_secs()
            );
        }
    };

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // Read the first line = the Hello frame, capped so a runaway/hostile child can't
    // force an unbounded allocation before the handshake even completes.
    let line = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        read_capped_line(&mut reader, MAX_FRAME_BYTES),
    )
    .await
    {
        Ok(Ok(Some(l))) => l,
        Ok(Ok(None)) => {
            let _ = child.start_kill();
            bail!("extension closed the connection before Hello");
        }
        Ok(Err(FrameReadError::TooLarge)) => {
            let _ = child.start_kill();
            bail!("ext Hello frame exceeds {MAX_FRAME_BYTES} bytes");
        }
        Ok(Err(FrameReadError::Other(e))) => {
            let _ = child.start_kill();
            bail!("ext handshake read failed: {e}");
        }
        Err(_) => {
            let _ = child.start_kill();
            bail!("ext handshake timed out");
        }
    };

    let hello: ExtMsg = match serde_json::from_str(line.trim()) {
        Ok(m) => m,
        Err(e) => {
            let _ = child.start_kill();
            bail!("ext Hello was not valid JSON: {e}");
        }
    };

    // Validate token + protocol; the echoed grants come from the Hello manifest.
    let granted = match hello {
        ExtMsg::Hello {
            protocol,
            token: got_token,
            manifest,
        } => {
            if protocol != PROTOCOL_VERSION {
                let reason =
                    format!("protocol mismatch: expected {PROTOCOL_VERSION}, got {protocol}");
                let _ = send_frame(&mut write_half, &KomaMsg::Reject { reason }).await;
                let _ = child.start_kill();
                bail!("ext protocol mismatch: got {protocol}");
            }
            if got_token != token {
                let _ = send_frame(
                    &mut write_half,
                    &KomaMsg::Reject {
                        reason: "token mismatch".to_string(),
                    },
                )
                .await;
                let _ = child.start_kill();
                bail!("ext token mismatch");
            }
            manifest.requires
        }
        _ => {
            let _ = send_frame(
                &mut write_half,
                &KomaMsg::Reject {
                    reason: "expected Hello as the first frame".to_string(),
                },
            )
            .await;
            let _ = child.start_kill();
            bail!("ext first frame was not Hello");
        }
    };

    // Handshake OK → Welcome. `granted` echoes the manifest's `requires` back to
    // the extension for its own info — the REAL grant enforcement lives in the
    // event loop's grant broker (`app::ext::broker::handle_ext_call`, gated via
    // `ExtHostManager::granted_for`), which re-checks every `agents.*` call
    // against what koma persisted, not what this handshake echoed.
    let welcome = KomaMsg::Welcome {
        protocol: PROTOCOL_VERSION.to_string(),
        koma_version: crate::model::store::current_version().to_string(),
        granted,
    };
    if let Err(e) = send_frame(&mut write_half, &welcome).await {
        let _ = child.start_kill();
        bail!("ext Welcome write failed: {e}");
    }

    Ok(Handshaked {
        child,
        reader,
        write_half,
    })
}

/// Queue a `KomaMsg::Result { id, result }` (reply to an ext→koma `Call`) onto the
/// writer task's channel as one newline-delimited frame. Best-effort: a closed
/// writer (child gone) simply drops it. Shared by every reply site in the reader
/// task's `Call` handling so the framing/serialization lives in one place.
fn send_result_frame(writer: &mpsc::UnboundedSender<String>, id: u64, result: serde_json::Value) {
    if let Ok(mut s) = serde_json::to_string(&KomaMsg::Result { id, result }) {
        s.push('\n');
        let _ = writer.send(s);
    }
}

/// Serialize `msg` as one newline-delimited JSON frame and write+flush it to `w`.
async fn send_frame<T: serde::Serialize>(w: &mut OwnedWriteHalf, msg: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

/// Owns the connection's write half and forwards every frame received on `rx`
/// (each already newline-terminated) to it. Exits when the channel closes (the
/// manager dropped the `writer` on stop) or on the first write error (child gone),
/// shutting the write half on the way out so the child observes EOF.
pub(super) async fn writer_task(mut write_half: OwnedWriteHalf, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(frame) = rx.recv().await {
        if write_half.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        if write_half.flush().await.is_err() {
            break;
        }
    }
    let _ = write_half.shutdown().await;
}

/// Owns the connection's read half and dispatches inbound [`ExtMsg`] frames:
///
/// - `Result{id,result}` completes (and removes) the matching pending koma→ext
///   `Invoke` with `Ok(result)`.
/// - `Call{id,method,params}` is an ext→koma request. An `agents.*` method is routed
///   to the grant broker: it is packaged into an [`ExtCallRequest`] and forwarded to
///   the event loop over `ext_call_tx` (the only place with `AppState` access), gated
///   there by this extension's granted scopes; the broker's reply is written back
///   (same `id`) from a detached task so this loop keeps reading. Any other method
///   (and a channel-closed / timeout on an `agents.*` call) gets a uniform error
///   `KomaMsg::Result` so the extension's `call()` always unblocks.
/// - `Health{ok}` updates the entry's liveness flag.
/// - a post-handshake `Hello` is unexpected and ignored.
///
/// On EOF/read-error every still-pending caller is failed (so no `invoke` hangs) and
/// the manager is flipped to stopped (generation-guarded so a superseding start is
/// never clobbered). A frame over [`MAX_FRAME_BYTES`] is treated as fatal misbehavior:
/// the extension is `stop()`-ed (killing the child) immediately rather than left to
/// keep flooding a connection nobody will finish reading.
pub(super) async fn reader_task(
    mut reader: BufReader<OwnedReadHalf>,
    pending: PendingMap,
    writer: mpsc::UnboundedSender<String>,
    mgr: Arc<ExtHostManager>,
    ext_id: String,
    gen: u64,
) {
    loop {
        match read_capped_line(&mut reader, MAX_FRAME_BYTES).await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let msg: ExtMsg = match serde_json::from_str(trimmed) {
                    Ok(m) => m,
                    Err(e) => {
                        crate::model::store::append_global_error_log(
                            "extensions",
                            &format!("[{ext_id}] ignoring unparseable frame: {e}"),
                        );
                        continue;
                    }
                };
                match msg {
                    ExtMsg::Result { id, result } => {
                        let tx = {
                            let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
                            map.remove(&id)
                        };
                        if let Some(tx) = tx {
                            let _ = tx.send(Ok(result));
                        }
                    }
                    ExtMsg::Call { id, method, params } => {
                        if method.starts_with("agents.") {
                            // The grant broker (`agents.*`): the reader task has no
                            // `AppState`/session access, so hand the call off to the
                            // event loop via `ext_call_tx` — gated there by THIS
                            // extension's granted scopes — and reply with the broker's
                            // Value once it answers. The await happens on a DETACHED
                            // task so this loop keeps draining the socket (a reply
                            // carries the same `id`, so out-of-order replies are fine);
                            // on channel-closed / timeout it still replies an error so
                            // the extension's `call()` never hangs.
                            match mgr.ext_call_tx() {
                                Some(tx) => {
                                    let granted = mgr.granted_for(&ext_id);
                                    let (reply_tx, reply_rx) = oneshot::channel::<serde_json::Value>();
                                    let req = ExtCallRequest {
                                        ext_id: ext_id.clone(),
                                        granted,
                                        method,
                                        params,
                                        reply: reply_tx,
                                    };
                                    if tx.send(req).is_err() {
                                        // Event loop receiver gone (shutdown).
                                        send_result_frame(
                                            &writer,
                                            id,
                                            serde_json::json!({ "error": "grant broker unavailable" }),
                                        );
                                    } else {
                                        let writer_reply = writer.clone();
                                        tokio::spawn(async move {
                                            let result = match tokio::time::timeout(
                                                EXT_CALL_TIMEOUT,
                                                reply_rx,
                                            )
                                            .await
                                            {
                                                Ok(Ok(v)) => v,
                                                Ok(Err(_)) => serde_json::json!({
                                                    "error": "grant broker dropped request"
                                                }),
                                                Err(_) => serde_json::json!({
                                                    "error": "grant broker timed out"
                                                }),
                                            };
                                            send_result_frame(&writer_reply, id, result);
                                        });
                                    }
                                }
                                None => {
                                    // Broker not wired (pre-startup / test manager).
                                    send_result_frame(
                                        &writer,
                                        id,
                                        serde_json::json!({ "error": "grant broker not initialized" }),
                                    );
                                }
                            }
                        } else {
                            // Non-`agents.*` ext→koma methods keep the uniform stub.
                            send_result_frame(
                                &writer,
                                id,
                                serde_json::json!({ "error": format!("unknown koma method: {method}") }),
                            );
                        }
                    }
                    ExtMsg::Health { ok } => {
                        mgr.note_health(&ext_id, ok, gen);
                    }
                    ExtMsg::Hello { .. } => {
                        // Unexpected once past the handshake; ignore.
                    }
                    ExtMsg::Notify { name, params } => {
                        // Fire-and-forget ext->koma notification: no `id`, no
                        // `Result` reply expected. Hand it off to the event loop
                        // (which has the `AppState` access this reader task
                        // lacks — real dispatch, e.g. routing `panel.push` to the
                        // panel bridge, is wired in a later wave) via
                        // `ext_notify_tx`. If it isn't wired yet (tests /
                        // pre-startup), drop the frame silently — there is no
                        // reply to fail back to the extension either way. Never
                        // spawns, never awaits: this loop keeps reading at once.
                        if let Some(tx) = mgr.ext_notify_tx() {
                            let _ = tx.send(ExtNotify {
                                ext_id: ext_id.clone(),
                                name,
                                params,
                            });
                        }
                    }
                }
            }
            Ok(None) => break, // EOF: child closed the connection
            Err(FrameReadError::TooLarge) => {
                crate::model::store::append_global_error_log(
                    "extensions",
                    &format!(
                        "[{ext_id}] frame exceeds {MAX_FRAME_BYTES} bytes; killing extension"
                    ),
                );
                // Kill immediately rather than let a misbehaving/hostile child keep
                // flooding a connection this loop is about to stop reading from.
                mgr.stop(&ext_id);
                break;
            }
            Err(FrameReadError::Other(e)) => {
                crate::model::store::append_global_error_log(
                    "extensions",
                    &format!("[{ext_id}] error reading ext socket: {e}"),
                );
                break;
            }
        }
    }

    // Child is gone: fail every remaining waiter so no `invoke` hangs the full timeout.
    {
        let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
        for (_id, tx) in map.drain() {
            let _ = tx.send(Err("extension closed connection".to_string()));
        }
    }

    // Flip the manager to stopped (generation-guarded) so `is_running` reads false.
    mgr.mark_stopped(&ext_id, gen);
}

/// Drain the child's stderr to EOF and, if non-empty, append it to the global error
/// log tagged with the extension id. TUI-safe (never prints to the terminal), and a
/// full stderr pipe can never block the child.
async fn stderr_log_task(mut stderr: tokio::process::ChildStderr, id: String) {
    let mut buf = Vec::new();
    if stderr.read_to_end(&mut buf).await.is_ok() && !buf.is_empty() {
        let text = String::from_utf8_lossy(&buf);
        crate::model::store::append_global_error_log("extensions", &format!("[{id}] stderr: {text}"));
    }
}

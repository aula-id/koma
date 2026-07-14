//! Sync wire helpers for the [`McpBackend::Proxy`](super::McpBackend) backend:
//! connect-per-call to the global MCP daemon's socket, write one request frame,
//! block for one response frame. Split out of [`super`] (the `mcp` module) for
//! file size; `proxy_request` is bumped to `pub(super)` (it is called from
//! several `McpManager` methods in the parent module) — `proxy_send`/
//! `proxy_recv` stay private (only used by `proxy_request`, in this same
//! file). No behaviour change.

use std::io::{Read, Write};

use crate::ipc::frame::FrameReader;
use crate::ipc::mcp_proto::{McpRequest, McpResponse};
use crate::ipc::SyncIpcStream as StdUnixStream;

use super::PROXY_IO_TIMEOUT;

/// Send ONE [`McpRequest`] to the global MCP daemon at `sock` and block until its
/// single [`McpResponse`] frame arrives (or the read times out).
///
/// The sync twin of the async accept loop's per-request cycle: a fresh blocking
/// [`std::os::unix::net::UnixStream`] with [`PROXY_IO_TIMEOUT`] read/write timeouts,
/// one length-prefixed JSON frame written (the SAME 4-byte-BE-len codec
/// [`crate::ipc::frame`] defines), then one frame read back and decoded. Connect-
/// per-call keeps it simple and robust — no long-lived connection state to manage,
/// no shared mutable stream, and the daemon already parallelises across connections.
///
/// Runtime-free (plain std sockets), so it is safe to call from the synchronous tool
/// dispatch thread whether or not a tokio runtime is in scope. Every failure
/// (connect refused, write/read IO error, timeout, decode error) is surfaced as an
/// `Err` the caller maps to a model-facing tool error or (for `connect_proxy`) a
/// fallback trigger.
pub(super) fn proxy_request(sock: &std::path::Path, req: &McpRequest) -> anyhow::Result<McpResponse> {
    use anyhow::Context;

    // Connect (blocking). A refused/absent socket means the daemon isn't accepting.
    let mut stream = StdUnixStream::connect(sock)
        .with_context(|| format!("connect to global MCP daemon socket {}", sock.display()))?;
    // Bound both directions so a wedged daemon can never hang the tool thread. The
    // read timeout is the primary guard (a slow tool); the write side is naturally
    // tiny but is bounded for symmetry.
    stream
        .set_read_timeout(Some(PROXY_IO_TIMEOUT))
        .context("set MCP proxy read timeout")?;
    stream
        .set_write_timeout(Some(PROXY_IO_TIMEOUT))
        .context("set MCP proxy write timeout")?;

    proxy_send(&mut stream, req)?;
    proxy_recv(&mut stream)
}

/// Write one [`McpRequest`] to `stream` as a length-prefixed JSON frame (4-byte
/// big-endian payload length + payload — the shared [`crate::ipc::frame`] codec).
/// The sync `McpRequest` twin of [`crate::app::runtime`]'s `send_request`.
fn proxy_send(stream: &mut StdUnixStream, req: &McpRequest) -> anyhow::Result<()> {
    use anyhow::Context;
    let payload = serde_json::to_vec(req).context("serialise McpRequest")?;
    let prefix = (payload.len() as u32).to_be_bytes();
    stream.write_all(&prefix).context("write MCP frame prefix")?;
    stream.write_all(&payload).context("write MCP frame payload")?;
    stream.flush().context("flush MCP frame")?;
    Ok(())
}

/// Block until ONE complete [`McpResponse`] frame arrives on `stream`, reassembling
/// via the shared [`FrameReader`] (so a frame split across reads — or coalesced with
/// a following one — is handled identically to the async path). The stream's read
/// timeout bounds the wait. The sync `McpResponse` twin of [`crate::app::runtime`]'s
/// `recv_frame`.
fn proxy_recv(stream: &mut StdUnixStream) -> anyhow::Result<McpResponse> {
    use anyhow::{anyhow, Context};
    let mut reader = FrameReader::new();
    loop {
        // A previous read may have buffered a whole frame already.
        if let Some(bytes) = reader.next_frame().context("MCP frame reassembly")? {
            return serde_json::from_slice(&bytes).context("decode McpResponse");
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).context("read from global MCP daemon socket")?;
        if n == 0 {
            return Err(anyhow!("global MCP daemon closed the connection mid-frame"));
        }
        reader.push(&chunk[..n]);
    }
}

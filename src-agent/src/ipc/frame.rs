//! Length-prefixed framing — the ONE shared wire codec used by BOTH the daemon
//! server and the TUI client.
//!
//! # Wire format (LOCKED)
//!
//! ```text
//! ┌────────────────────────┬───────────────────────────────┐
//! │ 4 bytes, BIG-ENDIAN u32 │  UTF-8 JSON payload (N bytes)  │
//! │   length prefix = N     │                               │
//! └────────────────────────┴───────────────────────────────┘
//! ```
//!
//! The prefix counts the PAYLOAD bytes only (it does not include itself). A
//! single frame carries exactly one serde-JSON value ([`super::proto`] types).
//!
//! # Partial reads
//!
//! A frame can be split across multiple `recv`s (and several frames can arrive in
//! one `recv`). [`FrameReader`] owns an accumulation buffer: callers feed it raw
//! bytes with [`FrameReader::push`] and pull out whole frames with
//! [`FrameReader::next_frame`], which yields `Ok(None)` until a full frame is
//! buffered. [`read_frame`] wraps that pull-from-socket loop for the common case.
//!
//! # Frame-size cap (critique #5)
//!
//! The length prefix is attacker- / corruption-controlled, so a bad prefix must
//! never trigger an unbounded allocation. The moment 4 prefix bytes are buffered,
//! a length exceeding [`super::proto::MAX_FRAME_BYTES`] is a PROTOCOL ERROR: we
//! return `Err` (`InvalidData`) and NEVER reserve the buffer eagerly. The payload
//! `Vec` is only grown once the bytes have actually been received.

use std::io::{self, ErrorKind};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;

use super::proto::MAX_FRAME_BYTES;

/// Number of bytes in the big-endian u32 length prefix.
const PREFIX_LEN: usize = 4;

/// Write one length-prefixed frame to any async writer: a 4-byte big-endian u32
/// payload length followed by `bytes`.
///
/// Generic over the sink so the per-client connection task (daemon stage 5) can
/// write to an `OwnedWriteHalf` of a split [`UnixStream`] while the matching read
/// half is concurrently read by a sibling task — a single non-split stream can't
/// be `&mut`-borrowed for read and write at once. [`write_frame`] is the
/// `UnixStream` convenience used by the client/self-test.
///
/// Rejects a payload larger than [`MAX_FRAME_BYTES`] with `InvalidInput` BEFORE
/// writing anything, so the two sides agree on the same cap in both directions.
/// Issues two writes (prefix, then payload); a unix socket is a stream socket, so
/// the reader reassembles regardless of how these land on the wire.
pub async fn write_frame_to<W: AsyncWrite + Unpin>(sink: &mut W, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "frame payload {} exceeds MAX_FRAME_BYTES {}",
                bytes.len(),
                MAX_FRAME_BYTES
            ),
        ));
    }
    // Cast is safe: bounded by MAX_FRAME_BYTES (well under u32::MAX).
    let prefix = (bytes.len() as u32).to_be_bytes();
    sink.write_all(&prefix).await?;
    sink.write_all(bytes).await?;
    sink.flush().await?;
    Ok(())
}

/// Write one length-prefixed frame to `stream`. Thin [`UnixStream`] wrapper over
/// the generic [`write_frame_to`] — the codec is identical; this is the form the
/// client and self-test use on a non-split stream.
pub async fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> io::Result<()> {
    write_frame_to(stream, bytes).await
}

/// Reassembles whole frames from a byte stream that may deliver them split across
/// — or coalesced within — individual reads.
///
/// Owns a growable buffer fed via [`push`](Self::push); [`next_frame`](Self::next_frame)
/// drains exactly one complete frame at a time. Cheap to keep alive across many
/// reads on one connection.
#[derive(Debug, Default)]
pub struct FrameReader {
    /// Bytes received but not yet consumed into a complete frame. Leading bytes
    /// are always at the start of the next (possibly incomplete) frame.
    buf: Vec<u8>,
}

impl FrameReader {
    /// A fresh reader with an empty buffer.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed raw bytes just read from the socket into the reassembly buffer.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Try to pull one complete frame's payload out of the buffer.
    ///
    /// Returns:
    /// - `Ok(Some(payload))` and removes that frame's bytes from the buffer when a
    ///   full frame is available;
    /// - `Ok(None)` when more bytes are still needed (prefix incomplete, or the
    ///   payload has not fully arrived) — the buffer is left untouched;
    /// - `Err(InvalidData)` when the length prefix exceeds [`MAX_FRAME_BYTES`]
    ///   (critique #5: a protocol error, never an eager allocation).
    pub fn next_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.buf.len() < PREFIX_LEN {
            return Ok(None);
        }
        // Read the length prefix WITHOUT consuming it yet — we only commit once the
        // whole payload is present, so a partial payload leaves the buffer intact.
        let mut prefix = [0u8; PREFIX_LEN];
        prefix.copy_from_slice(&self.buf[..PREFIX_LEN]);
        let len = u32::from_be_bytes(prefix) as usize;

        // Cap check happens here, the instant the prefix is known and BEFORE any
        // payload-sized allocation. A hostile/garbage prefix can never OOM us.
        if len > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("frame length {len} exceeds MAX_FRAME_BYTES {MAX_FRAME_BYTES}"),
            ));
        }

        let total = PREFIX_LEN + len;
        if self.buf.len() < total {
            // Payload not fully arrived yet — wait for more bytes.
            return Ok(None);
        }

        // Whole frame is buffered: extract the payload and shift the remainder
        // (the start of the next frame) down to the front.
        let payload = self.buf[PREFIX_LEN..total].to_vec();
        self.buf.drain(..total);
        Ok(Some(payload))
    }
}

/// Read exactly one complete frame from any async reader, blocking until it fully
/// arrives.
///
/// Generic over the source so the per-client connection task (daemon stage 5) can
/// read from an `OwnedReadHalf` of a split [`UnixStream`] while a sibling task
/// writes the matching write half. [`read_frame`] is the `UnixStream` convenience
/// used by the client/self-test.
///
/// Pulls from `reader`'s already-buffered bytes first (a previous read may have
/// delivered more than one frame), then loops reading from the source and feeding
/// the reader until a whole frame pops out. Returns `Err(UnexpectedEof)` if the
/// peer closes mid-frame, and propagates the [`FrameReader::next_frame`] cap error.
pub async fn read_frame_from<R: AsyncRead + Unpin>(
    src: &mut R,
    reader: &mut FrameReader,
) -> io::Result<Vec<u8>> {
    loop {
        // A prior socket read may have buffered a full frame already; drain those
        // before touching the socket again.
        if let Some(frame) = reader.next_frame()? {
            return Ok(frame);
        }
        let mut chunk = [0u8; 8192];
        let n = src.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "peer closed connection mid-frame",
            ));
        }
        reader.push(&chunk[..n]);
    }
}

/// Read exactly one complete frame from `stream`. Thin [`UnixStream`] wrapper over
/// the generic [`read_frame_from`] — the reassembly logic is identical; this is
/// the form the client and self-test use on a non-split stream.
pub async fn read_frame(stream: &mut UnixStream, reader: &mut FrameReader) -> io::Result<Vec<u8>> {
    read_frame_from(stream, reader).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A length prefix over the cap is rejected without allocating the payload.
    #[test]
    fn oversized_prefix_is_protocol_error() {
        let mut reader = FrameReader::new();
        // Prefix claims MAX_FRAME_BYTES + 1 bytes; no payload supplied.
        let bogus = (MAX_FRAME_BYTES as u64 + 1) as u32;
        reader.push(&bogus.to_be_bytes());
        let err = reader.next_frame().expect_err("oversized prefix must error");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    /// A frame delivered in two halves reassembles into the original payload.
    #[test]
    fn split_frame_reassembles() {
        let payload = b"hello frame";
        let mut wire = Vec::new();
        wire.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        wire.extend_from_slice(payload);

        let mut reader = FrameReader::new();
        // First half: only part of the prefix+payload.
        reader.push(&wire[..3]);
        assert!(reader.next_frame().unwrap().is_none(), "partial → None");
        // Second half completes the frame.
        reader.push(&wire[3..]);
        let got = reader.next_frame().unwrap().expect("frame completes");
        assert_eq!(got, payload);
        // Buffer drained: no second frame.
        assert!(reader.next_frame().unwrap().is_none());
    }

    /// Two frames coalesced in one push are yielded one at a time, in order.
    #[test]
    fn coalesced_frames_yield_in_order() {
        let mut wire = Vec::new();
        for p in [b"one".as_slice(), b"twotwo".as_slice()] {
            wire.extend_from_slice(&(p.len() as u32).to_be_bytes());
            wire.extend_from_slice(p);
        }
        let mut reader = FrameReader::new();
        reader.push(&wire);
        assert_eq!(reader.next_frame().unwrap().unwrap(), b"one");
        assert_eq!(reader.next_frame().unwrap().unwrap(), b"twotwo");
        assert!(reader.next_frame().unwrap().is_none());
    }
}

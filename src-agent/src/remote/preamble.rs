//! Skip what the remote login shell prints before Koma starts.
//!
//! Remote commands run under `bash -ilc`, so `.profile` and `.bashrc` run first
//! and anything they echo (a welcome banner, conda or nvm notices, `fortune`)
//! lands on the same stdout as Koma's framed IPC. [`super::ssh::remote_command`]
//! prints [`MARKER`] right before it execs Koma; readers drop every byte up to
//! and including it.

use std::io::{self, BufRead};
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

/// Printed by the remote shell on its own line right before `exec koma`. The
/// first byte appears once, so a mismatch can restart matching at that byte.
pub(crate) const MARKER: &str = "KOMA-STREAM-BEGIN";
const MARKER_LINE: &[u8] = b"KOMA-STREAM-BEGIN\n";

/// More shell noise than this before the marker means the stream is not Koma.
const MAX_PREAMBLE: usize = 1024 * 1024;

/// Incremental matcher for [`MARKER`] followed by a newline.
#[derive(Default)]
struct Matcher {
    matched: usize,
}

impl Matcher {
    /// Feed bytes; returns the index just past the marker when it completes.
    fn feed(&mut self, data: &[u8]) -> Option<usize> {
        let marker = MARKER_LINE;
        for (index, &byte) in data.iter().enumerate() {
            if byte == marker[self.matched] {
                self.matched += 1;
            } else {
                self.matched = usize::from(byte == marker[0]);
            }
            if self.matched == marker.len() {
                return Some(index + 1);
            }
        }
        None
    }
}

/// Text after the marker, or the whole text when the marker is absent (commands
/// that do not go through `remote_command`).
pub(crate) fn strip(output: &str) -> &str {
    match output.rfind(MARKER) {
        Some(at) => output[at + MARKER.len()..].strip_prefix('\n').unwrap_or(&output[at + MARKER.len()..]),
        None => output,
    }
}

/// Blocking line reader version, for the line-based coding worker.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub(crate) fn skip_sync<R: BufRead>(reader: &mut R) -> io::Result<()> {
    let mut skipped = 0usize;
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "remote shell closed before Koma started",
            ));
        }
        if line.ends_with(MARKER_LINE) {
            return Ok(());
        }
        skipped += n;
        if skipped > MAX_PREAMBLE {
            return Err(too_much());
        }
    }
}

fn too_much() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the remote login shell printed over 1 MiB before Koma started",
    )
}

/// Async reader that drops everything up to the marker, then passes bytes through.
pub(crate) struct PreambleSkip<R> {
    inner: R,
    matcher: Matcher,
    done: bool,
    skipped: usize,
    pending: Vec<u8>,
    pos: usize,
}

impl<R> PreambleSkip<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            matcher: Matcher::default(),
            done: false,
            skipped: 0,
            pending: Vec::new(),
            pos: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for PreambleSkip<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.pos < this.pending.len() {
                let n = buf.remaining().min(this.pending.len() - this.pos);
                buf.put_slice(&this.pending[this.pos..this.pos + n]);
                this.pos += n;
                if this.pos == this.pending.len() {
                    this.pending = Vec::new();
                    this.pos = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if this.done {
                return Pin::new(&mut this.inner).poll_read(cx, buf);
            }
            let mut chunk = [0u8; 8192];
            let mut read = ReadBuf::new(&mut chunk);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut read))?;
            let data = read.filled();
            if data.is_empty() {
                return Poll::Ready(Ok(()));
            }
            match this.matcher.feed(data) {
                Some(end) => {
                    this.done = true;
                    this.pending = data[end..].to_vec();
                    this.pos = 0;
                }
                None => {
                    this.skipped += data.len();
                    if this.skipped > MAX_PREAMBLE {
                        return Poll::Ready(Err(too_much()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "preamble_test.rs"]
mod tests;

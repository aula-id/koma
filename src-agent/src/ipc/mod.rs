//! IPC layer: the wire protocol the koma-daemon and its thin TUI client speak.
//!
//! The end-state architecture is ALWAYS-CLIENT/SERVER: a headless `koma-daemon`
//! owns the agent runtime + session locks, and the TUI is a thin attach/detach
//! client over a unix socket (`~/.koma/daemon.sock`) using length-prefixed JSON
//! frames. This module is STAGE 1 of that split: it defines ONLY the message
//! vocabulary ([`proto`]) — the request/response/snapshot/delta types — with no
//! transport and no callers yet. The socket server, framing, and snapshot/delta
//! emission land in later stages.
//!
//! See [`proto`] for the protocol types and the critique fixes (stable session
//! UUIDs, monotonic seq, frame-size cap) that are designed into them from the
//! start to prevent silent stream corruption later.
//!
//! STAGE 2 adds the transport primitives — [`frame`] (the shared length-prefixed
//! codec), [`server`] (bind = liveness oracle), and [`client`] (connect + frame
//! helpers) — plus a [`selftest`] that round-trips a real frame end-to-end. The
//! daemon/client loop wiring that consumes them is still a later stage; the
//! transport is additive and does not touch the TUI path.

/// Platform IPC transport aliases. On unix these are the tokio unix-domain-socket
/// types; on windows the [`win`] named-pipe backend provides the same shapes.
#[cfg(unix)]
pub type IpcListener = tokio::net::UnixListener;
#[cfg(unix)]
pub type IpcStream = tokio::net::UnixStream;
/// Blocking (std) counterpart used by the sync management/probe clients.
#[cfg(unix)]
pub type SyncIpcStream = std::os::unix::net::UnixStream;

/// Owned read/write halves of a split [`IpcStream`]. On unix the tokio unix-socket
/// owned halves (`into_split`); on windows the `tokio::io::split` halves of the
/// named-pipe stream. Consumed by the per-client connection tasks (daemon + client
/// bridge + ext host) that read and write the same stream from independent tasks.
#[cfg(unix)]
pub type IpcReadHalf = tokio::net::unix::OwnedReadHalf;
#[cfg(unix)]
pub type IpcWriteHalf = tokio::net::unix::OwnedWriteHalf;

/// Split an [`IpcStream`] into independent owned read/write halves. On unix this is
/// exactly `UnixStream::into_split()`; on windows it is `tokio::io::split`. A single
/// cross-platform shim so the read/write-task code stays identical on both.
#[cfg(unix)]
pub fn split_stream(stream: IpcStream) -> (IpcReadHalf, IpcWriteHalf) {
    stream.into_split()
}

// Windows named-pipe backend: the same IpcListener/IpcStream/SyncIpcStream shapes over
// `tokio::net::windows::named_pipe`, DACL-hardened. See [`win`].
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{IpcListener, IpcStream, SyncIpcStream};
#[cfg(windows)]
pub type IpcReadHalf = tokio::io::ReadHalf<IpcStream>;
#[cfg(windows)]
pub type IpcWriteHalf = tokio::io::WriteHalf<IpcStream>;
#[cfg(windows)]
pub fn split_stream(stream: IpcStream) -> (IpcReadHalf, IpcWriteHalf) {
    tokio::io::split(stream)
}

pub mod client;
pub mod conn;
pub mod frame;
#[cfg(feature = "linker")]
pub mod linker_proto;
pub mod mcp_proto;
pub mod oauth_proto;
pub mod proto;
pub mod selftest;
pub mod server;
pub mod snapshot;

#[cfg(test)]
#[path = "mod_roundtrip_tests.rs"]
mod roundtrip_tests;

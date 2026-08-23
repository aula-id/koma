//! Private stdio protocol for the Coding-panel remote thin client (`koma remote-fs`).
//!
//! Separate from session-daemon `ClientRequest`/`DaemonFrame`. Reuses the shared
//! length-prefix framing in [`crate::ipc::frame`] and field shapes that match
//! [`super::client::file_ops`] result types / `PushEnvelope` File* bodies so the
//! host can forward replies without remapping.

use super::client::file_ops::{
    FileCreateResult, FileDeleteResult, FileDownloadBytesResult, FileReadResult, FileRenameResult,
    FileSaveResult, FileTreeResult, FileWriteBytesResult,
};

/// Request from the local host to a remote `koma remote-fs` process.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum RemoteFsReq {
    /// Optional handshake: report effective sandbox roots + version.
    Hello,
    /// Replace the sandbox root list (absolute remote paths).
    SetRoots { roots: Vec<String> },
    Tree {
        root: String,
        path: String,
        request_id: String,
    },
    Read {
        root: String,
        path: String,
        request_id: String,
    },
    Save {
        root: String,
        path: String,
        content: String,
        expected_fingerprint: String,
        request_id: String,
    },
    Create {
        root: String,
        path: String,
        kind: String,
        request_id: String,
    },
    Rename {
        root: String,
        old_path: String,
        new_path: String,
        request_id: String,
    },
    Delete {
        root: String,
        path: String,
        request_id: String,
    },
    WriteBytes {
        root: String,
        path: String,
        bytes_b64: String,
        overwrite: bool,
        request_id: String,
    },
    DownloadBytes {
        root: String,
        path: String,
        request_id: String,
    },
}

/// Reply from remote-fs. Bodies mirror PushEnvelope File* fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum RemoteFsRep {
    Hello {
        roots: Vec<String>,
        version: String,
    },
    SetRoots {
        roots: Vec<String>,
        error: Option<String>,
    },
    Tree(FileTreeResult),
    Read(FileReadResult),
    Save(FileSaveResult),
    Create(FileCreateResult),
    Rename(FileRenameResult),
    Delete(FileDeleteResult),
    WriteBytes(FileWriteBytesResult),
    DownloadBytes(FileDownloadBytesResult),
    /// Catch-all for protocol/parse errors on a request that had no op body.
    Error {
        error: String,
        request_id: Option<String>,
    },
}

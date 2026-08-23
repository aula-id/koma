//! Private stdio protocol for the Import-Graph remote thin client (`koma remote-linker`).
//!
//! Separate from session-daemon `ClientRequest`/`DaemonFrame`. Reuses the shared
//! length-prefix framing in [`crate::ipc::frame`] and result shapes that match
//! [`super::client::import_graph`] / `PushEnvelope` ImportGraph* bodies so the
//! host can forward replies without remapping.

use super::client::import_graph::ImportGraphResult;
use super::client::push_proto::ImportGraphImpactResult;

/// Request from the local host to a remote `koma remote-linker` process.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum RemoteLinkerReq {
    /// Optional handshake: report effective roots + version.
    Hello,
    /// Replace the workdir root list (absolute remote paths).
    SetRoots { roots: Vec<String> },
    Graph {
        path: Option<String>,
        depth: u32,
        direction: crate::ipc::linker_proto::GraphDirection,
        filter_roots: Option<Vec<String>>,
        filter_languages: Option<Vec<String>>,
        session_id: Option<String>,
        request_id: Option<String>,
    },
    Impact {
        path: String,
        depth: u32,
        request_id: String,
        session_id: Option<String>,
    },
    Reindex {
        session_id: Option<String>,
        request_id: Option<String>,
        filter_roots: Option<Vec<String>>,
        filter_languages: Option<Vec<String>>,
    },
}

/// Reply from remote-linker. Bodies mirror PushEnvelope ImportGraph* fields.
/// No `Debug` — nested ImportGraph* DTOs are Serialize-only (match GUI push contract).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum RemoteLinkerRep {
    Hello {
        roots: Vec<String>,
        version: String,
    },
    SetRoots {
        roots: Vec<String>,
        error: Option<String>,
    },
    Graph(ImportGraphResult),
    Impact(ImportGraphImpactResult),
    /// Catch-all for protocol/parse errors on a request that had no op body.
    Error {
        error: String,
        request_id: Option<String>,
    },
}

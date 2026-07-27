//! Linker daemon wire protocol — the request/response vocabulary the global
//! linker daemon and its session clients speak.
//!
//! Same 4-byte-BE-len + JSON frame codec as the MCP daemon.

use serde::{Deserialize, Serialize};

/// A request from a session to the global linker daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinkerRequest {
    /// Register one or more workspace roots for scanning.
    RegisterWorkspaces {
        roots: Vec<String>,  // canonical abs paths as strings
        session_id: String,
    },
    /// Unregister a session (all its roots released).
    Unregister {
        session_id: String,
    },
    /// Query the graph.
    Query(LinkerQuery),
    /// Get a summary for L1 injection.
    Summary,
    /// Report build fingerprint (same pattern as MCP).
    Fingerprint,
    /// Graceful shutdown.
    Shutdown,
}

/// A graph query action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinkerQuery {
    /// Outgoing imports of a file.
    Dependencies { path: String },
    /// Files that import a given file.
    Dependents { path: String },
    /// Transitive impact set (files affected by changing a file).
    Impact { path: String, depth: Option<u32> },
    /// 1-hop neighborhood (imports + importers).
    Neighborhood { path: String },
    /// Full project status (file count, edge count, languages, top fan-in).
    Status,
    /// Force a full rescan.
    Rescan,
}

/// The linker daemon's reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinkerResponse {
    /// Ready (scan complete for all registered roots).
    Ready,
    /// Registration result with scan status.
    Registered {
        status: ScanStatus,
        generation: u64,
    },
    /// Summary for L1 injection.
    Summary {
        text: String,
        generation: u64,
        file_count: usize,
        edge_count: usize,
        languages: Vec<String>,
    },
    /// A list of paths (dependencies, dependents, impact, neighborhood).
    PathList {
        paths: Vec<String>,
        /// Total count (may exceed the returned list if capped).
        total: usize,
    },
    /// Build fingerprint.
    Fingerprint(String),
    /// Acknowledgement (register, unregister, rescan, shutdown).
    Ack,
    /// Error.
    Error(String),
}

/// Status of the graph scan returned with registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScanStatus {
    /// Scan is still running; poll Summary for completion.
    Scanning,
    /// Scan complete; graph is ready.
    Ready,
}

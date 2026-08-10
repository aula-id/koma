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
        roots: Vec<String>, // canonical abs paths as strings
        session_id: String,
    },
    /// Unregister a session (all its roots released).
    Unregister { session_id: String },
    /// Query the graph.
    Query(LinkerQuery),
    /// Get a summary for L1 injection.
    Summary,
    /// Lightweight generation check — returns just the current graph generation
    /// number without computing the full summary text. O(1) on the daemon side.
    Generation,
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
    /// Structured bounded subgraph for GUI visualization.
    Visualization(VisualizationRequest),
}

/// Direction filter for the visualization query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphDirection {
    /// Show only outgoing imports (dependencies).
    #[serde(rename = "dependencies")]
    Dependencies,
    /// Show only incoming imports (dependents).
    #[serde(rename = "dependents")]
    Dependents,
    /// Show both directions.
    #[serde(rename = "both")]
    Both,
}

/// Parameters for a bounded visualization query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualizationRequest {
    /// Optional focal file path. If `None`, returns an overview (top fan-in + entry points).
    pub path: Option<String>,
    /// Traversal depth (1–3, clamped by daemon).
    pub depth: u32,
    /// Which directions to traverse from the focus.
    pub direction: GraphDirection,
    /// Maximum nodes in the result (daemon-enforced).
    pub max_nodes: usize,
    /// Maximum edges in the result (daemon-enforced).
    pub max_edges: usize,
}

/// The linker daemon's reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinkerResponse {
    /// Ready (scan complete for all registered roots).
    Ready,
    /// Registration result with scan status.
    Registered { status: ScanStatus, generation: u64 },
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
    /// Current graph generation (lightweight probe, no summary text).
    Generation(u64),
    /// Bounded subgraph view for GUI visualization.
    GraphView(GraphViewResult),
    /// Error.
    Error(String),
}

/// Role of a node relative to the focal file in a visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphNodeRole {
    /// The focal file itself.
    Focus,
    /// A file imported by the focus (directly or transitively).
    Dependency,
    /// A file that imports the focus (directly or transitively).
    Dependent,
    /// Overview-only node (top fan-in or entry point, no focus set).
    Overview,
}

/// A node in the bounded visualization graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphViewNode {
    /// Canonical absolute path (stable identifier).
    pub path: String,
    /// Language name (e.g. "Rust", "Python").
    pub language: String,
    /// Number of outgoing imports within this view.
    pub out_degree: usize,
    /// Number of incoming imports within this view.
    pub in_degree: usize,
    /// Role relative to the focal file.
    pub role: GraphNodeRole,
    /// BFS depth from the focal file (0 = focus, None = overview).
    pub depth_from_focus: Option<u32>,
}

/// A directed edge in the bounded visualization graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphViewEdge {
    /// Source node path (the importer).
    pub from: String,
    /// Target node path (the imported file).
    pub to: String,
}

/// The result of a bounded visualization query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphViewResult {
    /// Nodes in the view (capped by max_nodes).
    pub nodes: Vec<GraphViewNode>,
    /// Edges in the view (capped by max_edges).
    pub edges: Vec<GraphViewEdge>,
    /// Canonical path of the focal file (if any).
    pub focus: Option<String>,
    /// Current graph generation.
    pub generation: u64,
    /// Total files in the full graph.
    pub file_count: usize,
    /// Total edges in the full graph.
    pub edge_count: usize,
    /// Languages present in the full graph.
    pub languages: Vec<String>,
    /// Number of nodes omitted by the cap.
    pub nodes_truncated: bool,
    /// Number of edges omitted by the cap.
    pub edges_truncated: bool,
    /// Total available nodes matching the query (may exceed returned count).
    pub total_nodes_available: usize,
    /// Total available edges matching the query (may exceed returned count).
    pub total_edges_available: usize,
}

/// Status of the graph scan returned with registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScanStatus {
    /// Scan is still running; poll Summary for completion.
    Scanning,
    /// Scan complete; graph is ready.
    Ready,
}

use serde::{Deserialize, Serialize};

/// Pure-data projection of `Mode::Remote`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSnapshot {
    /// Sub-mode: "compact" | "fullscreen" | "connecting" | "password"
    pub sub: String,
    /// All saved hosts.
    pub hosts: Vec<RemoteHostSnapshot>,
    /// Selected host index in the list.
    pub selected: usize,
    /// Current search/filter query.
    pub query: String,
    /// Indices matching the current query.
    pub filtered: Vec<usize>,
    /// Host ID when viewing detail (fullscreen sub).
    pub detail_host: Option<String>,
    /// Connection stage: "resolving" | "authenticating" | "bootstrapping" | "connected" | None
    pub stage: Option<String>,
    /// Connection error message.
    pub error: Option<String>,
    /// Sessions on the selected remote host.
    pub sessions: Vec<RemoteSessionSnapshot>,
    /// Selected session index.
    pub session_selected: usize,
    /// Host ID pending delete confirmation.
    pub pending_delete: Option<String>,
}

/// Snapshot of a single remote host for the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteHostSnapshot {
    pub id: String,
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub key_path: Option<String>,
    pub connected: bool,
    pub last_connected: Option<u64>,
    pub tags: Vec<String>,
}

/// Snapshot of a session on a remote host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionSnapshot {
    pub session_id: String,
    pub name: String,
    pub working: bool,
    pub is_foreground: bool,
}

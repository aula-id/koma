use serde::{Deserialize, Serialize};

/// Pure-data projection of `Mode::Remote`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSnapshot {
    /// Sub-mode: "compact" | "fullscreen"
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
    /// Selected/prepared host identity.
    #[serde(default)]
    pub selected_host_id: Option<String>,
    /// Transient connection state serialized as a string.
    /// "disconnected" | "resolving" | "authenticating" | "auth_required:<host_id>:<user>:<host>"
    /// | "bootstrapping" | "connecting" | "connected:<session_id>" | "error:<message>" | None
    pub connection_state: Option<String>,
    /// Legacy: connection stage (kept for backward compat, mirrors connection_state).
    pub stage: Option<String>,
    /// Legacy: connection error (kept for backward compat, mirrors connection_state).
    pub error: Option<String>,
    /// Sessions on the selected remote host.
    pub sessions: Vec<RemoteSessionSnapshot>,
    /// Selected session index.
    pub session_selected: usize,
    /// Host editor draft state (present when view == "edit").
    #[serde(default)]
    pub editor: Option<RemoteEditorSnapshot>,
    /// Whether the editor is actively typing into a field.
    #[serde(default)]
    pub editing_field: bool,
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

/// Snapshot of the host editor form (create/edit) for the wire.
///
/// Carried inside [`RemoteSnapshot`] so thin clients see the editor draft
/// fields when `view == "edit"`. The daemon owns the source-of-truth
/// [`crate::app::mode::remote::HostEditor`]; this is a projection only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteEditorSnapshot {
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: String,
    pub key_path: String,
    /// Which field is focused, encoded as: "name" | "user" | "host" | "port" | "key_path".
    pub focused: String,
    /// `Some(id)` when editing an existing host, `None` when creating.
    #[serde(default)]
    pub edit_id: Option<String>,
    /// Validation error message, if any.
    #[serde(default)]
    pub error: Option<String>,
}

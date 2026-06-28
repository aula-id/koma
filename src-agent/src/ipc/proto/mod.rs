//! Wire protocol vocabulary for the koma daemon <-> client split.
//!
//! These are PURE-DATA, serde-round-trippable types — the only things that ever
//! cross the unix-socket boundary between the headless `koma-daemon` (which owns
//! the agent runtime + session locks) and a thin attach/detach TUI client.

pub mod key;
pub mod snapshot;
pub mod stream;

// Re-export everything so downstream `crate::ipc::proto::*` paths keep working.
#[allow(unused_imports)]
pub use key::{key_mods, KeyCodeWire, KeyWire};
pub use snapshot::*;
#[allow(unused_imports)]
pub use stream::StreamEventWire;

use serde::{Deserialize, Serialize};

// ─── frame constants ─────────────────────────────────────────────────────────

/// Hard upper bound on a single length-prefixed frame's payload size (64 MiB).
#[allow(dead_code)]
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

// ─── client -> daemon ────────────────────────────────────────────────────────

/// A request sent from a TUI client to the daemon over the unix socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum ClientRequest {
    Attach {
        foreground_id: Option<String>,
        cwd: Option<String>,
    },
    Detach,
    ListSessions,
    Resync,
    SwitchForeground { session_id: String },
    SubmitInput { text: String },
    Shell { cmd: String },
    SendKey(KeyWire),
    Paste { text: String },
    ApproveTool { approve: bool },
    NewSession {
        name: Option<String>,
        working_dir: Option<String>,
    },
    QuitSession { session_id: String },
    QuitDaemon,
    EditorWrapW(usize),
    OpenSessionHub,
}

// ─── daemon -> client ────────────────────────────────────────────────────────

/// The daemon -> client envelope. Carries a monotonic `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DaemonFrame {
    pub seq: u64,
    pub event: DaemonEvent,
}

/// What a [`DaemonFrame`] carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum DaemonEvent {
    /// Build-skew handshake (task #142): sent VERY FIRST on attach.
    Hello { version: String },
    /// A full state projection — sent on attach and on resync. Boxed.
    Snapshot(Box<StateSnapshot>),
    /// An incremental update folded onto the existing shadow.
    Delta(StateDelta),
    /// Acknowledgement of a request that produces no other reply.
    Ack,
    /// A request failed; the `String` is a human-readable reason.
    Error(String),
    /// One-shot: the controller asked for the `/select` transcript dump.
    EnterSelect,
}

// ─── mode discriminant ───────────────────────────────────────────────────────

/// A PURE-DATA projection of the live [`crate::app::mode::Mode`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum ModeSnapshot {
    KeyInput(KeyInputSnapshot),
    SessionPicker(PickerSnapshot),
    SessionHub(SessionHubSnapshot),
    Chat,
    Loading(LoadingSnapshot),
    Settings(Box<SettingsSnapshot>),
    Agents(Box<AgentsSnapshot>),
    Effort(EffortSnapshot),
    Usage(Box<UsageSnapshot>),
    MessageRewind(RewindSnapshot),
    QuitConfirm { working: usize, total: usize },
}

// ─── incremental deltas ──────────────────────────────────────────────────────

/// An incremental state update the daemon emits between full snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum StateDelta {
    TokenAppended { session_id: String, text: String },
    ReasoningAppended { session_id: String, text: String },
    StatusChanged {
        session_id: Option<String>,
        text: String,
    },
    InputChanged { text: String, cursor: usize },
    ScrollChanged { scroll: u16, follow: bool },
    SessionStatusChanged {
        session_id: String,
        working: bool,
        finished_unseen: bool,
    },
    ForegroundChanged { session_id: String },
    SessionAdded(Box<SessionSnapshot>),
    Toast { kind: String, text: String },
}

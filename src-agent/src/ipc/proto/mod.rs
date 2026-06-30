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

// ─── lightweight session metadata ────────────────────────────────────────────

/// A NON-attaching snapshot of one session-daemon's single owned session — the data
/// the hub/swapper collects when DISCOVERING live daemons (it probes each
/// `run/<id>.sock` for this, never opening a full attach/snapshot stream). Carries
/// only the few fields the picker needs: which session it is, its display name, its
/// working dir (to disambiguate two sessions of the same name in the hub), and whether
/// its agent is currently cooking.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionStatus {
    pub session_id: String,
    pub name: String,
    pub pwd: String,     // the session's working dir, for disambiguation in the hub
    pub working: bool,   // is the session's agent currently cooking?
}

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
    /// Lightweight metadata probe used by live-session DISCOVERY: ask the daemon for a
    /// one-shot [`DaemonEvent::Status`] describing its single owned session, with NO
    /// attach and NO snapshot stream. The daemon must answer this WITHOUT mutating any
    /// session state (no create/attach, no foreground change, no Hello/Snapshot).
    Status,
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
    /// One-shot: signal the foreground client to open its LOCAL daemon swapper
    /// (the `/resume` picker). Mirrors [`EnterSelect`] — a daemon-side `/resume`
    /// (or `OpenSessionHub`) emits this to the requesting client INSTEAD of building
    /// a daemon-side `Mode::SessionHub`, so the daemon never changes its own mode.
    /// Handled entirely client-side in `render_loop` (it detaches + runs the swapper
    /// standalone); the shadow treats it as a non-visual no-op.
    OpenSwapper,
    /// One-shot reply to a [`ClientRequest::Status`] discovery probe: this daemon's
    /// single owned session's metadata. Sent WITHOUT attaching the client or streaming
    /// any snapshot — the connection is expected to close right after.
    Status(SessionStatus),
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
    Mcp(Box<McpSnapshot>),
    Security(Box<SecuritySnapshot>),
    Bash(Box<BashSnapshot>),
    Help(Box<HelpSnapshot>),
    Effort(EffortSnapshot),
    Usage(Box<UsageSnapshot>),
    MessageRewind(RewindSnapshot),
    QuitConfirm { working: usize, total: usize, selected: usize },
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

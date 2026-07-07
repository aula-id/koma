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

/// One workspace-file hit from a [`ClientRequest::FileSearch`] (the GUI omnisearch
/// overlay). `label` is the display string exactly as the `@`-palette produces it (the
/// workspace-relative path, `[N]`-prefixed in multi-root mode, trailing `/` for a dir);
/// `path` is the ABSOLUTE on-disk path the daemon resolved from it, ready to hand back
/// as an [`ClientRequest::Paste`]/attach target (empty for a directory row, which is not
/// attachable).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileSearchItem {
    pub path: String,
    pub label: String,
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
    /// Drop a single STAGED attachment (a GUI chip) by its `[Image #N]` marker number,
    /// unstaging it from the foreground session's `pending_attachments` before submit.
    /// The TUI has no equivalent request (it drops attachments by deleting the marker in
    /// the composer); this is the GUI's explicit remove path.
    RemoveAttachment { marker_n: usize },
    /// Fuzzy-search the foreground session's workspace file index (the `@`-palette engine)
    /// for the GUI omnisearch overlay. Read-only: the daemon runs `DirCache::search` and
    /// replies with a one-shot [`DaemonEvent::FileSearchResults`] WITHOUT attaching or
    /// mutating state. `limit` caps the result count (defaults to the palette cap).
    FileSearch { query: String, limit: Option<usize> },
    ApproveTool { approve: bool },
    /// Answer a paused `plan_ready` approval. `decision` is one of `"approve"`,
    /// `"compact"` (approve + compact history to the plan), or `"deny"` (keep
    /// discussing). Parity with [`ApproveTool`] for a direct (non-key) path; the
    /// TUI client normally forwards the y/a/n keystroke as `SendKey` instead.
    PlanDecision { decision: String },
    NewSession {
        name: Option<String>,
        working_dir: Option<String>,
    },
    QuitSession { session_id: String },
    QuitDaemon,
    EditorWrapW(usize),
    OpenSessionHub,
    /// Rename the requesting client's FOREGROUND session (the GUI RenameOverlay
    /// submit). Sets the session's display name + persists via
    /// [`crate::model::store::rename_session`] (in-memory `name`/`settings.name` +
    /// the SQLite registry). The TUI has no equivalent request — it renames through
    /// the `/rename` slash-command / the Settings save — so this is the GUI's direct
    /// atomic rename path (fixes the "rename not working" gap where `NewSession.name`
    /// was accept-ignored). An empty/whitespace name is a no-op.
    RenameSession { name: String },

    // ─── GUI config setters (Connector + MCP panels) ─────────────────────────
    // All gui-gated: the TUI drives config through `Mode::Settings`/`Mode::Mcp` and
    // never sends these. Each mutates the daemon's authoritative `AppConfig` (or the
    // foreground session's local model overrides) + persists, reusing the SAME
    // config-layer setters/parsers the TUI editors use. The resulting config change
    // forces a full snapshot, so the GUI host re-derives + re-pushes its `Config`
    // envelope (see `ipc::snapshot::diff`).
    /// Upsert an MCP server. `uuid` is `None` for a brand-new server (the daemon mints
    /// one) or `Some` to edit an existing one by uuid. `args`/`env` are the panel's
    /// single-line STRING forms (space-separated args; `K=V, K2=V2` env); the daemon
    /// parses them into its array/pair forms. `transport` is `"stdio"` or `"http"`.
    SetMcpServer {
        uuid: Option<String>,
        name: String,
        enabled: bool,
        transport: String,
        command: String,
        args: String,
        env: String,
        url: String,
    },
    /// Remove the MCP server with `uuid`, persist, and live-reconnect the manager.
    DeleteMcpServer { uuid: String },
    /// Toggle the `enabled` flag on the MCP server with `uuid` (the list-row switch),
    /// persist, and live-reconnect the manager.
    EnableMcpServer { uuid: String, enabled: bool },
    /// Upsert a provider (Connector ProviderForm). `uuid` is `None` for a new provider
    /// (minted OpenAI-compatible) or `Some` to edit by uuid (preserving its wire type).
    SetProvider {
        uuid: Option<String>,
        name: String,
        endpoint: String,
        api_key: String,
    },
    /// Remove the provider with `uuid` and persist.
    DeleteProvider { uuid: String },
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
    /// One-shot: signal the foreground client to spawn + attach a BRAND-NEW
    /// session-daemon (the `/new` hand-off). Mirrors [`OpenSwapper`] — a daemon-side
    /// `/new` sets `new_pending` and the hub emits this to the controlling client INSTEAD
    /// of creating a session itself (a daemon owns exactly ONE session). `kill` is the
    /// `/new kill` flag: `true` tears the CURRENT session-daemon down (`QuitDaemon`) before
    /// attaching the new one; `false` leaves it cooking (resumable via the swapper). Handled
    /// entirely client-side in `render_loop`/`client_run` (it detaches — or kills, then
    /// detaches — and attaches a freshly minted id); the shadow treats it as a non-visual
    /// no-op.
    NewSession { kill: bool },
    /// One-shot reply to a [`ClientRequest::Status`] discovery probe: this daemon's
    /// single owned session's metadata. Sent WITHOUT attaching the client or streaming
    /// any snapshot — the connection is expected to close right after.
    Status(SessionStatus),
    /// One-shot reply to a [`ClientRequest::FileSearch`]: the resolved workspace-file
    /// hits for `query` (echoed so the GUI can drop a stale/out-of-order reply). Sent
    /// WITHOUT attaching or snapshotting — a metadata reply like [`Status`].
    FileSearchResults { query: String, items: Vec<FileSearchItem> },
}

// ─── mode discriminant ───────────────────────────────────────────────────────

/// A PURE-DATA projection of the live [`crate::app::mode::Mode`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum ModeSnapshot {
    Onboard(Box<OnboardSnapshot>),
    OnboardProvider(Box<OnboardProviderSnapshot>),
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
    Todo(Box<TodoSnapshot>),
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

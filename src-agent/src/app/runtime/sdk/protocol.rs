//! Public wire types. No private IPC structs are serialized on stdout.
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

pub const MAX_LINE: usize = 40 * 1024 * 1024;
/// Requests one adapter process accepts; the next one is fatal.
pub const REQUEST_IDS: usize = 65536;
/// Prompt text and each pasted attachment.
pub const TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const ATTACHMENT_COUNT: usize = 16;
pub const ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024;
/// How long a config write waits for the SDK advisory lock.
pub const CONFIG_LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(3000);

/// The first frame on stdout. `capabilities` stays the ten v1 names; the
/// other lists say what this binary implements beyond them.
pub fn hello(path: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "hello",
        "protocol": 1,
        "binary": {"path": path, "version": env!("CARGO_PKG_VERSION")},
        "diagnostics": [],
        "capabilities": super::methods::LEGACY_METHODS,
        "methods": super::methods::methods(),
        "features": super::methods::FEATURES,
        "events": super::methods::EVENTS,
        "limits": limits(),
    })
}

/// Only the bounds that apply to implemented methods.
pub fn limits() -> serde_json::Value {
    serde_json::json!({
        "frame_bytes": MAX_LINE,
        "request_ids": REQUEST_IDS,
        "text_bytes": TEXT_BYTES,
        "attachments": {
            "count": ATTACHMENT_COUNT,
            "pasted_bytes": TEXT_BYTES,
            "total_bytes": ATTACHMENT_BYTES,
        },
        "config": {"lock_wait_ms": CONFIG_LOCK_WAIT.as_millis()},
        "list": {
            "sessions": super::host::sessions::LIST_MAX,
            "usage_groups": crate::model::usage::query::MAX_GROUP_LIMIT,
            "usage_buckets": crate::model::usage::query::MAX_BUCKETS,
            "skills": super::host::skills::MAX_LIST,
            "rooms": super::room::LIST_ROOMS,
            "history_lines": super::room::ring::HISTORY_LINES,
            "shelf_rows": super::room::reads::SHELF_ROWS,
            "reports": super::room::reads::REPORTS,
            "trajectory_goals": super::room::reads::GOALS,
            "trajectory_nodes": super::room::reads::GOAL_NODES,
            "trajectory_edges": super::room::reads::GOAL_EDGES,
        },
        "room": super::room::limits(),
        "skills": super::host::skills::limits(),
        "time": {
            "ipc_ms": super::connection::BUDGET.as_millis(),
            "session_stop_ms": super::host::sessions::STOP_BUDGET.as_millis(),
            "usage_query_ms": crate::model::usage::query::DEADLINE.as_millis(),
            "room_start_ms": super::room::ROOM_START_MAX.as_millis(),
        },
    })
}
#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    Allow,
    Forbid,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    Default,
    Reject,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Policy {
    pub plan: Plan,
    pub approval: Approval,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            plan: Plan::Forbid,
            approval: Approval::Reject,
        }
    }
}
#[derive(Debug, Deserialize)]
pub struct Create {
    pub cwd: String,
    pub model_id: Option<String>,
    pub policy: Option<Policy>,
    #[serde(default)]
    pub external_approvals: bool,
    /// Validated before anything is spawned; applied once the session is
    /// attached.
    pub name: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize)]
pub struct Resume {
    pub session_id: String,
    pub policy: Option<Policy>,
    #[serde(default)]
    pub external_approvals: bool,
}
#[derive(Debug, Deserialize)]
pub struct Input {
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
    ImageFile { path: String },
    PastedText { text: String, name: Option<String> },
}
#[derive(Debug, Deserialize)]
pub struct Target {
    pub turn_id: String,
}
#[derive(Debug, Deserialize)]
pub struct Replacement {
    pub turn_id: String,
    #[serde(flatten)]
    pub input: Input,
}
#[derive(Debug, Deserialize)]
pub struct Decision {
    pub call_id: String,
    pub approve: bool,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Turn {
    pub id: String,
    pub state: &'static str,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PendingApproval {
    pub call_id: String,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Gate {
    pub kind: &'static str,
    pub reason: Option<String>,
    pub supported: bool,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct State {
    pub revision: u64,
    pub external_approvals: bool,
    pub pending_approval: Option<PendingApproval>,
    pub gate: Option<Gate>,
    #[serde(skip)]
    pub resync: u64,
    pub session_id: String,
    pub cwd: String,
    pub model_id: String,
    pub policy: Policy,
    pub turn: Option<Turn>,
    pub working: bool,
    pub awaiting_approval: bool,
    pub approval_reason: Option<String>,
    pub pending_count: usize,
    pub messages: Vec<Message>,
    pub streaming: Option<String>,
}
/// The reasons new errors carry. Each one fixes its error code and delivery,
/// so a reason can never be paired with another code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    WrongBinding,
    AlreadyBound,
    RoomClosed,
    OutOfRange,
    MissingParams,
    ConflictingParams,
    NotOffered,
    NotFound,
    Stale,
    Managed,
    MainChanged,
    OptionsNotReady,
    DaemonRefused,
    LockTimeout,
    ConfigUnreadable,
    SaveFailed,
    FrameLimit,
    IoError,
    Unverified,
    PartialApply,
    Binary,
    BoundSelf,
    ClerkNotAddressable,
    EpochChanged,
    InvalidName,
    LaneRunning,
    LayerRejected,
    Live,
    NoMainRoute,
    NoSuchLane,
    NoWorkLane,
    NotARoom,
    NotRunning,
    NoteRequired,
    ParentIsRoom,
    PartialDelete,
    ReadOnly,
    RejectedByRoom,
    ReservedId,
    RestoreFailed,
    RoleImmutable,
    RoomIdInUse,
    RoomStartFailed,
    RoomUnreadable,
    SessionLocked,
    SqliteBusy,
    StillRunning,
    TargetRequired,
    UnknownAvatar,
    UnknownModel,
    UnknownRole,
    UnsafeYaml,
    WorkdirRequired,
    AttachmentRefused,
    BadCursor,
    Exists,
    InvalidPackage,
    PeerUnsupported,
    QueryTimeout,
    RangeTooLarge,
    StreamError,
    TooLarge,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::WrongBinding => "wrong_binding",
            Reason::AlreadyBound => "already_bound",
            Reason::RoomClosed => "room_closed",
            Reason::OutOfRange => "out_of_range",
            Reason::MissingParams => "missing_params",
            Reason::ConflictingParams => "conflicting_params",
            Reason::NotOffered => "not_offered",
            Reason::NotFound => "not_found",
            Reason::Stale => "stale",
            Reason::Managed => "managed",
            Reason::MainChanged => "main_changed",
            Reason::OptionsNotReady => "options_not_ready",
            Reason::DaemonRefused => "daemon_refused",
            Reason::LockTimeout => "lock_timeout",
            Reason::ConfigUnreadable => "config_unreadable",
            Reason::SaveFailed => "save_failed",
            Reason::FrameLimit => "frame_limit",
            Reason::IoError => "io_error",
            Reason::Unverified => "unverified",
            Reason::PartialApply => "partial_apply",
            Reason::AttachmentRefused => "attachment_refused",
            Reason::BadCursor => "bad_cursor",
            Reason::Exists => "exists",
            Reason::InvalidPackage => "invalid_package",
            Reason::PeerUnsupported => "peer_unsupported",
            Reason::QueryTimeout => "query_timeout",
            Reason::RangeTooLarge => "range_too_large",
            Reason::StreamError => "stream_error",
            Reason::TooLarge => "too_large",
            Reason::Binary => "binary",
            Reason::BoundSelf => "bound_self",
            Reason::ClerkNotAddressable => "clerk_not_addressable",
            Reason::EpochChanged => "epoch_changed",
            Reason::InvalidName => "invalid_name",
            Reason::LaneRunning => "lane_running",
            Reason::LayerRejected => "layer_rejected",
            Reason::Live => "live",
            Reason::NoMainRoute => "no_main_route",
            Reason::NoSuchLane => "no_such_lane",
            Reason::NoWorkLane => "no_work_lane",
            Reason::NotARoom => "not_a_room",
            Reason::NotRunning => "not_running",
            Reason::NoteRequired => "note_required",
            Reason::ParentIsRoom => "parent_is_room",
            Reason::PartialDelete => "partial_delete",
            Reason::ReadOnly => "read_only",
            Reason::RejectedByRoom => "rejected_by_room",
            Reason::ReservedId => "reserved_id",
            Reason::RestoreFailed => "restore_failed",
            Reason::RoleImmutable => "role_immutable",
            Reason::RoomIdInUse => "room_id_in_use",
            Reason::RoomStartFailed => "room_start_failed",
            Reason::RoomUnreadable => "room_unreadable",
            Reason::SessionLocked => "session_locked",
            Reason::SqliteBusy => "sqlite_busy",
            Reason::StillRunning => "still_running",
            Reason::TargetRequired => "target_required",
            Reason::UnknownAvatar => "unknown_avatar",
            Reason::UnknownModel => "unknown_model",
            Reason::UnknownRole => "unknown_role",
            Reason::UnsafeYaml => "unsafe_yaml",
            Reason::WorkdirRequired => "workdir_required",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Reason::OutOfRange
            | Reason::MissingParams
            | Reason::ConflictingParams
            | Reason::NotOffered
            | Reason::AttachmentRefused
            | Reason::BadCursor
            | Reason::InvalidPackage
            | Reason::RangeTooLarge
            | Reason::TooLarge
            | Reason::Binary
            | Reason::ClerkNotAddressable
            | Reason::InvalidName
            | Reason::LayerRejected
            | Reason::NoSuchLane
            | Reason::NoWorkLane
            | Reason::NoteRequired
            | Reason::ParentIsRoom
            | Reason::ReservedId
            | Reason::RoleImmutable
            | Reason::TargetRequired
            | Reason::WorkdirRequired => "invalid_request",
            Reason::NotFound
            | Reason::UnknownAvatar
            | Reason::UnknownModel
            | Reason::UnknownRole => "not_found",
            Reason::WrongBinding
            | Reason::AlreadyBound
            | Reason::RoomClosed
            | Reason::Stale
            | Reason::Managed
            | Reason::MainChanged
            | Reason::OptionsNotReady
            | Reason::DaemonRefused
            | Reason::Exists
            | Reason::PeerUnsupported
            | Reason::BoundSelf
            | Reason::EpochChanged
            | Reason::LaneRunning
            | Reason::Live
            | Reason::NoMainRoute
            | Reason::NotARoom
            | Reason::NotRunning
            | Reason::ReadOnly
            | Reason::RejectedByRoom
            | Reason::RoomIdInUse
            | Reason::SessionLocked
            | Reason::UnsafeYaml => "invalid_state",
            Reason::LockTimeout
            | Reason::ConfigUnreadable
            | Reason::SaveFailed
            | Reason::FrameLimit
            | Reason::IoError
            | Reason::Unverified
            | Reason::PartialApply
            | Reason::QueryTimeout
            | Reason::StreamError
            | Reason::PartialDelete
            | Reason::RestoreFailed
            | Reason::RoomStartFailed
            | Reason::RoomUnreadable
            | Reason::SqliteBusy
            | Reason::StillRunning => "unavailable",
        }
    }

    /// Whether the outcome of an error with this reason is unknown.
    pub fn unknown(self) -> bool {
        matches!(
            self,
            Reason::Unverified
                | Reason::PartialApply
                | Reason::PartialDelete
                | Reason::RestoreFailed
                | Reason::RoomStartFailed
                | Reason::StillRunning
                | Reason::StreamError
        )
    }
}

/// The optional keys are serialized only when set, so errors that never set
/// them keep their v1 bytes.
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: &'static str,
    pub message: String,
    pub delivery: &'static str,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    extra: Option<Box<ErrorExtra>>,
    /// An `unknown` outcome of in-process host work, which leaves the bound
    /// coding connection intact and so does not end the process.
    #[serde(skip)]
    pub contained: bool,
}
/// Boxed so that `Result<_, RpcError>` stays small.
#[derive(Debug, Default, Serialize)]
pub struct ErrorExtra {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}
impl RpcError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            delivery: "not_sent",
            extra: None,
            contained: false,
        }
    }
    /// An unknown outcome on the bound coding connection; the process ends
    /// after the response.
    pub fn unknown(e: impl std::fmt::Display) -> Self {
        Self {
            delivery: "unknown",
            ..Self::new("unavailable", e.to_string())
        }
    }
    /// The optional keys; `None` when the error carries none of them.
    #[allow(dead_code)] // Read where a caller branches on `reason`.
    pub fn extra(&self) -> Option<&ErrorExtra> {
        self.extra.as_deref()
    }
    fn extra_mut(&mut self) -> &mut ErrorExtra {
        self.extra.get_or_insert_with(Box::default)
    }
    /// An error with a registered reason. The reason fixes the code, the
    /// delivery and, for an unverified write, `written`.
    pub fn reason(reason: Reason, message: impl Into<String>) -> Self {
        let mut error = Self::new(reason.code(), message);
        if reason.unknown() {
            error.delivery = "unknown";
        }
        let extra = error.extra_mut();
        extra.reason = Some(reason.name());
        if reason == Reason::Unverified {
            extra.written = Some(true);
        }
        error
    }
    /// The outcome concerns in-process host work, not the bound coding
    /// connection, so an `unknown` delivery does not end the process.
    pub fn in_process(mut self) -> Self {
        self.contained = true;
        self
    }
    pub fn with_binding(
        mut self,
        state: super::binding::BindingState,
        allowed: &[super::binding::BindingState],
    ) -> Self {
        let extra = self.extra_mut();
        extra.binding = Some(state.name());
        extra.allowed = Some(allowed.iter().map(|s| s.name()).collect());
        self
    }
    /// Bytes are known to be committed.
    pub fn written(mut self) -> Self {
        self.extra_mut().written = Some(true);
        self
    }
    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.extra_mut().detail = Some(detail);
        self
    }
    /// Whether answering with this error ends the process. Every `unknown`
    /// outcome of a v1 method does, as in v1.
    pub fn ends_process(&self, legacy: bool) -> bool {
        self.delivery == "unknown" && (legacy || !self.contained)
    }
}
pub fn params<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, RpcError> {
    if !v.is_object() {
        return Err(RpcError::new("invalid_request", "params must be an object"));
    }
    serde_json::from_value(v).map_err(|e| RpcError::new("invalid_request", e.to_string()))
}
/// [`params`] for a method added after v1: the same checks, and the error
/// carries a reason.
pub fn new_params<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, RpcError> {
    if !v.is_object() {
        return Err(RpcError::reason(
            Reason::OutOfRange,
            "params must be an object",
        ));
    }
    serde_json::from_value(v).map_err(|e| RpcError::reason(Reason::OutOfRange, e.to_string()))
}
/// Incremental bounded reader: never allocates an unbounded read_until buffer.
pub async fn line<R: AsyncBufRead + Unpin>(r: &mut R) -> anyhow::Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    loop {
        let chunk = r.fill_buf().await?;
        if chunk.is_empty() {
            anyhow::ensure!(out.is_empty(), "unterminated JSONL frame");
            return Ok(None);
        }
        let n = chunk.iter().position(|b| *b == b'\n');
        let take = n.unwrap_or(chunk.len());
        anyhow::ensure!(out.len() + take <= MAX_LINE, "JSONL frame exceeds 40 MiB");
        out.extend_from_slice(&chunk[..take]);
        r.consume(take + usize::from(n.is_some()));
        if n.is_some() {
            if out.last() == Some(&b'\r') {
                out.pop();
            }
            anyhow::ensure!(!out.is_empty(), "blank JSONL frame");
            return Ok(Some(out));
        }
    }
}
pub fn attachments(
    input: Input,
) -> Result<(String, Vec<crate::ipc::proto::TurnAttachmentInput>), RpcError> {
    use crate::ipc::proto::TurnAttachmentInput as A;
    if input.text.trim().is_empty()
        || input.text.len() > 2 * 1024 * 1024
        || input.attachments.len() > 16
    {
        return Err(RpcError::new(
            "invalid_request",
            "invalid prompt or attachment count",
        ));
    }
    let mut total = 0u64;
    let mut out = Vec::new();
    for a in input.attachments {
        out.push(match a {
            Attachment::PastedText { text, name } => {
                if text.len() > 2 * 1024 * 1024 {
                    return Err(RpcError::new(
                        "invalid_request",
                        "pasted text exceeds 2 MiB",
                    ));
                }
                total += text.len() as u64;
                A::PastedText { text, name }
            }
            Attachment::ImageFile { path } => {
                let p = std::path::Path::new(&path);
                if !p.is_absolute() {
                    return Err(RpcError::new(
                        "invalid_request",
                        "image path must be absolute",
                    ));
                }
                let m = std::fs::metadata(p).map_err(|_| {
                    RpcError::new("invalid_request", "image is not a readable regular file")
                })?;
                if !m.is_file() {
                    return Err(RpcError::new(
                        "invalid_request",
                        "image is not a regular file",
                    ));
                }
                total += m.len();
                A::ImageFile { path }
            }
        });
        if total > 32 * 1024 * 1024 {
            return Err(RpcError::new(
                "invalid_request",
                "attachments exceed 32 MiB",
            ));
        }
    }
    Ok((input.text, out))
}

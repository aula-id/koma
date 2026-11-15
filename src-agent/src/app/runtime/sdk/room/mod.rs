//! The ROOM binding: one adapter attached to a secure room daemon.
//!
//! `room.open` and `room.attach` take over the room's single live stream. A
//! reader task then owns the socket's read half for the whole binding and feeds
//! the line ring, the cached status and the event queue. When the stream ends,
//! or a socket write ends with an unknown outcome, the binding moves to
//! ROOM(closed): the ring, the epoch, the cached status and the room identity
//! stay, disk reads keep working and socket writes are refused. The process
//! never ends because of the room socket.
//!
//! Room reads and team methods build on [`RoomBinding`]: the room's session dir
//! and ids, the read-only shelf handle, the adapter's registry view, the roster
//! and its refresh, the `sdk_reload` request, the cached status and the scrubber.
pub mod connection;
pub mod reader;
pub mod reads;
pub mod ring;
pub mod scrub;
pub mod send;
pub mod spend;
pub mod team;

use self::connection::{Hello, OpenError, RoomSocket, MAX_FEATURES};
use self::reader::{Core, Pending, RoomStatus, Shared, StatusCounts, Want};
use self::ring::{Filter, RoomLine, HISTORY_LINES, TAIL_LINES};
use super::binding::{Binding, BindingState};
use super::connection::BUDGET;
use super::methods::HandlerFuture;
use super::protocol::{Reason, Request, RpcError};
use super::Adapter;
use crate::app::runtime::manage::RoomStartError;
use crate::model::session_registry::RegRowFullCreated;
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use crate::room::wire::RoomClient;
use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// `room.list` pages hold at most this many rooms.
pub const LIST_ROOMS: usize = 500;
const LIST_DEFAULT: usize = 100;
const TAIL_DEFAULT: usize = 50;
/// `room.open` waits at most this long for a started room to accept.
pub const ROOM_START_MAX: Duration = Duration::from_millis(30_000);
const ROOM_START_DEFAULT: Duration = Duration::from_millis(12_000);
const ATTACH_WAIT_MAX: Duration = Duration::from_millis(15_000);
const ATTACH_WAIT_DEFAULT: Duration = Duration::from_millis(2000);
/// `room.close` waits this long for the room's `closed`.
const QUIT_WAIT: Duration = Duration::from_secs(2);
/// A failed start reports at most this much of the start log.
const LOG_TAIL_BYTES: usize = 2048;
const MAX_ROSTER: usize = 64;
const MAX_ID_CHARS: usize = 64;

type SockPathFn = dyn Fn(&str) -> anyhow::Result<PathBuf> + Send + Sync;
type SessionsFn = dyn Fn() -> anyhow::Result<Vec<RegRowFullCreated>> + Send + Sync;
type SessionDirFn = dyn Fn(&str, &str) -> anyhow::Result<PathBuf> + Send + Sync;
type EnsureFn = dyn Fn(&str, &str, &Path, Duration) -> Result<bool, RoomStartError> + Send + Sync;
type StopFn = dyn Fn(&str) -> bool + Send + Sync;
type TemplateFn = dyn Fn() -> Option<PathBuf> + Send + Sync;
type UsageFn = dyn Fn(
        &crate::model::usage::query::UsageQuery,
    ) -> Result<
        crate::model::usage::query::UsageReport,
        crate::model::usage::query::UsageError,
    > + Send
    + Sync;

/// Where rooms are found and how they are started and stopped. The adapter
/// uses koma's data dir; tests point every hook at scratch paths and fakes.
#[derive(Clone)]
pub struct RoomEnv {
    pub sock_path: Arc<SockPathFn>,
    pub sessions: Arc<SessionsFn>,
    pub session_dir: Arc<SessionDirFn>,
    pub ensure_running: Arc<EnsureFn>,
    pub start_log: Arc<SockPathFn>,
    pub stop: Arc<StopFn>,
    pub template_dir: Arc<TemplateFn>,
    /// The usage ledger query behind `room.spend`.
    pub usage: Arc<UsageFn>,
}

impl RoomEnv {
    pub fn home() -> Self {
        use crate::app::runtime::manage;
        Self {
            sock_path: Arc::new(crate::model::store::daemon_sock_path),
            sessions: Arc::new(crate::model::session_registry::list_all_full),
            session_dir: Arc::new(crate::model::store::session_dir),
            ensure_running: Arc::new(manage::ensure_room_daemon_running),
            start_log: Arc::new(manage::room_start_log_path),
            stop: Arc::new(manage::kill_session_daemon),
            template_dir: Arc::new(crate::room::registry::template_dir),
            usage: Arc::new(crate::model::usage::query::run),
        }
    }

    fn alive(&self, id: &str) -> bool {
        (self.sock_path)(id).is_ok_and(|path| crate::ipc::SyncIpcStream::connect(&path).is_ok())
    }
}

/// One room as the SDK lists it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoomInfo {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub parent_name: Option<String>,
    pub name: Option<String>,
    pub workdir: Option<String>,
    pub live: bool,
    pub locked: bool,
    pub bound: bool,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DaemonFacts {
    pub version: Option<String>,
    pub features: Vec<String>,
    pub template_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HistoryBounds {
    pub first_line_seq: Option<u64>,
    pub last_line_seq: Option<u64>,
    pub count: usize,
    pub replayed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AttachResult {
    pub room: RoomInfo,
    pub started: bool,
    pub daemon: DaemonFacts,
    pub epoch: String,
    pub history: HistoryBounds,
    pub tail: Vec<RoomLine>,
    pub status: Option<RoomStatus>,
    pub roster: Vec<String>,
    pub roster_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct HistoryResult {
    epoch: String,
    first_line_seq: Option<u64>,
    last_line_seq: Option<u64>,
    lines: Vec<RoomLine>,
    has_more_before: bool,
    has_more_after: bool,
}

/// What an `sdk_reload` answered.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // Read by the room team methods.
pub struct Reloaded {
    pub ok: bool,
    pub version: Option<u64>,
    pub warnings: Vec<String>,
    pub code: Option<String>,
    pub error: Option<String>,
}

/// A bound room. Dropping it stops the reader and closes the socket.
pub struct RoomBinding {
    core: Arc<Core>,
    socket: Option<RoomSocket>,
    reader: tokio::task::JoinHandle<()>,
    info: RoomInfo,
    #[allow(dead_code)] // Read by the room reads and team methods.
    session_dir: PathBuf,
    hello: Hello,
    roster: Vec<String>,
    roster_truncated: bool,
    env: RoomEnv,
}

impl Drop for RoomBinding {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl RoomBinding {
    pub fn state(&self) -> BindingState {
        if self.core.live() {
            BindingState::RoomLive
        } else {
            BindingState::RoomClosed
        }
    }

    pub fn is_live(&self) -> bool {
        self.core.live()
    }

    pub fn session_id(&self) -> &str {
        &self.core.session_id
    }

    pub fn parent_session_id(&self) -> Option<&str> {
        self.info.parent_session_id.as_deref()
    }

    /// The room's session directory: `room.sqlite`, the `room/` overlay, the
    /// lane transcripts.
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn epoch(&self) -> &str {
        &self.core.epoch
    }

    /// The room as it was listed when it was bound.
    pub fn info(&self) -> &RoomInfo {
        &self.info
    }

    /// The read-only shelf handle opened at bind time. It survives into
    /// ROOM(closed). Use it on the blocking pool.
    pub fn shelf(&self) -> Result<Arc<Shelf>, RpcError> {
        self.core.shelf.clone().ok_or_else(|| {
            RpcError::reason(Reason::RoomUnreadable, "the room's shelf cannot be read")
        })
    }

    /// The adapter's own view of the room's registry layers (base, template,
    /// session overlay). `reload()` it after writing a layer.
    pub fn registry(&self) -> Option<Arc<RegistryHandle>> {
        self.core.registry.clone()
    }

    pub fn has_feature(&self, name: &str) -> bool {
        self.hello.features.iter().any(|f| f == name)
    }

    pub fn daemon_version(&self) -> Option<&str> {
        Some(self.hello.version.as_str()).filter(|v| !v.is_empty())
    }

    pub fn template_revision(&self) -> Option<&str> {
        self.hello.template_revision.as_deref()
    }

    /// The distinct avatars of the room's lanes, from its latest `models`
    /// answer.
    pub fn roster(&self) -> &[String] {
        &self.roster
    }

    /// The latest status the reader cached, or `None` before the first one.
    pub fn cached_status(&self) -> Option<RoomStatus> {
        self.core.cached_status()
    }

    /// Queue a `room.notice` (for example `layer_warnings` after a reload).
    pub fn notice(&self, code: &'static str, text: String) {
        let text = scrub::lock(&self.core.scrub).text(&text);
        self.core.notice(code, text);
    }

    /// Scrub every string of a room result. Refreshes the secret set first.
    pub async fn scrub(&self, value: Value) -> Value {
        scrub::scrub_value(&self.core.scrub, &self.core.store, value).await
    }

    /// Write one frame. A refusal before anything was written is E4; a
    /// failed write moves the binding to ROOM(closed) and is an unknown
    /// outcome that does not end the process.
    pub async fn write(&mut self, message: &RoomClient) -> Result<(), RpcError> {
        if !self.core.live() {
            return Err(room_closed());
        }
        let Some(socket) = self.socket.as_mut() else {
            return Err(room_closed());
        };
        if socket.send(message).await.is_err() {
            self.lose_stream();
            return Err(stream_error("the room connection failed during the write"));
        }
        Ok(())
    }

    /// Send `message` and wait up to `wait` for the direct answer `want`. A
    /// lost answer moves the binding to ROOM(closed).
    pub async fn request(
        &mut self,
        want: Want,
        message: &RoomClient,
        wait: Duration,
    ) -> Result<Value, RpcError> {
        self.request_with_echo(want, None, message, wait).await
    }

    /// [`RoomBinding::request`] for a send: an accepting answer puts `echo`
    /// in the ring at its place in the stream (see [`Pending::echo`]).
    pub async fn request_with_echo(
        &mut self,
        want: Want,
        echo: Option<String>,
        message: &RoomClient,
        wait: Duration,
    ) -> Result<Value, RpcError> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        {
            let mut state = self.core.lock();
            if !state.live {
                return Err(room_closed());
            }
            state.pending = Some(Pending { want, reply, echo });
        }
        self.write(message).await?;
        match tokio::time::timeout(wait, answer).await {
            Ok(Ok(value)) => Ok(value),
            _ => {
                self.core.lock().pending = None;
                self.lose_stream();
                Err(stream_error("the room did not answer"))
            }
        }
    }

    fn lose_stream(&mut self) {
        self.core.fail_stream();
        self.socket = None;
        self.reader.abort();
    }

    /// The room's `models` answer (`choices` and `lanes`), scrubbed.
    pub async fn models(&mut self) -> Result<Value, RpcError> {
        let answer = self
            .request(Want::Models, &RoomClient::Models, BUDGET)
            .await?;
        let models = answer.get("models").cloned().unwrap_or(Value::Null);
        Ok(self.scrub(models).await)
    }

    /// Ask the room for its lanes again and keep the distinct avatars as the
    /// roster.
    pub async fn refresh_roster(&mut self) -> Result<&[String], RpcError> {
        let models = self.models().await?;
        self.set_roster(&models);
        Ok(&self.roster)
    }

    /// Keep the distinct avatars of a `models` answer's lanes as the roster.
    pub fn set_roster(&mut self, models: &Value) {
        let mut roster: Vec<String> = Vec::new();
        for lane in models
            .get("lanes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(avatar) = lane.get("avatar").and_then(Value::as_str) {
                if !roster.iter().any(|a| a == avatar) {
                    roster.push(avatar.to_string());
                }
            }
        }
        self.roster_truncated = roster.len() > MAX_ROSTER;
        roster.truncate(MAX_ROSTER);
        self.roster = roster;
    }

    /// Ask the room to re-read its registry files (`sdk_reload`). Without
    /// the room's `sdk_reload` feature this is `peer_unsupported` and nothing
    /// is sent.
    pub async fn reload(&mut self) -> Result<Reloaded, RpcError> {
        if !self.has_feature("sdk_reload") {
            return Err(peer_unsupported("this room cannot reload its registry"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let message = RoomClient::SdkReload { id: id.clone() };
        let ack = self.request(Want::Ack(id), &message, BUDGET).await?;
        let ack = self.scrub(ack).await;
        let strings = |key: &str| -> Vec<String> {
            ack.get(key)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(Reloaded {
            ok: ack.get("ok").and_then(Value::as_bool).unwrap_or(false),
            version: ack.get("version").and_then(Value::as_u64),
            warnings: strings("warnings"),
            code: ack.get("code").and_then(Value::as_str).map(str::to_string),
            error: ack.get("error").and_then(Value::as_str).map(str::to_string),
        })
    }

    /// Detach from a live room: send `quit` and wait briefly for `closed`.
    /// The end of the stream announces nothing.
    pub async fn quit(&mut self) {
        self.core.lock().closing = true;
        if self.core.live() {
            let _ = self
                .request(Want::Closed, &RoomClient::Quit, QUIT_WAIT)
                .await;
        }
        self.core.queue.deactivate();
        self.socket = None;
        self.reader.abort();
    }

    fn history_bounds(state: &Shared) -> HistoryBounds {
        HistoryBounds {
            first_line_seq: state.ring.first_line_seq(),
            last_line_seq: state.ring.last_line_seq(),
            count: state.ring.len(),
            replayed: state.replayed_lines,
        }
    }
}

// ── errors ──────────────────────────────────────────────────────────────

fn out_of_range(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::OutOfRange, message)
}

fn not_found(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::NotFound, message)
}

fn room_closed() -> RpcError {
    RpcError::reason(Reason::RoomClosed, "the room connection is closed")
        .with_binding(BindingState::RoomClosed, &[BindingState::RoomLive])
}

pub(crate) fn peer_unsupported(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::PeerUnsupported, message)
}

/// An unknown outcome on the room socket. It never ends the process.
fn stream_error(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::StreamError, message).in_process()
}

fn io_error(e: impl std::fmt::Display) -> RpcError {
    RpcError::reason(Reason::IoError, e.to_string())
}

fn bound_room(adapter: &Adapter) -> Result<&RoomBinding, RpcError> {
    match &adapter.binding {
        Binding::Room(room) => Ok(room),
        _ => Err(RpcError::reason(Reason::WrongBinding, "no room bound")),
    }
}

fn bound_room_mut(adapter: &mut Adapter) -> Result<&mut RoomBinding, RpcError> {
    match &mut adapter.binding {
        Binding::Room(room) => Ok(room),
        _ => Err(RpcError::reason(Reason::WrongBinding, "no room bound")),
    }
}

// ── params ──────────────────────────────────────────────────────────────

pub(crate) fn object(params: Value) -> Result<Map<String, Value>, RpcError> {
    match params {
        Value::Object(map) => Ok(map),
        _ => Err(RpcError::reason(Reason::OutOfRange, "params must be an object")),
    }
}

fn id_param(map: &Map<String, Value>, key: &str) -> Result<String, RpcError> {
    match map.get(key) {
        Some(Value::String(id)) if !id.is_empty() && id.chars().count() <= MAX_ID_CHARS => {
            Ok(id.clone())
        }
        Some(_) => Err(out_of_range(format!(
            "{key} must be a string of 1 to {MAX_ID_CHARS} characters"
        ))),
        None => Err(out_of_range(format!("{key} is required"))),
    }
}

fn uint_param(
    map: &Map<String, Value>,
    key: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(value) => match value.as_u64() {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(out_of_range(format!("{key} must be {min} to {max}"))),
        },
    }
}

fn bool_param(map: &Map<String, Value>, key: &str) -> Result<Option<bool>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(out_of_range(format!("{key} must be a boolean"))),
    }
}

fn wait_param(
    map: &Map<String, Value>,
    max: Duration,
    default: Duration,
) -> Result<Duration, RpcError> {
    let max_ms = u64::try_from(max.as_millis()).unwrap_or(u64::MAX);
    Ok(uint_param(map, "wait_ms", 0, max_ms)?.map_or(default, Duration::from_millis))
}

fn tail_param(map: &Map<String, Value>) -> Result<usize, RpcError> {
    Ok(uint_param(map, "tail", 0, TAIL_LINES as u64)?.map_or(TAIL_DEFAULT, |n| n as usize))
}

// ── listing ─────────────────────────────────────────────────────────────

fn is_room_dir(dir: &Path) -> bool {
    crate::room::room_db_path(dir).is_file()
}

/// Every room in the registry, most recently updated first, with its parent
/// found by deriving each coding session's room id.
fn room_rows(
    env: &RoomEnv,
    bound: Option<&str>,
    only: Option<&str>,
) -> Result<Vec<RoomInfo>, RpcError> {
    let rows = (env.sessions)().map_err(io_error)?;
    let mut parents = std::collections::HashMap::new();
    let mut rooms = Vec::new();
    for row in &rows {
        let dir = (env.session_dir)(&row.pwd_hash, &row.uuid).ok();
        if dir.as_deref().is_some_and(is_room_dir) {
            if only.is_none_or(|id| id == row.uuid) {
                rooms.push((row, dir));
            }
        } else {
            parents.insert(crate::room::room_session_id(&row.uuid), row);
        }
    }
    Ok(rooms
        .into_iter()
        .map(|(row, dir)| {
            let parent = parents.get(&row.uuid);
            RoomInfo {
                session_id: row.uuid.clone(),
                parent_session_id: parent.map(|p| p.uuid.clone()),
                parent_name: parent.map(|p| p.name.clone()).filter(|n| !n.is_empty()),
                name: Some(row.name.clone()).filter(|n| !n.is_empty()),
                workdir: Some(row.workdir.clone()).filter(|w| !w.is_empty()),
                live: env.alive(&row.uuid),
                locked: dir
                    .as_deref()
                    .is_some_and(crate::model::session_lock::owner_alive),
                bound: bound == Some(row.uuid.as_str()),
                created_at: Some(row.created_at),
                updated_at: Some(row.updated_at),
            }
        })
        .collect())
}

fn encode_cursor(updated_at: i64, id: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{updated_at}:{id}"))
}

fn decode_cursor(cursor: &str) -> Option<(i64, String)> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let (key, id) = text.split_once(':')?;
    Some((key.parse().ok()?, id.to_string()))
}

fn bad_cursor() -> RpcError {
    RpcError::reason(Reason::BadCursor, "the cursor does not decode")
}

pub fn handle_list(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(list(r.params, adapter))
}

async fn list(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let parent = match map.get("parent_session_id") {
        None => None,
        Some(_) => Some(id_param(&map, "parent_session_id")?),
    };
    let live = bool_param(&map, "live")?;
    let limit =
        uint_param(&map, "limit", 1, LIST_ROOMS as u64)?.map_or(LIST_DEFAULT, |n| n as usize);
    let after = match map.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) => Some(decode_cursor(cursor).ok_or_else(bad_cursor)?),
        Some(_) => return Err(bad_cursor()),
    };
    let env = adapter.room_env.clone();
    let bound = match &adapter.binding {
        Binding::Room(room) => Some(room.session_id().to_string()),
        _ => None,
    };
    let rows = adapter
        .blocking(move || room_rows(&env, bound.as_deref(), None))
        .await?;
    let mut rooms: Vec<RoomInfo> = rows
        .into_iter()
        .filter(|room| parent.is_none() || room.parent_session_id == parent)
        .filter(|room| live.is_none_or(|live| room.live == live))
        .filter(|room| {
            after.as_ref().is_none_or(|(at, id)| {
                let updated = room.updated_at.unwrap_or(0);
                updated < *at || (updated == *at && room.session_id.as_str() > id.as_str())
            })
        })
        .collect();
    let next_cursor = (rooms.len() > limit).then(|| {
        let last = &rooms[limit - 1];
        encode_cursor(last.updated_at.unwrap_or(0), &last.session_id)
    });
    rooms.truncate(limit);
    let value = json!({"rooms": rooms, "next_cursor": next_cursor});
    Ok(scrub::scrub_value(&adapter.scrub, &adapter.config, value).await)
}

// ── binding ─────────────────────────────────────────────────────────────

pub fn handle_open(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(open(r.params, adapter))
}

pub fn handle_attach(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(attach(r.params, adapter))
}

/// The room id of `parent`, after the checks that make opening it safe.
struct Openable {
    room_id: String,
    workdir: PathBuf,
    live: bool,
}

fn check_openable(
    env: &RoomEnv,
    store: &super::host::config::ConfigStore,
    parent: &str,
) -> Result<Openable, RpcError> {
    let rows = (env.sessions)().map_err(io_error)?;
    let row = rows
        .iter()
        .find(|row| row.uuid == parent)
        .ok_or_else(|| not_found("unknown session"))?;
    let parent_dir = (env.session_dir)(&row.pwd_hash, &row.uuid).map_err(io_error)?;
    if is_room_dir(&parent_dir) {
        return Err(
            RpcError::reason(Reason::ParentIsRoom, "the parent session is itself a room"),
        );
    }
    let room_id = crate::room::room_session_id(parent);
    if let Some(room) = rows.iter().find(|row| row.uuid == room_id) {
        let dir = (env.session_dir)(&room.pwd_hash, &room.uuid).map_err(io_error)?;
        if !is_room_dir(&dir) && dir.join("messages.json").is_file() {
            return Err(RpcError::reason(Reason::RoomIdInUse, "the room's session id holds a coding session"));
        }
    }
    let live = env.alive(&room_id);
    if !live {
        let config = store.read()?.config;
        let settings = crate::model::session::Session::load(&parent_dir)
            .map(|s| s.settings)
            .unwrap_or_default();
        let usable = crate::app::resolve::resolve_role(
            &config,
            &settings,
            crate::model::app_config::ModelRole::Main,
        )
        .is_some_and(|route| route.is_usable());
        if !usable {
            return Err(RpcError::reason(Reason::NoMainRoute, "the parent session has no usable Main route"));
        }
    }
    Ok(Openable {
        room_id,
        workdir: PathBuf::from(&row.workdir),
        live,
    })
}

/// The last [`LOG_TAIL_BYTES`] of the start log, on a char boundary.
fn log_tail(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut start = text.len().saturating_sub(LOG_TAIL_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

async fn open(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let parent = id_param(&map, "parent_session_id")?;
    let tail = tail_param(&map)?;
    let wait = wait_param(&map, ROOM_START_MAX, ROOM_START_DEFAULT)?;
    let env = adapter.room_env.clone();
    let store = adapter.config.clone();
    let checked = {
        let parent = parent.clone();
        let env = env.clone();
        adapter
            .blocking(move || check_openable(&env, &store, &parent))
            .await?
    };
    let mut started = false;
    if !checked.live {
        let room_id = checked.room_id.clone();
        let env = env.clone();
        let workdir = checked.workdir.clone();
        let spawned = adapter
            .blocking(move || {
                Ok(
                    (env.ensure_running)(&room_id, &parent, &workdir, wait).map_err(|e| {
                        let tail = (env.start_log)(&room_id)
                            .map(|path| log_tail(&path))
                            .unwrap_or_default();
                        let spawned = matches!(e, RoomStartError::NotAccepting { .. });
                        (spawned, e.to_string(), tail)
                    }),
                )
            })
            .await?;
        match spawned {
            Ok(fresh) => started = fresh,
            Err((spawned, message, tail)) => {
                let message = scrub::scrub_text(&adapter.scrub, &adapter.config, &message).await;
                let tail = scrub::scrub_text(&adapter.scrub, &adapter.config, &tail).await;
                let detail = json!({"log_tail": tail});
                // A daemon that was spawned may still come up; one that was
                // never spawned changed nothing.
                return Err(if spawned {
                    RpcError::reason(Reason::RoomStartFailed, format!("the room did not start: {message}")).in_process()
                        .with_detail(detail)
                } else {
                    io_error(format!("the room could not be started: {message}"))
                        .with_detail(detail)
                });
            }
        }
    }
    bind(adapter, checked.room_id, tail, ATTACH_WAIT_DEFAULT, started).await
}

async fn attach(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let id = id_param(&map, "session_id")?;
    let tail = tail_param(&map)?;
    let wait = wait_param(&map, ATTACH_WAIT_MAX, ATTACH_WAIT_DEFAULT)?;
    bind(adapter, id, tail, wait, false).await
}

/// What `bind` finds on disk before it connects.
struct Located {
    live: bool,
    info: Option<RoomInfo>,
    dir: Option<PathBuf>,
    sock: Option<PathBuf>,
    shelf: Option<Arc<Shelf>>,
    registry: Option<Arc<RegistryHandle>>,
}

fn locate(env: &RoomEnv, id: &str) -> Result<Located, RpcError> {
    let sock = (env.sock_path)(id).ok();
    let live = env.alive(id);
    let rows = (env.sessions)().map_err(io_error)?;
    let dir = rows
        .iter()
        .find(|row| row.uuid == id)
        .and_then(|row| (env.session_dir)(&row.pwd_hash, &row.uuid).ok());
    let info = room_rows(env, Some(id), Some(id))?
        .into_iter()
        .find(|room| room.session_id == id);
    let shelf = dir
        .as_deref()
        .and_then(|dir| Shelf::open_read_only(&crate::room::room_db_path(dir)).ok())
        .map(Arc::new);
    let registry = dir.as_deref().and_then(|dir| {
        RegistryHandle::with_layers(
            (env.template_dir)(),
            crate::room::registry::overlay_dir_for(dir),
        )
        .ok()
        .map(Arc::new)
    });
    Ok(Located {
        live,
        info,
        dir,
        sock,
        shelf,
        registry,
    })
}

/// Wait until `done` holds, the stream ends or `wait` passes.
async fn wait_for(core: &Core, wait: Duration, done: impl Fn(&Shared) -> bool) {
    let mut changes = core.subscribe();
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        {
            let state = core.lock();
            if done(&state) || !state.live {
                return;
            }
        }
        if tokio::time::timeout_at(deadline, changes.changed())
            .await
            .map_or(true, |changed| changed.is_err())
        {
            return;
        }
    }
}

/// Attach to the live room `id`, take over its stream, and bind it. An
/// earlier ROOM(closed) binding is replaced only when this succeeds.
async fn bind(
    adapter: &mut Adapter,
    id: String,
    tail: usize,
    wait: Duration,
    started: bool,
) -> Result<Value, RpcError> {
    let env = adapter.room_env.clone();
    let located = {
        let id = id.clone();
        let env = env.clone();
        adapter.blocking(move || locate(&env, &id)).await?
    };
    if !located.live {
        return Err(RpcError::reason(Reason::NotRunning, "the room is not running; open it with room.open"));
    }
    let (Some(info), Some(dir), Some(sock)) = (located.info, located.dir, located.sock) else {
        return Err(not_found("unknown room session"));
    };
    let opened = connection::open(&sock).await.map_err(|e| match e {
        OpenError::NotRunning => RpcError::reason(Reason::NotRunning, "the room is not running; open it with room.open"),
        OpenError::NotARoom => RpcError::reason(Reason::NotARoom, "the session's socket is not a room"),
    })?;
    let epoch = uuid::Uuid::new_v4().to_string();
    let core = Arc::new(Core::new(
        id,
        epoch.clone(),
        sock,
        Arc::clone(&adapter.queue),
        Arc::clone(&adapter.scrub),
        adapter.config.clone(),
        located.registry,
        located.shelf,
        opened.hello.replayed,
    ));
    let reader = tokio::spawn(reader::run(Arc::clone(&core), opened.read, opened.frames));
    let mut room = RoomBinding {
        core: Arc::clone(&core),
        socket: Some(opened.socket),
        reader,
        info: RoomInfo {
            bound: true,
            live: true,
            ..info
        },
        session_dir: dir,
        hello: opened.hello,
        roster: Vec::new(),
        roster_truncated: false,
        env,
    };
    room.refresh_roster().await?;
    wait_for(&core, BUDGET, |state| state.replay_left == 0).await;
    wait_for(&core, wait, |state| state.fresh).await;
    if !core.live() {
        return Err(stream_error("the room connection ended during the attach"));
    }
    let result = {
        let mut state = core.lock();
        let page = state.ring.page(None, None, tail, &Filter::default());
        let result = AttachResult {
            room: room.info.clone(),
            started,
            daemon: DaemonFacts {
                version: room.daemon_version().map(str::to_string),
                features: room
                    .hello
                    .features
                    .iter()
                    .take(MAX_FEATURES)
                    .cloned()
                    .collect(),
                template_revision: room.hello.template_revision.clone(),
            },
            epoch,
            history: RoomBinding::history_bounds(&state),
            tail: if tail == 0 { Vec::new() } else { page.lines },
            status: reader::status_now(&state),
            roster: room.roster.clone(),
            roster_truncated: room.roster_truncated,
        };
        core.start_emitting(&mut state);
        result
    };
    let value =
        serde_json::to_value(&result).map_err(|e| RpcError::new("internal", e.to_string()))?;
    let value = room.scrub(value).await;
    adapter.binding = Binding::Room(Box::new(room));
    Ok(value)
}

// ── close, status, history ──────────────────────────────────────────────

pub fn handle_close(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(close(r.params, adapter))
}

async fn close(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let stop = bool_param(&map, "stop")?.unwrap_or(false);
    let room = bound_room_mut(adapter)?;
    room.quit().await;
    let id = room.session_id().to_string();
    let env = room.env.clone();
    adapter.binding = Binding::Unbound;
    if stop {
        let stopped = adapter.blocking(move || Ok((env.stop)(&id))).await?;
        if !stopped {
            return Err(RpcError::reason(Reason::StillRunning, "the room daemon is still running").in_process());
        }
    }
    Ok(json!({"closed": true, "stopped": stop}))
}

/// `detach` in a room binding: a live room gets `quit` first.
pub async fn detach(adapter: &mut Adapter) -> Result<Value, RpcError> {
    if let Binding::Room(room) = &mut adapter.binding {
        room.quit().await;
    }
    Ok(json!({"detached": true}))
}

pub fn handle_status(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(status(r.params, adapter))
}

async fn status(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let _ = object(params)?;
    let room = bound_room(adapter)?;
    let live = room.is_live();
    let cached = room.cached_status();
    let session_id = room.session_id().to_string();
    let shelf = room.core.shelf.clone();
    let status = match cached {
        Some(status) if live => status,
        _ => {
            let disk = adapter
                .blocking(move || {
                    let Some(shelf) = shelf else {
                        return Ok(None);
                    };
                    let unreadable = |e: crate::room::shelf::ShelfError| {
                        RpcError::reason(Reason::RoomUnreadable, format!("the room's shelf cannot be read: {}", e.0))
                    };
                    let fingerprint = shelf.fingerprint().map_err(unreadable)?;
                    let counts = if live {
                        None
                    } else {
                        let items = shelf.scan("").map_err(unreadable)?;
                        Some(crate::room::trajectory::gate::count(&items))
                    };
                    Ok(Some((fingerprint, counts)))
                })
                .await?;
            if !live && disk.is_none() {
                return Err(
                    RpcError::reason(Reason::RoomUnreadable, "the room's shelf cannot be read"),
                );
            }
            let (fingerprint, counts) = disk.unwrap_or_default();
            let room = bound_room(adapter)?;
            reader::blank_status(
                &session_id,
                live,
                room.cached_status().and_then(|s| s.age_ms),
                fingerprint,
                counts.map(|c| StatusCounts {
                    open: c.open as u64,
                    verified: c.verified as u64,
                    returned: c.returned as u64,
                    reports: c.reports as u64,
                }),
            )
        }
    };
    let value =
        serde_json::to_value(&status).map_err(|e| RpcError::new("internal", e.to_string()))?;
    Ok(bound_room(adapter)?.scrub(value).await)
}

pub fn handle_history(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(history(r.params, adapter))
}

async fn history(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let epoch = match map.get("epoch") {
        Some(Value::String(epoch)) => epoch.clone(),
        Some(_) => return Err(out_of_range("epoch must be a string")),
        None => return Err(out_of_range("epoch is required")),
    };
    let after = uint_param(&map, "after_line_seq", 0, u64::MAX)?;
    let before = uint_param(&map, "before_line_seq", 0, u64::MAX)?;
    if after.is_some() && before.is_some() {
        return Err(RpcError::reason(Reason::ConflictingParams, "after_line_seq and before_line_seq exclude each other"));
    }
    let limit = uint_param(&map, "limit", 1, HISTORY_LINES as u64)?.map_or(100, |n| n as usize);
    let kinds = match map.get("kinds") {
        None => None,
        Some(Value::Array(list)) => Some(
            list.iter()
                .map(|kind| {
                    kind.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| out_of_range("kinds must be strings"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(out_of_range("kinds must be a list")),
    };
    let author = match map.get("author") {
        None => None,
        Some(Value::String(author)) => Some(author.clone()),
        Some(_) => return Err(out_of_range("author must be a string")),
    };
    let room = bound_room(adapter)?;
    if epoch != room.epoch() {
        return Err(RpcError::reason(Reason::EpochChanged, "the epoch is not this binding's; start again from the attach result"));
    }
    let result = {
        let state = room.core.lock();
        let page = state
            .ring
            .page(after, before, limit, &Filter { kinds, author });
        HistoryResult {
            epoch,
            first_line_seq: state.ring.first_line_seq(),
            last_line_seq: state.ring.last_line_seq(),
            lines: page.lines,
            has_more_before: page.has_more_before,
            has_more_after: page.has_more_after,
        }
    };
    let value =
        serde_json::to_value(&result).map_err(|e| RpcError::new("internal", e.to_string()))?;
    Ok(room.scrub(value).await)
}

/// `hello.limits.room`: the ring, page, text and tail bounds.
pub fn limits() -> Value {
    json!({
        "ring_lines": ring::RING_LINES,
        "ring_bytes": ring::RING_BYTES,
        "page_bytes": ring::PAGE_BYTES,
        "event_text_bytes": ring::LINE_TEXT_BYTES,
        "status_text_bytes": reader::STATUS_TEXT_BYTES,
        "item_bytes": reads::ITEM_BYTES,
        "tail_lines": TAIL_LINES,
    })
}

//! The `sessions.*` host methods and the `name` step of `create`. Every
//! target is resolved from koma's session registry and run dir by its id;
//! no method takes a path. Listings use the non-sweeping probe and the
//! read-only lock check, so looking at sessions never removes a socket, a
//! pidfile or a lock file.
use crate::app::runtime::sdk::{
    binding::Binding,
    connection::Connection,
    methods::HandlerFuture,
    protocol::{Reason, Request, RpcError},
    Adapter,
};
use crate::ipc::proto::{ClientRequest, SessionStatus};
use crate::model::session_registry::RegRowFullCreated;
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `sessions.list` accepts at most this `limit`.
pub const LIST_MAX: usize = 500;
const LIST_DEFAULT: usize = 100;
/// The whole of `sessions.stop`, probes included.
pub const STOP_BUDGET: Duration = Duration::from_millis(8000);
pub const MAX_NAME_CHARS: usize = 200;
/// The longest id a session can have: a hyphenated uuid.
const MAX_ID_CHARS: usize = 36;

/// The registry, run dir and session dirs as these methods see them. The
/// adapter uses koma's data dir; tests play the registry and the daemons.
pub trait SessionHost: Send + Sync {
    /// Every registry row, newest first.
    fn rows(&self) -> anyhow::Result<Vec<RegRowFullCreated>>;
    /// `Status` replies of every live daemon. Removes nothing.
    fn probe(&self) -> Vec<SessionStatus>;
    /// Whether the daemon socket of `id` accepts a connection.
    fn daemon_alive(&self, id: &str) -> bool;
    fn session_dir(&self, pwd_hash: &str, id: &str) -> anyhow::Result<PathBuf>;
    fn pwd_hash(&self, workdir: &str) -> String;
    fn set_name(&self, id: &str, name: &str) -> anyhow::Result<()>;
    /// Remove the session dir and its registry row.
    fn delete(&self, dir: &Path) -> anyhow::Result<()>;
    /// The bucket directory names under the sessions dir.
    fn buckets(&self) -> anyhow::Result<Vec<String>>;
    /// Stop the daemon of `id`; true when it is gone afterwards.
    fn stop(&self, id: &str) -> bool;

    /// Whether the owner of the session lock is alive. Deletes nothing.
    fn owner_alive(&self, dir: &Path) -> bool {
        crate::model::session_lock::owner_alive(dir)
    }

    fn is_room(&self, dir: &Path) -> bool {
        crate::room::room_db_path(dir).is_file()
    }
}

/// koma's own data dir.
pub struct Home;

/// Unit tests play the registry and the daemons with a host of their own, so
/// a test build never reads, probes or changes koma's real data dir.
fn real_data_dir() -> anyhow::Result<()> {
    if cfg!(test) {
        anyhow::bail!("koma's data dir is not used by unit tests");
    }
    Ok(())
}

impl SessionHost for Home {
    fn rows(&self) -> anyhow::Result<Vec<RegRowFullCreated>> {
        real_data_dir()?;
        crate::model::session_registry::list_all_full()
    }

    fn probe(&self) -> Vec<SessionStatus> {
        if real_data_dir().is_err() {
            return Vec::new();
        }
        crate::app::runtime::manage::probe_live_sessions()
    }

    fn daemon_alive(&self, id: &str) -> bool {
        real_data_dir().is_ok() && crate::app::runtime::manage::daemon_alive(id)
    }

    fn session_dir(&self, pwd_hash: &str, id: &str) -> anyhow::Result<PathBuf> {
        real_data_dir()?;
        crate::model::store::session_dir(pwd_hash, id)
    }

    fn pwd_hash(&self, workdir: &str) -> String {
        crate::model::store::pwd_hash(Path::new(workdir))
    }

    fn set_name(&self, id: &str, name: &str) -> anyhow::Result<()> {
        real_data_dir()?;
        crate::model::session_registry::set_name(id, name)
    }

    fn delete(&self, dir: &Path) -> anyhow::Result<()> {
        real_data_dir()?;
        crate::model::store::delete_session(dir)
    }

    fn buckets(&self) -> anyhow::Result<Vec<String>> {
        real_data_dir()?;
        bucket_names(&crate::model::store::sessions_dir()?)
    }

    fn stop(&self, id: &str) -> bool {
        real_data_dir().is_ok() && crate::app::runtime::manage::kill_session_daemon(id)
    }
}

/// One row of `sessions.list`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionRow {
    pub session_id: String,
    pub name: Option<String>,
    pub workdir: Option<String>,
    pub kind: &'static str,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub live: bool,
    /// `None` for rooms, whose probe never reports work.
    pub working: Option<bool>,
    pub locked: bool,
    pub bound: bool,
    pub parent_session_id: Option<String>,
    pub room_session_id: Option<String>,
    pub room_live: bool,
}

#[derive(Debug, Serialize)]
struct ListResult {
    sessions: Vec<SessionRow>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct DeleteResult {
    deleted: bool,
    kind: &'static str,
    orphaned_room_session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StopResult {
    pub stopped: bool,
    pub was_live: bool,
}

/// An entry of `create`'s `warnings`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CreateWarning {
    pub code: &'static str,
    pub message: String,
}

// ── handlers ───────────────────────────────────────────────────────────────

pub fn handle_list(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(list(Arc::new(Home), r, adapter))
}
pub fn handle_rename(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(rename(Arc::new(Home), r, adapter))
}
pub fn handle_delete(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(delete(Arc::new(Home), r, adapter))
}
pub fn handle_stop(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(stop(Arc::new(Home), r, adapter))
}

/// `sessions.list`.
pub async fn list(
    host: Arc<dyn SessionHost>,
    r: Request,
    adapter: &mut Adapter,
) -> Result<Value, RpcError> {
    let query = ListQuery::parse(r.params)?;
    let bound = bound_id(&adapter.binding);
    let rows = adapter
        .blocking(move || collect(host.as_ref(), bound.as_deref()))
        .await?;
    let (sessions, next_cursor) = page(rows, &query);
    to_value(&ListResult {
        sessions,
        next_cursor,
    })
}

/// `sessions.rename`. The bound coding session is renamed by its daemon;
/// a room, or a coding session that is not live, in the registry. Another
/// live coding session is refused and nothing is sent to its daemon.
pub async fn rename(
    host: Arc<dyn SessionHost>,
    r: Request,
    adapter: &mut Adapter,
) -> Result<Value, RpcError> {
    let map = object(r.params)?;
    let name = session_name(map.get("name"))?;
    let id = session_id(&map)?;
    let bound_coding = adapter
        .binding
        .coding_mut()
        .is_some_and(|connection| connection.projection.id == id);
    if bound_coding {
        let secrets = super::config::daemon_secrets(adapter).await;
        let connection = bound(adapter)?;
        rename_bound(connection, &name, secrets.as_deref()).await?;
    } else {
        let host = host.clone();
        let (id, name) = (id.clone(), name.clone());
        adapter
            .blocking(move || rename_in_registry(host.as_ref(), &id, &name))
            .await?;
    }
    let bound = bound_id(&adapter.binding);
    let row = adapter
        .blocking(move || {
            collect(host.as_ref(), bound.as_deref())?
                .into_iter()
                .find(|row| row.session_id == id)
                .ok_or_else(not_found)
        })
        .await?;
    Ok(json!({ "session": to_value(&row)? }))
}

/// `sessions.delete`, with the guards of `koma daemon delete` in its order.
pub async fn delete(
    host: Arc<dyn SessionHost>,
    r: Request,
    adapter: &mut Adapter,
) -> Result<Value, RpcError> {
    let map = object(r.params)?;
    let id = session_id(&map)?;
    refuse_bound(&adapter.binding, &id)?;
    let result = adapter
        .blocking(move || delete_session(host.as_ref(), &id))
        .await?;
    to_value(&result)
}

/// `sessions.stop`. Bounded by [`STOP_BUDGET`]; a daemon that outlives it
/// is reported as still running.
pub async fn stop(
    host: Arc<dyn SessionHost>,
    r: Request,
    adapter: &mut Adapter,
) -> Result<Value, RpcError> {
    let map = object(r.params)?;
    let id = session_id(&map)?;
    refuse_bound(&adapter.binding, &id)?;
    let result = adapter
        .blocking(move || stop_session(host, id, STOP_BUDGET))
        .await?;
    to_value(&result)
}

// ── create{name} ───────────────────────────────────────────────────────────

/// Name a session `create` just started. A daemon refusal leaves the
/// session created and bound and becomes a warning; an unknown outcome is
/// returned as the error it is.
pub async fn name_created(
    connection: &mut Connection,
    name: &str,
    secrets: Option<&[String]>,
) -> Result<Option<CreateWarning>, RpcError> {
    let sent = connection
        .ack(ClientRequest::RenameSession {
            name: name.to_string(),
        })
        .await;
    let refused = match sent {
        Err(error) if error.delivery == "unknown" => return Err(error),
        Err(error) => Some(super::config::daemon_text(&error.message, secrets)),
        Ok(()) => None,
    };
    let run = connection.readback().await?;
    Ok(match refused {
        Some(message) => Some(rename_failed(message)),
        None if run.name != name => Some(rename_failed(
            "the daemon did not apply the name".to_string(),
        )),
        None => None,
    })
}

fn rename_failed(message: String) -> CreateWarning {
    CreateWarning {
        code: "rename_failed",
        message,
    }
}

/// Rename the bound coding session through its daemon, confirmed by the
/// run state it reports afterwards.
async fn rename_bound(
    connection: &mut Connection,
    name: &str,
    secrets: Option<&[String]>,
) -> Result<(), RpcError> {
    let before = connection.projection.info.name.clone();
    let sent = connection
        .ack(ClientRequest::RenameSession {
            name: name.to_string(),
        })
        .await;
    let refused = match sent {
        Err(error) if error.delivery == "unknown" => return Err(error),
        Err(error) => Some(super::config::daemon_text(&error.message, secrets)),
        Ok(()) => None,
    };
    let run = connection.readback().await?;
    match refused {
        Some(message) if run.name == name && before != name => Err(RpcError::reason(Reason::PartialApply, format!(
            "the daemon reported an error, but the name is in effect: {message}"
        ))
        .with_detail(json!({"in_memory": true, "daemon_error": message}))),
        Some(message) => Err(RpcError::reason(Reason::DaemonRefused, message)),
        None if run.name != name => Err(RpcError::reason(Reason::Unverified, "the daemon acknowledged the rename, but reports another name")
        .written()),
        None => Ok(()),
    }
}

/// A session name: trimmed, 1 to [`MAX_NAME_CHARS`] characters, no control
/// characters.
pub fn session_name(value: Option<&Value>) -> Result<String, RpcError> {
    let raw = match value {
        Some(Value::String(raw)) => raw,
        Some(_) => return Err(invalid_name("name must be a string")),
        None => return Err(invalid_name("name is required")),
    };
    if raw
        .chars()
        .any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
    {
        return Err(invalid_name("name must not contain control characters"));
    }
    let name = raw.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS {
        return Err(invalid_name(format!(
            "name must be 1 to {MAX_NAME_CHARS} characters after trimming"
        )));
    }
    Ok(name.to_string())
}

// ── work on the registry and the daemons ───────────────────────────────────

fn rename_in_registry(host: &dyn SessionHost, id: &str, name: &str) -> Result<(), RpcError> {
    let rows = host.rows().map_err(registry_error)?;
    let row = rows
        .iter()
        .find(|row| row.uuid == id)
        .ok_or_else(not_found)?;
    let room = dir_of(host, row).is_some_and(|dir| host.is_room(&dir));
    if !room && is_live(host, id) {
        return Err(RpcError::reason(Reason::Live, "the session is live: rename it from an adapter bound to it, or stop it first"));
    }
    host.set_name(id, name).map_err(registry_error)?;
    let after = host
        .rows()
        .map_err(|e| unverified_registry(format!("the registry could not be read back: {e}")))?;
    match after.iter().find(|row| row.uuid == id) {
        Some(row) if row.name == name => Ok(()),
        _ => Err(unverified_registry(
            "the registry does not hold the new name after the write".to_string(),
        )),
    }
}

fn unverified_registry(message: String) -> RpcError {
    RpcError::reason(Reason::Unverified, message).in_process()
        .written()
}

/// The directory names in `sessions`; none when it does not exist.
pub fn bucket_names(sessions: &Path) -> anyhow::Result<Vec<String>> {
    let entries = match std::fs::read_dir(sessions) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect())
}

/// The directory a partial delete left behind: its registry row is gone,
/// so it is found by its id in the buckets.
fn leftover_dir(host: &dyn SessionHost, id: &str) -> Result<Option<PathBuf>, RpcError> {
    if !valid_id(id) {
        return Ok(None);
    }
    let buckets = host.buckets().map_err(|e| {
        RpcError::reason(Reason::IoError, format!("the sessions dir could not be read: {e}"))
    })?;
    Ok(buckets
        .iter()
        .filter(|bucket| valid_hash(bucket))
        .filter_map(|bucket| host.session_dir(bucket, id).ok())
        .find(|dir| std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir())))
}

fn delete_session(host: &dyn SessionHost, id: &str) -> Result<DeleteResult, RpcError> {
    if is_live(host, id) {
        return Err(live_refusal());
    }
    let rows = host.rows().map_err(registry_error)?;
    let dir = match rows.iter().find(|row| row.uuid == id) {
        Some(row) => dir_of(host, row).ok_or_else(|| {
            RpcError::reason(Reason::IoError, "the registry row does not name a session directory")
        })?,
        // A retry after a partial delete finishes the removal.
        None => leftover_dir(host, id)?.ok_or_else(not_found)?,
    };
    if host.owner_alive(&dir) {
        return Err(RpcError::reason(Reason::SessionLocked, "the session is locked by a running process"));
    }
    // The daemon may have started since the first probe.
    if host.daemon_alive(id) {
        return Err(live_refusal());
    }
    let room = host.is_room(&dir);
    let orphaned = if room {
        None
    } else {
        let child = crate::room::room_session_id(id);
        rows.iter().any(|row| row.uuid == child).then_some(child)
    };
    if let Err(e) = host.delete(&dir) {
        return Err(partial_delete(format!("removing the session failed: {e}")));
    }
    let row_left = host
        .rows()
        .map_err(|e| partial_delete(format!("the registry could not be read back: {e}")))?
        .iter()
        .any(|row| row.uuid == id);
    if row_left || std::fs::symlink_metadata(&dir).is_ok() {
        return Err(partial_delete(
            "the session directory or its registry row is still present".to_string(),
        ));
    }
    Ok(DeleteResult {
        deleted: true,
        kind: kind(room),
        orphaned_room_session_id: orphaned,
    })
}

/// The work of `sessions.stop` on the blocking pool, bounded by `budget`.
pub fn stop_session(
    host: Arc<dyn SessionHost>,
    id: String,
    budget: Duration,
) -> Result<StopResult, RpcError> {
    let deadline = Instant::now() + budget;
    if !host.daemon_alive(&id) {
        return Ok(StopResult {
            stopped: true,
            was_live: false,
        });
    }
    // The escalation blocks; a thread of its own lets the call keep its
    // bound even when the daemon does not die.
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = host.clone();
    let target = id.clone();
    std::thread::spawn(move || {
        let _ = tx.send(worker.stop(&target));
    });
    let wait = deadline.saturating_duration_since(Instant::now());
    let gone = matches!(rx.recv_timeout(wait), Ok(true)) && !host.daemon_alive(&id);
    if gone {
        Ok(StopResult {
            stopped: true,
            was_live: true,
        })
    } else {
        Err(
            RpcError::reason(Reason::StillRunning, "the session's daemon is still running").in_process(),
        )
    }
}

/// Every session the registry or the run dir knows, unsorted.
fn collect(host: &dyn SessionHost, bound: Option<&str>) -> Result<Vec<SessionRow>, RpcError> {
    let registry = host.rows().map_err(registry_error)?;
    let live: HashMap<String, SessionStatus> = host
        .probe()
        .into_iter()
        .filter(|status| valid_id(&status.session_id))
        .map(|status| (status.session_id.clone(), status))
        .collect();
    let mut rows = Vec::new();
    let mut registered = HashSet::new();
    for row in registry.iter().filter(|row| valid_id(&row.uuid)) {
        if !registered.insert(row.uuid.clone()) {
            continue;
        }
        let dir = dir_of(host, row);
        rows.push(SessionRow {
            session_id: row.uuid.clone(),
            name: Some(row.name.clone()),
            workdir: Some(row.workdir.clone()),
            kind: kind(dir.as_deref().is_some_and(|d| host.is_room(d))),
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
            live: false,
            working: None,
            locked: dir.as_deref().is_some_and(|d| host.owner_alive(d)),
            bound: false,
            parent_session_id: None,
            room_session_id: None,
            room_live: false,
        });
    }
    for (id, status) in &live {
        if registered.contains(id) {
            continue;
        }
        let dir = (!status.pwd.is_empty())
            .then(|| host.session_dir(&host.pwd_hash(&status.pwd), id).ok())
            .flatten();
        rows.push(SessionRow {
            session_id: id.clone(),
            name: None,
            workdir: None,
            kind: kind(dir.as_deref().is_some_and(|d| host.is_room(d))),
            created_at: None,
            updated_at: None,
            live: true,
            working: None,
            locked: dir.as_deref().is_some_and(|d| host.owner_alive(d)),
            bound: false,
            parent_session_id: None,
            room_session_id: None,
            room_live: false,
        });
    }
    let rooms: HashSet<String> = rows
        .iter()
        .filter(|row| row.kind == "room")
        .map(|row| row.session_id.clone())
        .collect();
    let mut parents = HashMap::new();
    for row in rows.iter().filter(|row| row.kind == "coding") {
        parents.insert(
            crate::room::room_session_id(&row.session_id),
            row.session_id.clone(),
        );
    }
    for row in &mut rows {
        let status = live.get(&row.session_id);
        row.live = status.is_some();
        row.bound = bound == Some(row.session_id.as_str());
        if row.kind == "room" {
            row.working = None;
            row.parent_session_id = parents.get(&row.session_id).cloned();
        } else {
            row.working = Some(status.is_some_and(|s| s.working));
            let child = crate::room::room_session_id(&row.session_id);
            if rooms.contains(&child) {
                row.room_live = live.contains_key(&child);
                row.room_session_id = Some(child);
            }
        }
    }
    Ok(rows)
}

/// A live session: its socket accepts, or a probe answered for it.
fn is_live(host: &dyn SessionHost, id: &str) -> bool {
    host.daemon_alive(id) || host.probe().iter().any(|s| s.session_id == id)
}

/// The session dir of a registry row, when the row names a valid bucket.
pub(super) fn dir_of(host: &dyn SessionHost, row: &RegRowFullCreated) -> Option<PathBuf> {
    if !valid_hash(&row.pwd_hash) || !valid_id(&row.uuid) {
        return None;
    }
    host.session_dir(&row.pwd_hash, &row.uuid).ok()
}

fn kind(room: bool) -> &'static str {
    if room {
        "room"
    } else {
        "coding"
    }
}

/// A session id as koma mints them: a hyphenated or a simple uuid. Nothing
/// else can name a session file, so nothing else reaches a path.
fn valid_id(id: &str) -> bool {
    let simple = id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit());
    let hyphenated = id.len() == MAX_ID_CHARS
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        });
    simple || hyphenated
}

/// A bucket name: the simple uuid of a working dir.
fn valid_hash(hash: &str) -> bool {
    hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

// ── paging ─────────────────────────────────────────────────────────────────

struct ListQuery {
    kind: Option<&'static str>,
    live: Option<bool>,
    limit: usize,
    after: Option<(i64, String)>,
}

impl ListQuery {
    fn parse(params: Value) -> Result<Self, RpcError> {
        let map = object(params)?;
        let kind = match map.get("kind") {
            None => None,
            Some(Value::String(kind)) => match kind.as_str() {
                "all" => None,
                "coding" => Some("coding"),
                "room" => Some("room"),
                _ => return Err(out_of_range("kind must be \"coding\", \"room\" or \"all\"")),
            },
            Some(_) => return Err(out_of_range("kind must be a string")),
        };
        let live = match map.get("live") {
            None => None,
            Some(Value::Bool(live)) => Some(*live),
            Some(_) => return Err(out_of_range("live must be a boolean")),
        };
        let limit = match map.get("limit") {
            None => LIST_DEFAULT,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=LIST_MAX as u64).contains(n))
                .map(|n| n as usize)
                .ok_or_else(|| out_of_range(format!("limit must be 1 to {LIST_MAX}")))?,
        };
        let after = match map.get("cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) => Some(decode_cursor(cursor).ok_or_else(bad_cursor)?),
            Some(_) => return Err(bad_cursor()),
        };
        Ok(Self {
            kind,
            live,
            limit,
            after,
        })
    }
}

/// Rows without a registry timestamp are live sockets nobody registered;
/// they sort as newest.
fn sort_key(row: &SessionRow) -> i64 {
    row.updated_at.unwrap_or(i64::MAX)
}

/// Order by `updated_at` descending, then `session_id`, and cut one page
/// after the cursor's position.
fn page(mut rows: Vec<SessionRow>, query: &ListQuery) -> (Vec<SessionRow>, Option<String>) {
    rows.retain(|row| {
        query.kind.is_none_or(|kind| row.kind == kind)
            && query.live.is_none_or(|live| row.live == live)
    });
    rows.sort_by(|a, b| {
        sort_key(b)
            .cmp(&sort_key(a))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    let start = match &query.after {
        None => 0,
        Some((key, id)) => rows
            .iter()
            .position(|row| {
                let k = sort_key(row);
                k < *key || (k == *key && row.session_id.as_str() > id.as_str())
            })
            .unwrap_or(rows.len()),
    };
    let more = rows.len() > start + query.limit;
    let sessions: Vec<SessionRow> = rows.into_iter().skip(start).take(query.limit).collect();
    let next = if more {
        sessions
            .last()
            .map(|row| encode_cursor(sort_key(row), &row.session_id))
    } else {
        None
    };
    (sessions, next)
}

pub fn encode_cursor(key: i64, id: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{key}:{id}"))
}

fn decode_cursor(cursor: &str) -> Option<(i64, String)> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let (key, id) = text.split_once(':')?;
    let key = key.parse().ok()?;
    valid_id(id).then(|| (key, id.to_string()))
}

// ── params, bindings and errors ────────────────────────────────────────────

/// The session this adapter is bound to: its coding session, or its room in
/// ROOM(live) and ROOM(closed).
fn bound_id(binding: &Binding) -> Option<String> {
    match binding {
        Binding::Unbound => None,
        Binding::Coding(connection) => Some(connection.projection.id.clone()),
        Binding::Room(room) => Some(room.session_id().to_string()),
    }
}

fn refuse_bound(binding: &Binding, id: &str) -> Result<(), RpcError> {
    if bound_id(binding).as_deref() == Some(id) {
        Err(
            RpcError::reason(Reason::BoundSelf, "this adapter is bound to that session"),
        )
    } else {
        Ok(())
    }
}

fn bound(adapter: &mut Adapter) -> Result<&mut Connection, RpcError> {
    if adapter.lost.is_some() {
        return Err(RpcError::reason(
            Reason::IoError,
            "the session connection was lost",
        ));
    }
    adapter
        .binding
        .coding_mut()
        .ok_or_else(|| RpcError::reason(Reason::WrongBinding, "no session bound"))
}

pub(super) fn object(params: Value) -> Result<Map<String, Value>, RpcError> {
    match params {
        Value::Object(map) => Ok(map),
        _ => Err(RpcError::reason(Reason::OutOfRange, "params must be an object")),
    }
}

/// `session_id`: required, and a well-formed id. An id koma could not have
/// minted names no session.
pub(super) fn session_id(map: &Map<String, Value>) -> Result<String, RpcError> {
    match map.get("session_id") {
        Some(Value::String(id)) if !id.is_empty() && id.chars().count() <= 64 => {
            if valid_id(id) {
                Ok(id.clone())
            } else {
                Err(not_found())
            }
        }
        Some(_) => Err(out_of_range(
            "session_id must be a string of 1 to 64 characters",
        )),
        None => Err(out_of_range("session_id is required")),
    }
}

pub(super) fn to_value<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::new("internal", e.to_string()))
}

pub(super) fn out_of_range(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::OutOfRange, message)
}

fn invalid_name(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::InvalidName, message)
}

fn bad_cursor() -> RpcError {
    RpcError::reason(Reason::BadCursor, "the cursor does not decode")
}

pub(super) fn not_found() -> RpcError {
    RpcError::reason(Reason::NotFound, "unknown session")
}

fn live_refusal() -> RpcError {
    RpcError::reason(Reason::Live, "the session is live; stop it first")
}

fn partial_delete(message: String) -> RpcError {
    RpcError::reason(Reason::PartialDelete, message).in_process()
        .written()
}

/// A registry failure: `sqlite_busy` when SQLite reported contention.
pub(super) fn registry_error(error: anyhow::Error) -> RpcError {
    let busy = error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    });
    if busy {
        RpcError::reason(Reason::SqliteBusy, format!("the session registry is busy: {error}"))
    } else {
        RpcError::reason(Reason::IoError, format!("the session registry could not be read: {error}"))
    }
}

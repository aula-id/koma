//! `sessions.transcript`: the stored messages of a coding session, read from
//! its `messages.sqlite` archive. The read is in process and read-only: it
//! starts no daemon, sends nothing to one, and never creates, migrates or
//! writes the database. A session that is stopped reads the same as one that
//! is running; a running one may have newer rows by the next call.
use super::redact;
use super::sessions::{
    dir_of, not_found, object, out_of_range, registry_error, session_id, to_value, Home,
    SessionHost,
};
use crate::app::runtime::sdk::{
    methods::HandlerFuture,
    protocol::{Reason, Request, RpcError},
    Adapter,
};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// The most messages in one page, and the default.
pub const MAX_LIMIT: usize = 500;
const DEFAULT_LIMIT: usize = 100;
/// The most bytes of one message's text; a longer one is cut at a character
/// boundary and flagged `truncated`.
pub const MAX_TEXT_BYTES: usize = 262_144;
/// A page stops before it passes this many bytes of text, so it fits a frame.
/// A page always holds at least one message.
pub const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize, PartialEq)]
pub struct TranscriptMessage {
    pub id: i64,
    pub role: String,
    pub text: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TranscriptPage {
    pub session_id: String,
    pub live: bool,
    pub messages: Vec<TranscriptMessage>,
    pub next_since: Option<i64>,
}

pub fn handle_transcript(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(transcript(Arc::new(Home), r, adapter))
}

struct Query {
    session_id: String,
    since: i64,
    limit: usize,
}

impl Query {
    fn parse(params: Value) -> Result<Self, RpcError> {
        let map = object(params)?;
        let session_id = session_id(&map)?;
        let since = match map.get("since") {
            None => 0,
            Some(value) => value
                .as_i64()
                .filter(|n| *n >= 0)
                .ok_or_else(|| out_of_range("since must be an integer of 0 or more"))?,
        };
        let limit = match map.get("limit") {
            None => DEFAULT_LIMIT,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=MAX_LIMIT as u64).contains(n))
                .map(|n| n as usize)
                .ok_or_else(|| out_of_range(format!("limit must be 1 to {MAX_LIMIT}")))?,
        };
        Ok(Self {
            session_id,
            since,
            limit,
        })
    }
}

/// `sessions.transcript`. Any binding may read it.
pub async fn transcript(
    host: Arc<dyn SessionHost>,
    r: Request,
    adapter: &mut Adapter,
) -> Result<Value, RpcError> {
    let query = Query::parse(r.params)?;
    let store = adapter.config.clone();
    let page = adapter
        .blocking(move || {
            let rows = host.rows().map_err(registry_error)?;
            let row = rows
                .iter()
                .find(|row| row.uuid == query.session_id)
                .ok_or_else(not_found)?;
            let dir = dir_of(host.as_ref(), row).ok_or_else(not_found)?;
            if host.is_room(&dir) {
                return Err(out_of_range(
                    "the session is a room; read it with room.history",
                ));
            }
            // Without the secret values nothing can be scrubbed, so an
            // unreadable config fails the call instead of leaking.
            let secrets = redact::secret_values(&store.read()?.config);
            let live = host.daemon_alive(&query.session_id);
            let mut page = read_page(&dir, &query.session_id, live, query.since, query.limit)?;
            for message in &mut page.messages {
                message.text = redact::scrub(&message.text, &secrets);
            }
            Ok(page)
        })
        .await?;
    to_value(&page)
}

/// One page of the archive in `dir`: messages with an id above `since`, oldest
/// first. A session that never stored a message has no database file, and
/// reads as empty.
pub fn read_page(
    dir: &Path,
    session_id: &str,
    live: bool,
    since: i64,
    limit: usize,
) -> Result<TranscriptPage, RpcError> {
    let mut page = TranscriptPage {
        session_id: session_id.to_string(),
        live,
        messages: Vec::new(),
        next_since: None,
    };
    let path = dir.join("messages.sqlite");
    if !path.is_file() {
        return Ok(page);
    }
    let conn =
        Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(db_error)?;
    conn.busy_timeout(Duration::from_millis(2000))
        .map_err(db_error)?;
    let mut stmt = match conn.prepare(
        "SELECT id, role, content, created_at FROM messages WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
    ) {
        Ok(stmt) => stmt,
        // A database file that was never given its tables holds nothing.
        Err(e) if e.to_string().contains("no such table") => return Ok(page),
        Err(e) => return Err(db_error(e)),
    };
    let rows = stmt
        .query_map(rusqlite::params![since, limit as i64 + 1], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(db_error)?;
    let mut bytes = 0usize;
    for row in rows {
        let (id, role, text, created_at) = row.map_err(db_error)?;
        let (text, truncated) = clip(text);
        let over = bytes + text.len() > MAX_PAGE_BYTES;
        if page.messages.len() == limit || (over && !page.messages.is_empty()) {
            page.next_since = page.messages.last().map(|m| m.id);
            break;
        }
        bytes += text.len();
        page.messages.push(TranscriptMessage {
            id,
            role,
            text,
            created_at,
            truncated,
        });
    }
    Ok(page)
}

/// `text` cut to [`MAX_TEXT_BYTES`] at a character boundary.
fn clip(mut text: String) -> (String, bool) {
    if text.len() <= MAX_TEXT_BYTES {
        return (text, false);
    }
    let mut end = MAX_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

/// `sqlite_busy` when SQLite reported contention, otherwise `io_error`.
fn db_error(error: rusqlite::Error) -> RpcError {
    let busy = matches!(
        &error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    );
    if busy {
        RpcError::reason(
            Reason::SqliteBusy,
            format!("the session archive is busy: {error}"),
        )
    } else {
        RpcError::reason(
            Reason::IoError,
            format!("the session archive could not be read: {error}"),
        )
    }
}

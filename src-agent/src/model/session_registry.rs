//! SQLite registry of sessions, keyed by working-directory hash.
//!
//! Lives at `~/.simple-coder/session.sqlite` (see
//! [`crate::model::store::registry_path`]). It is the index that maps each
//! session UUID to the working directory it was opened from (`pwd_hash`), its
//! display name, and its timestamps — so `/resume` can list the sessions for the
//! CURRENT directory without scanning the filesystem, and `/rename` can change a
//! name without moving any directory on disk.
//!
//! Mirrors the rusqlite patterns in [`crate::model::msglog`]: open + ensure
//! schema on every entry point, `execute` with bound params for writes,
//! `prepare` + `query_map` for reads. Timestamps are unix seconds.
//!
//! Wired into the live flows by `store::{create_session, list_sessions,
//! rename_session}` and `Session::{load, save}`: `register` on create, `touch`
//! on save, `set_name` on rename, `list_by_pwd` for the picker, `get` to source
//! a session's display name on load.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::model::store::registry_path;

/// One registry row as listed for a working directory.
///
/// Mirrors the selected columns one-to-one; `list_sessions` consumes `uuid`,
/// `name`, `pwd_hash`, `workdir`, and `updated_at`.
#[derive(Debug, Clone)]
pub struct RegRow {
    pub uuid: String,
    pub pwd_hash: String,
    pub name: String,
    pub workdir: String,
    pub updated_at: i64,
}

/// A single registry row fetched by UUID, including which bucket it belongs to
/// (`pwd_hash`) — needed to locate the session's on-disk directory.
///
/// `Session::load` consumes `name` (the rename source of truth); the remaining
/// columns are carried as a faithful row representation for future callers,
/// hence the field-level `allow`.
#[derive(Debug, Clone)]
pub struct RegRowFull {
    #[allow(dead_code)] // faithful row; name is the only field read today
    pub uuid: String,
    #[allow(dead_code)] // faithful row; name is the only field read today
    pub pwd_hash: String,
    pub name: String,
    #[allow(dead_code)] // faithful row; name is the only field read today
    pub workdir: String,
    #[allow(dead_code)] // faithful row; name is the only field read today
    pub updated_at: i64,
}

/// Open the registry DB and ensure its schema. Centralises the path join so
/// every entry point hits the same file + schema.
fn open() -> Result<Connection> {
    let conn = Connection::open(registry_path()?)?;
    ensure_schema(&conn)?;
    Ok(conn)
}

/// Unix-seconds timestamp, or 0 if the clock is before the epoch (won't happen
/// in practice; keeps the call infallible).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64
}

/// Create the `sessions` table and its `pwd_hash` index if absent. All
/// statements are `IF NOT EXISTS`, so re-running against an existing DB is a
/// no-op.
fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            uuid       TEXT PRIMARY KEY,
            pwd_hash   TEXT NOT NULL,
            name       TEXT NOT NULL,
            workdir    TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_pwd ON sessions(pwd_hash);",
    )?;
    Ok(())
}

/// Insert a new session row. `created_at` and `updated_at` are both set to now.
pub fn register(uuid: &str, pwd_hash: &str, name: &str, workdir: &str) -> Result<()> {
    let conn = open()?;
    let now = now_secs();
    conn.execute(
        "INSERT INTO sessions (uuid, pwd_hash, name, workdir, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![uuid, pwd_hash, name, workdir, now],
    )?;
    Ok(())
}

/// Rename a session: update its `name` and bump `updated_at`.
pub fn set_name(uuid: &str, name: &str) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "UPDATE sessions SET name = ?2, updated_at = ?3 WHERE uuid = ?1",
        rusqlite::params![uuid, name, now_secs()],
    )?;
    Ok(())
}

/// Bump a session's `updated_at` to now (marks it most-recently used so it sorts
/// to the top of its bucket's listing).
pub fn touch(uuid: &str) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "UPDATE sessions SET updated_at = ?2 WHERE uuid = ?1",
        rusqlite::params![uuid, now_secs()],
    )?;
    Ok(())
}

/// All sessions for a working directory, most-recently updated first.
pub fn list_by_pwd(pwd_hash: &str) -> Result<Vec<RegRow>> {
    let conn = open()?;
    let mut stmt = conn.prepare(
        "SELECT uuid, pwd_hash, name, workdir, updated_at FROM sessions
         WHERE pwd_hash = ?1 ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map(rusqlite::params![pwd_hash], |r| {
        Ok(RegRow {
            uuid: r.get(0)?,
            pwd_hash: r.get(1)?,
            name: r.get(2)?,
            workdir: r.get(3)?,
            updated_at: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// All sessions across ALL working directories, most-recently updated first.
pub fn list_all() -> Result<Vec<RegRow>> {
    let conn = open()?;
    let mut stmt = conn.prepare(
        "SELECT uuid, pwd_hash, name, workdir, updated_at FROM sessions ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RegRow {
            uuid: r.get(0)?,
            pwd_hash: r.get(1)?,
            name: r.get(2)?,
            workdir: r.get(3)?,
            updated_at: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Fetch a single session by UUID, or `None` if no such row exists.
pub fn get(uuid: &str) -> Result<Option<RegRowFull>> {
    let conn = open()?;
    let row = conn
        .query_row(
            "SELECT uuid, pwd_hash, name, workdir, updated_at FROM sessions
             WHERE uuid = ?1",
            rusqlite::params![uuid],
            |r| {
                Ok(RegRowFull {
                    uuid: r.get(0)?,
                    pwd_hash: r.get(1)?,
                    name: r.get(2)?,
                    workdir: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Delete a session's registry row by UUID. A missing row is a no-op (DELETE
/// matches zero rows). Used by [`crate::model::store::delete_session`].
pub fn delete_by_uuid(uuid: &str) -> Result<()> {
    let conn = open()?;
    conn.execute(
        "DELETE FROM sessions WHERE uuid = ?1",
        rusqlite::params![uuid],
    )?;
    Ok(())
}

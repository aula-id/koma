//! Per-session persisted RECORDS that survive close/reopen + compaction.
//!
//! Wave-5 SQL persistence: three side tables in the per-session
//! `messages.sqlite` (the same DB the short-send archive uses). They ride that
//! DB because it is already per-session, already opened/migrated idempotently on
//! every access, and — crucially — `/compact` never touches it (compaction
//! rewrites `messages.json`, not this file), so these records persist across
//! compaction AND daemon close/reopen for free.
//!
//! - `file_changes` (#24): a CUMULATIVE log of every workspace file the `write` /
//!   `edit` / `delete` tools touched this session, keyed by path (latest status
//!   wins — an upsert), so the GUI Explore "File changed" panel can render what
//!   THIS session changed (not a git-status snapshot).
//! - `bash_jobs` / `subagents` (#25): the identity + last-known status of every
//!   background-bash job / sub-agent, so those Explore lists survive close/reopen.
//!   The LIVE workers die with the daemon and cannot be resurrected — only the
//!   RECORD is persisted; the app layer restores them INERT (see
//!   `app::runtime::bg_persist`).
//!
//! This layer is deliberately decoupled from the `app::` runtime types: it deals
//! only in plain-`String` records, so the caller owns the enum ⇄ string mapping.
//! All writes are best-effort — callers ignore the error so a DB hiccup never
//! interrupts a tool round or a session load.

use std::path::Path;

use anyhow::Result;

use super::schema::{now_secs, open};

/// One cumulative file-change entry: a workspace path + its latest status
/// (`"added"` / `"modified"` / `"deleted"`). Dedup is by `path` (the table's
/// primary key), so re-touching the same file overwrites its status in place.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub status: String,
}

/// One persisted background-bash job record: the per-session id, the command,
/// and the last-known status (a JSON-encoded `BashJobStatus`, opaque here).
#[derive(Debug, Clone)]
pub struct BashJobRecord {
    pub id: i64,
    pub command: String,
    pub status: String,
}

/// One persisted sub-agent record: the per-session id, the agent name, the
/// compact one-line label, and the last-known status string
/// (`"running"` / `"done"` / `"killed"` / `"error: …"`, matching the wire form).
#[derive(Debug, Clone)]
pub struct SubAgentRecord {
    pub id: i64,
    pub name: String,
    pub label: String,
    pub status: String,
}

/// One persisted file BASELINE: the file's pre-image as it was the FIRST time this
/// session's `write`/`edit`/`delete` tools touched it ("virtual git" — lets the GUI
/// diff tab show session changes in a non-git directory). `kind` says what `content`
/// holds: `"text"` (the original bytes), `"empty"` (file did not exist — created by
/// koma this session), `"binary"` / `"toolarge"` (pre-image not stored, marker only).
#[derive(Debug, Clone)]
pub struct FileBaseline {
    pub kind: String,
    pub content: Option<Vec<u8>>,
}

/// Store a file's baseline pre-image, FIRST TOUCH WINS: `INSERT OR IGNORE` keyed by
/// `path`, so re-touching the same file never overwrites the session's original
/// snapshot (diffs stay cumulative-since-session-start). Best-effort like every
/// side-table write — a DB hiccup must never fail the fs tool that's about to run.
pub fn record_file_baseline(
    session_dir: &Path,
    path: &str,
    kind: &str,
    content: Option<&[u8]>,
) -> Result<()> {
    let conn = open(session_dir)?;
    conn.execute(
        "INSERT OR IGNORE INTO file_baselines (path, kind, content, captured_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![path, kind, content, now_secs()],
    )?;
    Ok(())
}

/// Read one file's baseline pre-image, `None` when the session never captured one
/// (file untouched by koma, or a pre-feature session). Best-effort — an unreadable
/// DB reads as "no baseline".
pub fn read_file_baseline(session_dir: &Path, path: &str) -> Option<FileBaseline> {
    let conn = open(session_dir).ok()?;
    conn.query_row(
        "SELECT kind, content FROM file_baselines WHERE path = ?1",
        rusqlite::params![path],
        |r| {
            Ok(FileBaseline {
                kind: r.get(0)?,
                content: r.get(1)?,
            })
        },
    )
    .ok()
}

/// Upsert one file-change entry (latest status wins). Best-effort.
///
/// Called event-driven from the `write` / `edit` / `delete` tools the instant
/// the fs op succeeds, so the record is durable immediately (an unclean daemon
/// exit still leaves it on disk).
pub fn record_file_change(session_dir: &Path, path: &str, status: &str) -> Result<()> {
    let conn = open(session_dir)?;
    conn.execute(
        "INSERT INTO file_changes (path, status, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET
            status     = excluded.status,
            updated_at = excluded.updated_at",
        rusqlite::params![path, status, now_secs()],
    )?;
    Ok(())
}

/// Read the cumulative file-change log, oldest-touched first. Best-effort — a
/// missing/unreadable DB yields an empty vec (never blocks a load).
pub fn read_file_changes(session_dir: &Path) -> Vec<FileChange> {
    let Ok(conn) = open(session_dir) else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT path, status FROM file_changes ORDER BY updated_at ASC, path ASC")
    else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |r| {
        Ok(FileChange {
            path: r.get(0)?,
            status: r.get(1)?,
        })
    });
    match rows {
        Ok(it) => it.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Replace the persisted bash-job set wholesale (DELETE + INSERT in one txn), so
/// the table always mirrors the live `bash_jobs` vec. Best-effort. Called at
/// job spawn + on the completion drain (the only lifecycle transitions).
pub fn write_bash_jobs(session_dir: &Path, jobs: &[BashJobRecord]) -> Result<()> {
    let mut conn = open(session_dir)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM bash_jobs", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO bash_jobs (id, command, status, updated_at) VALUES (?1, ?2, ?3, ?4)",
        )?;
        let now = now_secs();
        for j in jobs {
            stmt.execute(rusqlite::params![j.id, j.command, j.status, now])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Read the persisted bash-job records, ascending by id. Best-effort.
pub fn read_bash_jobs(session_dir: &Path) -> Vec<BashJobRecord> {
    let Ok(conn) = open(session_dir) else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT id, command, status FROM bash_jobs ORDER BY id ASC")
    else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |r| {
        Ok(BashJobRecord {
            id: r.get(0)?,
            command: r.get(1)?,
            status: r.get(2)?,
        })
    });
    match rows {
        Ok(it) => it.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Replace the persisted sub-agent set wholesale (DELETE + INSERT in one txn).
/// Best-effort. Called at sub-agent spawn + on terminal status transitions.
pub fn write_subagents(session_dir: &Path, agents: &[SubAgentRecord]) -> Result<()> {
    let mut conn = open(session_dir)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM subagents", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO subagents (id, name, label, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let now = now_secs();
        for a in agents {
            stmt.execute(rusqlite::params![a.id, a.name, a.label, a.status, now])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Read the persisted sub-agent records, ascending by id. Best-effort.
pub fn read_subagents(session_dir: &Path) -> Vec<SubAgentRecord> {
    let Ok(conn) = open(session_dir) else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT id, name, label, status FROM subagents ORDER BY id ASC")
    else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |r| {
        Ok(SubAgentRecord {
            id: r.get(0)?,
            name: r.get(1)?,
            label: r.get(2)?,
            status: r.get(3)?,
        })
    });
    match rows {
        Ok(it) => it.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

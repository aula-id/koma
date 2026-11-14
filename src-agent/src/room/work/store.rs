//! SQLite persistence for goals, assignments and review items.
//!
//! The coordinator runs each tool call inside one transaction (`with_tx`), so
//! a call either applies completely or not at all.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use super::model::{Assignment, ReviewItem, ReviewStatus, Work};
use crate::room::types::{AssignmentKind, AssignmentStatus, Outcome, Reference, WorkStatus};

/// Error text shown to the model.
pub type R<T> = Result<T, String>;

pub fn db<T>(result: rusqlite::Result<T>) -> R<T> {
    result.map_err(|e| format!("room storage error: {e}"))
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn enc<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn dec<T: DeserializeOwned>(text: &str) -> rusqlite::Result<T> {
    serde_json::from_value(Value::String(text.to_string())).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn refs_to_text(refs: &[Reference]) -> String {
    serde_json::to_string(refs).unwrap_or_else(|_| "[]".to_string())
}

fn refs_from_text(text: &str) -> rusqlite::Result<Vec<Reference>> {
    serde_json::from_str(text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

const ASSIGNMENT_COLS: &str =
    "id, work_id, dispatch_key, avatar, kind, prompt, reviews_id, status, result, outcome, refs";

fn assignment_from(r: &Row<'_>) -> rusqlite::Result<Assignment> {
    let outcome: Option<String> = r.get(9)?;
    Ok(Assignment {
        id: r.get(0)?,
        work_id: r.get(1)?,
        dispatch_key: r.get(2)?,
        avatar: r.get(3)?,
        kind: dec::<AssignmentKind>(&r.get::<_, String>(4)?)?,
        prompt: r.get(5)?,
        reviews_id: r.get(6)?,
        status: dec::<AssignmentStatus>(&r.get::<_, String>(7)?)?,
        result: r.get(8)?,
        outcome: outcome.as_deref().map(dec::<Outcome>).transpose()?,
        references: refs_from_text(&r.get::<_, String>(10)?)?,
    })
}

const ITEM_COLS: &str = "id, work_id, source_assignment_id, claim_key, claim_revision, snapshot_hash, refs, status, review_assignment_id, verdict_key, feedback_sent";

fn item_from(r: &Row<'_>) -> rusqlite::Result<ReviewItem> {
    Ok(ReviewItem {
        id: r.get(0)?,
        work_id: r.get(1)?,
        source_assignment_id: r.get(2)?,
        claim_key: r.get(3)?,
        claim_revision: r.get::<_, i64>(4)?.max(0) as u64,
        snapshot_hash: r.get(5)?,
        references: refs_from_text(&r.get::<_, String>(6)?)?,
        status: dec::<ReviewStatus>(&r.get::<_, String>(7)?)?,
        review_assignment_id: r.get(8)?,
        verdict_key: r.get(9)?,
        feedback_sent: r.get::<_, i64>(10)? != 0,
    })
}

pub struct WorkStore {
    conn: Mutex<Connection>,
}

impl WorkStore {
    pub fn open(path: &Path) -> R<Self> {
        Self::init(db(Connection::open(path))?)
    }

    pub fn open_in_memory() -> R<Self> {
        Self::init(db(Connection::open_in_memory())?)
    }

    fn init(conn: Connection) -> R<Self> {
        let _mode: String = db(conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0)))?;
        db(conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS works (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE, key TEXT NOT NULL UNIQUE, prompt TEXT NOT NULL,
                 status TEXT NOT NULL, report TEXT
             );
             CREATE TABLE IF NOT EXISTS assignments (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE, work_id TEXT NOT NULL, dispatch_key TEXT NOT NULL,
                 avatar TEXT NOT NULL, kind TEXT NOT NULL, prompt TEXT NOT NULL, reviews_id TEXT,
                 status TEXT NOT NULL, result TEXT, outcome TEXT, refs TEXT NOT NULL DEFAULT '[]',
                 UNIQUE (work_id, dispatch_key, avatar)
             );
             CREATE TABLE IF NOT EXISTS review_items (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE, work_id TEXT NOT NULL, source_assignment_id TEXT NOT NULL,
                 claim_key TEXT NOT NULL, claim_revision INTEGER NOT NULL, snapshot_hash TEXT NOT NULL,
                 refs TEXT NOT NULL, status TEXT NOT NULL, review_assignment_id TEXT,
                 verdict_key TEXT, feedback_sent INTEGER NOT NULL DEFAULT 0,
                 UNIQUE (work_id, claim_key, snapshot_hash)
             );
             CREATE TABLE IF NOT EXISTS fired_nonces (
                 nonce TEXT PRIMARY KEY,
                 work_id TEXT NOT NULL DEFAULT ''
             );",
        ))?;
        migrate_nonce_scope(&conn)?;
        db(conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS fired_nonces_by_work ON fired_nonces (work_id);",
        ))?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record `nonce` for `work_id` if it is new. `Ok(true)` means this call
    /// stored it. The unique key is the only record of what already fired, so
    /// nothing is held in memory.
    pub fn remember_nonce(&self, work_id: &str, nonce: &str) -> R<bool> {
        self.with_tx(|conn| insert_fired_nonce(conn, work_id, nonce))
    }

    /// How many nonces are stored for `work_id`.
    pub fn nonce_count(&self, work_id: &str) -> R<usize> {
        let conn = self.lock();
        let n: i64 = db(conn.query_row(
            "SELECT COUNT(*) FROM fired_nonces WHERE work_id = ?1",
            params![work_id],
            |r| r.get(0),
        ))?;
        Ok(n as usize)
    }

    /// Run `f` in one transaction: commit on `Ok`, roll back on `Err`.
    pub fn with_tx<T>(&self, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
        let mut conn = self.lock();
        let tx = db(conn.transaction())?;
        let out = f(&tx)?;
        db(tx.commit())?;
        Ok(out)
    }
}

// ---- queries (all take the transaction's connection) ----

/// `Ok(true)` when `nonce` was not stored yet. The nonce belongs to `work_id`
/// and goes when that work ends, see [`set_work_status`].
pub fn insert_fired_nonce(conn: &Connection, work_id: &str, nonce: &str) -> R<bool> {
    let n = db(conn.execute(
        "INSERT OR IGNORE INTO fired_nonces (nonce, work_id) VALUES (?1, ?2)",
        params![nonce, work_id],
    ))?;
    Ok(n > 0)
}

/// Forget the nonces of a work that ended. The reconciler only visits active
/// works, so none of them can fire again.
pub fn purge_work_nonces(conn: &Connection, work_id: &str) -> R<()> {
    db(conn.execute("DELETE FROM fired_nonces WHERE work_id = ?1", params![work_id]))?;
    Ok(())
}

/// A database made before nonces were scoped has no `work_id` column. Add it;
/// the old rows keep an empty scope and stay as they were.
fn migrate_nonce_scope(conn: &Connection) -> R<()> {
    let scoped: i64 = db(conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('fired_nonces') WHERE name = 'work_id'",
        [],
        |r| r.get(0),
    ))?;
    if scoped == 0 {
        db(conn.execute_batch("ALTER TABLE fired_nonces ADD COLUMN work_id TEXT NOT NULL DEFAULT ''"))?;
    }
    Ok(())
}

pub fn assignments_of(conn: &Connection, work_id: &str) -> R<Vec<Assignment>> {
    let mut stmt = db(conn.prepare(&format!(
        "SELECT {ASSIGNMENT_COLS} FROM assignments WHERE work_id = ?1 ORDER BY seq"
    )))?;
    let rows = db(stmt.query_map(params![work_id], assignment_from))?;
    db(rows.collect())
}

fn work_with_assignments(
    conn: &Connection,
    row: Option<(String, String, String, String, Option<String>)>,
) -> R<Option<Work>> {
    let Some((id, key, prompt, status, report)) = row else {
        return Ok(None);
    };
    Ok(Some(Work {
        assignments: assignments_of(conn, &id)?,
        status: db(dec::<WorkStatus>(&status))?,
        id,
        key,
        prompt,
        report,
    }))
}

const WORK_SELECT: &str = "SELECT id, key, prompt, status, report FROM works";

fn work_row(r: &Row<'_>) -> rusqlite::Result<(String, String, String, String, Option<String>)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

pub fn work_by_key(conn: &Connection, key: &str) -> R<Option<Work>> {
    let row = db(conn.query_row(&format!("{WORK_SELECT} WHERE key = ?1"), params![key], work_row).optional())?;
    work_with_assignments(conn, row)
}

pub fn work_by_id(conn: &Connection, id: &str) -> R<Option<Work>> {
    let row = db(conn.query_row(&format!("{WORK_SELECT} WHERE id = ?1"), params![id], work_row).optional())?;
    work_with_assignments(conn, row)
}

pub fn active_work(conn: &Connection) -> R<Option<Work>> {
    let row = db(conn
        .query_row(&format!("{WORK_SELECT} WHERE status = 'active' ORDER BY seq LIMIT 1"), [], work_row)
        .optional())?;
    work_with_assignments(conn, row)
}

pub fn latest_work(conn: &Connection) -> R<Option<Work>> {
    let row = db(conn
        .query_row(&format!("{WORK_SELECT} ORDER BY seq DESC LIMIT 1"), [], work_row)
        .optional())?;
    work_with_assignments(conn, row)
}

pub fn all_active_work_ids(conn: &Connection) -> R<Vec<String>> {
    let mut stmt = db(conn.prepare("SELECT id FROM works WHERE status = 'active' ORDER BY seq"))?;
    let rows = db(stmt.query_map([], |r| r.get::<_, String>(0)))?;
    db(rows.collect())
}

pub fn insert_work(conn: &Connection, key: &str, prompt: &str) -> R<Work> {
    let id = new_id();
    db(conn.execute(
        "INSERT INTO works (id, key, prompt, status) VALUES (?1, ?2, ?3, 'active')",
        params![id, key, prompt],
    ))?;
    Ok(Work {
        id,
        key: key.to_string(),
        prompt: prompt.to_string(),
        status: WorkStatus::Active,
        report: None,
        assignments: Vec::new(),
    })
}

pub fn set_work_prompt(conn: &Connection, id: &str, prompt: &str) -> R<()> {
    db(conn.execute("UPDATE works SET prompt = ?2 WHERE id = ?1", params![id, prompt]))?;
    Ok(())
}

pub fn set_work_status(conn: &Connection, id: &str, status: WorkStatus, report: Option<&str>) -> R<()> {
    db(conn.execute(
        "UPDATE works SET status = ?2, report = COALESCE(?3, report) WHERE id = ?1",
        params![id, enc(&status), report],
    ))?;
    if status != WorkStatus::Active {
        purge_work_nonces(conn, id)?;
    }
    Ok(())
}

pub fn insert_assignment(conn: &Connection, a: &Assignment) -> R<()> {
    db(conn.execute(
        &format!("INSERT INTO assignments ({ASSIGNMENT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"),
        params![
            a.id,
            a.work_id,
            a.dispatch_key,
            a.avatar,
            enc(&a.kind),
            a.prompt,
            a.reviews_id,
            enc(&a.status),
            a.result,
            a.outcome.as_ref().map(enc),
            refs_to_text(&a.references),
        ],
    ))?;
    Ok(())
}

pub fn set_assignment_status(conn: &Connection, id: &str, status: AssignmentStatus) -> R<()> {
    db(conn.execute("UPDATE assignments SET status = ?2 WHERE id = ?1", params![id, enc(&status)]))?;
    Ok(())
}

pub fn submit_assignment(
    conn: &Connection,
    id: &str,
    result: &str,
    outcome: Outcome,
    references: &[Reference],
) -> R<()> {
    db(conn.execute(
        "UPDATE assignments SET result = ?2, outcome = ?3, refs = ?4, status = 'submitted' WHERE id = ?1",
        params![id, result, enc(&outcome), refs_to_text(references)],
    ))?;
    Ok(())
}

pub fn review_items(conn: &Connection, work_id: &str) -> R<Vec<ReviewItem>> {
    let mut stmt = db(conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM review_items WHERE work_id = ?1 ORDER BY seq"
    )))?;
    let rows = db(stmt.query_map(params![work_id], item_from))?;
    db(rows.collect())
}

pub fn items_for_review(conn: &Connection, review_assignment_id: &str, status: ReviewStatus) -> R<Vec<ReviewItem>> {
    let mut stmt = db(conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM review_items WHERE review_assignment_id = ?1 AND status = ?2 ORDER BY seq"
    )))?;
    let rows = db(stmt.query_map(params![review_assignment_id, enc(&status)], item_from))?;
    db(rows.collect())
}

pub fn find_item(conn: &Connection, work_id: &str, claim_key: &str, snapshot_hash: &str) -> R<Option<ReviewItem>> {
    db(conn
        .query_row(
            &format!("SELECT {ITEM_COLS} FROM review_items WHERE work_id = ?1 AND claim_key = ?2 AND snapshot_hash = ?3"),
            params![work_id, claim_key, snapshot_hash],
            item_from,
        )
        .optional())
}

pub fn insert_item(conn: &Connection, item: &ReviewItem) -> R<()> {
    db(conn.execute(
        &format!("INSERT INTO review_items ({ITEM_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"),
        params![
            item.id,
            item.work_id,
            item.source_assignment_id,
            item.claim_key,
            item.claim_revision as i64,
            item.snapshot_hash,
            refs_to_text(&item.references),
            enc(&item.status),
            item.review_assignment_id,
            item.verdict_key,
            item.feedback_sent as i64,
        ],
    ))?;
    Ok(())
}

pub fn update_item(
    conn: &Connection,
    id: &str,
    status: ReviewStatus,
    review_assignment_id: Option<&str>,
    verdict_key: Option<&str>,
) -> R<()> {
    db(conn.execute(
        "UPDATE review_items SET status = ?2, review_assignment_id = ?3, verdict_key = COALESCE(?4, verdict_key) WHERE id = ?1",
        params![id, enc(&status), review_assignment_id, verdict_key],
    ))?;
    Ok(())
}

/// Mark every `from`-status item of this claim as `to` (used to supersede stale snapshots).
pub fn retire_items(conn: &Connection, work_id: &str, claim_key: &str, from: &[ReviewStatus], to: ReviewStatus) -> R<()> {
    for status in from {
        db(conn.execute(
            "UPDATE review_items SET status = ?4 WHERE work_id = ?1 AND claim_key = ?2 AND status = ?3",
            params![work_id, claim_key, enc(status), enc(&to)],
        ))?;
    }
    Ok(())
}

/// Record that a returned claim's feedback reached the originating Red.
pub fn mark_feedback_sent(conn: &Connection, id: &str) -> R<()> {
    db(conn.execute("UPDATE review_items SET feedback_sent = 1 WHERE id = ?1", params![id]))?;
    Ok(())
}

/// Re-point failed review items from a retried review assignment onto its replacement.
pub fn reassign_failed_items(conn: &Connection, old_review_id: &str, new_review_id: &str) -> R<()> {
    db(conn.execute(
        "UPDATE review_items SET status = 'assigned', review_assignment_id = ?2
         WHERE review_assignment_id = ?1 AND status = 'failed'",
        params![old_review_id, new_review_id],
    ))?;
    Ok(())
}

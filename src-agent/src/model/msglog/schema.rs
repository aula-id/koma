//! DB connection helpers, schema migrations, and small utility functions.
//!
//! The `reasoning` column on `messages` stores display-only thinking traces
//! from assistant turns. It is NOT indexed by FTS5 (search stays on user-
//! visible content); reasoning is rehydrated from `messages.json` on load
//! and returned as a snippet by `search_messages` for display in
//! `message_find` results.
//!
//! `schema_meta` is a key/value table that gates one-shot migrations (e.g.
//! FTS backfill) so `open()` never re-runs expensive init work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::Connection;

use crate::dto::chat::Role;

/// A heavy message is one whose token estimate clears this bar (~1600 chars).
pub(super) const HEAVY_TOKEN_EST: i64 = 400;
/// Lower bar applied only to tool outputs (they're worth indexing sooner).
pub(super) const TOOL_HEAVY_TOKEN_EST: i64 = 150;
/// How many leading characters of a heavy message to keep as a preview snippet.
/// Bumped to 250 (from 120) so the snippet captures real semantic text rather
/// than getting eaten by leading fences/borders the skip-noise pass already
/// strips. Bigger snippets give the router/fold more to match against.
pub(super) const SNIPPET_CHARS: usize = 250;

/// Canonical lowercase role label stored in the DB.
pub(super) fn role_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// Per-path schema state. Ready paths skip work; InFlight waiters park on the
/// condvar instead of spinning 20 ms polls for the whole multi-minute backfill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemaPathState {
    InFlight,
    Ready,
}

/// Per-process map of paths whose schema has already been run (or is running).
/// Prevents the expensive `ensure_schema` (especially FTS backfill) from
/// executing more than once per `messages.sqlite` file across all `open()`
/// calls in this process — the primary fix for the multi-minute stall on
/// large sessions.
///
/// Entries become `Ready` only after schema/backfill succeeds. The mutex is
/// released while backfill runs so other session DBs are not blocked.
static SCHEMA_READY: OnceLock<(Mutex<HashMap<PathBuf, SchemaPathState>>, Condvar)> =
    OnceLock::new();

/// How long a waiter will park for an in-flight backfill before giving up and
/// trying to claim the path itself (avoids indefinite hang if the owner dies).
const SCHEMA_WAIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Rows per FTS backfill batch. Keeps each INSERT bounded so a huge archive
/// does not lock the connection (and the process-wide ready-set) for minutes
/// in one shot.
const FTS_BACKFILL_BATCH: i64 = 500;

/// Open the session's SQLite archive and run migrations. Centralises the path
/// join so every entry point hits the same file + schema.
///
/// Sets WAL mode + performance PRAGMAs on every connection, then runs
/// `ensure_schema` at most once per file path (guarded by [`SCHEMA_READY`]).
pub fn open(session_dir: &Path) -> Result<Connection> {
    let path = session_dir.join("messages.sqlite");
    let conn = Connection::open(&path)?;
    // WAL + performance PRAGMAs. WAL mode is persistent in the file; the rest
    // are per-connection but harmless to repeat.
    let _ = conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-65536;
         PRAGMA mmap_size=268435456;
         PRAGMA temp_store=MEMORY;",
    );
    ensure_schema_once(&path, &conn)?;
    Ok(conn)
}

/// Run [`ensure_schema`] exactly once per file path.
///
/// Fast path: path already ready → return under a short lock.
/// Slow path: claim the path as in-flight (so concurrent opens of the *same*
/// DB wait on a condvar instead of double-backfilling), drop the lock, run
/// schema+backfill, then mark ready only on success. Other session paths are
/// never blocked by this work.
fn ensure_schema_once(path: &Path, conn: &Connection) -> Result<()> {
    let pair = SCHEMA_READY.get_or_init(|| (Mutex::new(HashMap::new()), Condvar::new()));
    let (lock, cvar) = pair;
    let key = path.to_path_buf();

    loop {
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        match guard.get(&key).copied() {
            Some(SchemaPathState::Ready) => return Ok(()),
            Some(SchemaPathState::InFlight) => {
                // Another thread is backfilling this path — park with a bound.
                let wait_start = Instant::now();
                loop {
                    let (g, timed_out) = match cvar.wait_timeout(guard, Duration::from_secs(1)) {
                        Ok((g, res)) => (g, res.timed_out()),
                        Err(e) => (e.into_inner().0, true),
                    };
                    guard = g;
                    match guard.get(&key).copied() {
                        Some(SchemaPathState::Ready) => return Ok(()),
                        Some(SchemaPathState::InFlight) => {
                            if wait_start.elapsed() >= SCHEMA_WAIT_TIMEOUT {
                                // Owner stuck or dead — drop the stale claim and
                                // try to take over rather than hang forever.
                                guard.remove(&key);
                                break;
                            }
                            if timed_out {
                                continue;
                            }
                        }
                        None => break, // claim freed; fall through to take it
                    }
                }
                // Fall through to try claiming.
            }
            None => {}
        }

        // Claim in-flight.
        if matches!(
            guard.get(&key).copied(),
            Some(SchemaPathState::Ready | SchemaPathState::InFlight)
        ) {
            // Raced with another claim/ready — loop and re-evaluate.
            continue;
        }
        guard.insert(key.clone(), SchemaPathState::InFlight);
        drop(guard);

        let result = ensure_schema(conn);

        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => {
                guard.insert(key, SchemaPathState::Ready);
                cvar.notify_all();
                return Ok(());
            }
            Err(e) => {
                guard.remove(&key);
                cvar.notify_all();
                return Err(e);
            }
        }
    }
}

/// Unix-seconds timestamp, or 0 if the clock is before the epoch (won't happen
/// in practice; keeps the call infallible).
pub(super) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64
}

/// Create the `messages` table if absent, then best-effort add the usage
/// and `reasoning` columns so pre-existing DBs migrate forward. The CREATE
/// includes all columns for fresh DBs; the ALTERs cover existing DBs and
/// intentionally ignore the "duplicate column" error they raise once the
/// columns exist.
///
/// Also creates the Phase-1 side tables (`blobs`, `summary`), the FTS5
/// full-text search index (`messages_fts`), and the `schema_meta` migration-
/// gate table. All statements are `IF NOT EXISTS`, so running this against a
/// DB that already has the tables is a no-op.
///
/// FTS backfill is gated by `schema_meta.fts_backfilled`: once the flag is set,
/// we never re-run the backfill (even if the FTS is somehow emptied). This
/// replaces the old `COUNT(*)` probe which was O(n) on multi-million-row FTS
/// tables and caused multi-minute stalls on every `open()`.
fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            role              TEXT NOT NULL,
            content           TEXT NOT NULL,
            reasoning         TEXT,
            created_at        INTEGER NOT NULL,
            prompt_tokens     INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            cost              REAL NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS blobs (
            id         INTEGER PRIMARY KEY,
            msg_id     INTEGER NOT NULL UNIQUE,
            kind       TEXT NOT NULL,
            token_est  INTEGER NOT NULL,
            snippet    TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS summary (
            id           INTEGER PRIMARY KEY CHECK(id = 1),
            text         TEXT NOT NULL,
            covers_up_to INTEGER NOT NULL,
            sent_start   INTEGER NOT NULL,
            updated_at   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS file_changes (
            path       TEXT PRIMARY KEY,
            status     TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS bash_jobs (
            id         INTEGER PRIMARY KEY,
            command    TEXT NOT NULL,
            status     TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS subagents (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            label      TEXT NOT NULL,
            status     TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS file_baselines (
            path        TEXT PRIMARY KEY,
            kind        TEXT NOT NULL,
            content     BLOB,
            captured_at INTEGER NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            content, role, tokenize='unicode61'
        );

        CREATE TABLE IF NOT EXISTS schema_meta (
            key   TEXT PRIMARY KEY,
            value INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sdlc_nodes (
            id        TEXT PRIMARY KEY,
            parent_id TEXT,
            title     TEXT NOT NULL,
            status    TEXT NOT NULL,
            phase     TEXT,
            notes     TEXT NOT NULL DEFAULT '',
            verify_bit INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL,
            owned_paths TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE IF NOT EXISTS sdlc_events (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id    TEXT,
            kind       TEXT NOT NULL,
            detail     TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS mission_meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    // Migrate older DBs (created before the usage/reasoning columns existed).
    // Errors here are expected once the columns are present, so they're discarded.
    let _ = conn.execute(
        "ALTER TABLE messages ADD COLUMN prompt_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE messages ADD COLUMN completion_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE messages ADD COLUMN cost REAL NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN reasoning TEXT", []);

    // FTS backfill: gated by schema_meta to avoid the O(n) COUNT(*) that caused
    // multi-minute stalls on large sessions. Flag is only set after a successful
    // backfill (or when there is nothing to backfill). If a previous run set the
    // flag but FTS stayed empty while messages exist (failed/interrupted
    // backfill), clear the flag and repair.
    let mut backfilled: i64 = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'fts_backfilled'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if backfilled == 1 {
        let fts_empty: bool = conn
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM messages_fts LIMIT 1)",
                [],
                |r| r.get(0),
            )
            .unwrap_or(true);
        let has_messages: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages LIMIT 1)",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if fts_empty && has_messages {
            // Permanent empty-search state from a prior failed backfill — retry.
            let _ = conn.execute(
                "DELETE FROM schema_meta WHERE key = 'fts_backfilled'",
                [],
            );
            backfilled = 0;
        }
    }

    if backfilled == 0 {
        // Batched backfill by id cursor. Each batch is bounded so open() cannot
        // monopolize the connection for one giant INSERT. Failures leave
        // fts_backfilled unset so the next open retries.
        //
        // Start from max existing FTS rowid so a partial prior attempt resumes
        // instead of re-inserting (duplicate rowid would error on FTS5).
        // Advance the cursor from the *batch* (max id among candidates), not
        // from MAX(messages_fts.rowid) after INSERT — a concurrent writer can
        // append higher FTS rowids and push that MAX past unprocessed messages.
        let mut after_id: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(rowid), 0) FROM messages_fts",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        loop {
            // Bound of this batch from the messages table alone.
            let batch_max: Option<i64> = conn
                .query_row(
                    "SELECT MAX(id) FROM (
                         SELECT id FROM messages WHERE id > ?1 ORDER BY id LIMIT ?2
                     )",
                    rusqlite::params![after_id, FTS_BACKFILL_BATCH],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let Some(batch_max) = batch_max else {
                break;
            };
            conn.execute(
                "INSERT INTO messages_fts(rowid, content, role)
                 SELECT m.id, m.content, m.role
                 FROM messages m
                 WHERE m.id > ?1 AND m.id <= ?2
                   AND NOT EXISTS (
                       SELECT 1 FROM messages_fts f WHERE f.rowid = m.id
                   )
                 ORDER BY m.id",
                rusqlite::params![after_id, batch_max],
            )?;
            after_id = batch_max;
        }
        conn.execute(
            "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('fts_backfilled', 1)",
            [],
        )?;
    }
    Ok(())
}

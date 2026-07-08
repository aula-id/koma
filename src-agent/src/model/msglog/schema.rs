//! DB connection helpers, schema migrations, and small utility functions.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Open the session's SQLite archive and run migrations. Centralises the path
/// join so every entry point hits the same file + schema.
pub(super) fn open(session_dir: &Path) -> Result<Connection> {
    let conn = Connection::open(session_dir.join("messages.sqlite"))?;
    ensure_schema(&conn)?;
    Ok(conn)
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
/// columns so pre-usage DBs migrate forward. The CREATE includes the usage
/// columns for fresh DBs; the ALTERs cover existing DBs and intentionally
/// ignore the "duplicate column" error they raise once the columns exist.
///
/// Also creates the Phase-1 side tables (`blobs`, `summary`). All statements
/// are `IF NOT EXISTS`, so running this against a DB that already has the
/// `messages` table (or already has the side tables) is a no-op.
fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            role              TEXT NOT NULL,
            content           TEXT NOT NULL,
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
        );",
    )?;
    // Migrate older DBs (created before the usage columns existed). Errors here
    // are expected once the columns are present, so they're discarded.
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
    Ok(())
}

//! Core append and query operations on the `messages` table.

use std::path::Path;

use anyhow::Result;

use crate::dto::chat::Role;

use super::blobs::classify_blob;
use super::schema::{now_secs, open, role_str};

/// One archived message row, role + content, as stored in `messages`.
/// Used by [`fetch_messages_since`] for replaying history (P2/P3).
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
#[derive(Debug, Clone)]
pub struct ArchivedMsg {
    pub id: i64,
    pub role: String,
    pub content: String,
}

/// A match from the FTS5 full-text search index (`messages_fts`).
/// Returned by [`search_messages`]; the `snippet` field is FTS5's own
/// `snippet()` output with matching terms highlighted by the marker chars.
#[derive(Debug, Clone)]
pub struct MessageMatch {
    pub id: i64,
    pub role: String,
    pub snippet: String,
    #[allow(dead_code)]
    pub created_at: i64,
}

/// Append one message to the session's SQLite log, creating the DB + tables on
/// first use, and return the inserted `messages.id`. `session_dir` is the
/// session directory (where messages.json lives). `usage` is
/// `(prompt_tokens, completion_tokens, cost)` for assistant messages, or `None`
/// (stored as zeros) for user messages. Best-effort — callers ignore the
/// result.
///
/// The message insert and the heavy-blob index run in one transaction: after
/// the row is written, [`classify_blob`] decides whether to record a `blobs`
/// row keyed by the new `msg_id`. The blob insert is `INSERT OR IGNORE`, so
/// re-indexing the same message id is a no-op. The `messages` table is only
/// ever inserted into — never updated or deleted.
pub fn append(
    session_dir: &Path,
    role: Role,
    content: &str,
    usage: Option<(u64, u64, f64)>,
) -> Result<i64> {
    let mut conn = open(session_dir)?;
    let created_at = now_secs();
    let (pt, ct, cost) = usage.unwrap_or((0, 0, 0.0));

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO messages
            (role, content, created_at, prompt_tokens, completion_tokens, cost)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![role_str(role), content, created_at, pt, ct, cost],
    )?;
    let msg_id = tx.last_insert_rowid();

    // Index heavy content in the SAME transaction so the blob can't be lost if
    // the process dies between the two writes. Idempotent via INSERT OR IGNORE
    // on the UNIQUE msg_id.
    if let Some((kind, token_est, snippet)) = classify_blob(role, content) {
        tx.execute(
            "INSERT OR IGNORE INTO blobs
                (id, msg_id, kind, token_est, snippet, created_at)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![msg_id, kind, token_est, snippet, created_at],
        )?;
    }

    // Keep the FTS5 index in sync: one row per message, keyed by rowid = msg_id.
    tx.execute(
        "INSERT INTO messages_fts(rowid, content, role) VALUES (?1, ?2, ?3)",
        rusqlite::params![msg_id, content, role_str(role)],
    )?;

    tx.commit()?;
    Ok(msg_id)
}

/// Return `(current_context_tokens, output_tokens, cost)` for the session.
///
/// - First value: the most recent assistant request's `prompt_tokens` (current
///   context size). OpenRouter reports the whole context window on each request,
///   so the latest row is the right number — summing would balloon across turns.
/// - Second value: cumulative `completion_tokens` (each turn adds new output).
/// - Third value: cumulative cost (each turn adds new spend).
///
/// Returns `(0, 0, 0.0)` if the DB is absent/unreadable (handled by the caller
/// via `unwrap_or`). A never-written session has no DB yet — `Connection::open`
/// creates an empty file, so `ensure_schema` is run first to make the query
/// valid against a clean schema.
pub fn totals(session_dir: &Path) -> Result<(u64, u64, f64)> {
    let conn = open(session_dir)?;
    let row: (i64, i64, f64) = conn.query_row(
        "SELECT
            COALESCE((SELECT prompt_tokens FROM messages WHERE role = 'assistant' ORDER BY id DESC LIMIT 1), 0),
            COALESCE(SUM(completion_tokens), 0),
            COALESCE(SUM(cost), 0)
         FROM messages",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok((row.0 as u64, row.1 as u64, row.2))
}

/// Fetch up to `limit` archived messages with `id > after_id`, ordered by id
/// ascending. Returns an empty vec if the DB is absent/unreadable. Best-effort.
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
pub fn fetch_messages_since(session_dir: &Path, after_id: i64, limit: i64) -> Vec<ArchivedMsg> {
    fn inner(session_dir: &Path, after_id: i64, limit: i64) -> Result<Vec<ArchivedMsg>> {
        let conn = open(session_dir)?;
        let mut stmt = conn.prepare(
            "SELECT id, role, content FROM messages
             WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![after_id, limit], |r| {
            Ok(ArchivedMsg {
                id: r.get(0)?,
                role: r.get(1)?,
                content: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    inner(session_dir, after_id, limit).unwrap_or_default()
}

/// Sorted ascending ids of all `user`-role messages in the archive. Used to
/// snap the summary fold boundary to a completed-exchange edge. Returns an empty
/// vec if the DB is absent/unreadable. Best-effort.
pub fn user_message_ids(session_dir: &Path) -> Vec<i64> {
    fn inner(session_dir: &Path) -> Result<Vec<i64>> {
        let conn = open(session_dir)?;
        // `role` is stored lowercase by `role_str` (Role::User => "user").
        let mut stmt =
            conn.prepare("SELECT id FROM messages WHERE role = 'user' ORDER BY id ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    inner(session_dir).unwrap_or_default()
}

/// Count of messages in the session's SQLite archive, or `None` if the
/// archive doesn't exist yet (a pre-msglog-era session that has never had
/// `append` called on it — callers should fall back to the legacy
/// `messages.json` count in that case).
///
/// `msglog::append` is only ever called with `Role::User`, `Role::Assistant`,
/// or `Role::Tool` (see the call sites across `app/runtime`) — `Role::System`
/// rows are never written to this table — so a plain `COUNT(*)` already
/// equals "count of non-System messages", matching the legacy
/// `messages.json`-based count without needing a `WHERE role != 'system'`
/// filter.
pub fn message_count(session_dir: &Path) -> Option<usize> {
    // Check existence up front: `open()` (via `Connection::open`) would
    // otherwise create an empty `messages.sqlite` for a session that never
    // had one, silently reporting 0 instead of "no DB, use the fallback".
    if !session_dir.join("messages.sqlite").exists() {
        return None;
    }
    fn inner(session_dir: &Path) -> Result<usize> {
        let conn = open(session_dir)?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
        Ok(n as usize)
    }
    inner(session_dir).ok()
}

/// Highest `messages.id` in the archive, or 0 when empty / unreadable.
/// Best-effort.
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
pub fn max_message_id(session_dir: &Path) -> i64 {
    fn inner(session_dir: &Path) -> Result<i64> {
        let conn = open(session_dir)?;
        let id: i64 = conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM messages",
            [],
            |r| r.get(0),
        )?;
        Ok(id)
    }
    inner(session_dir).unwrap_or(0)
}

/// Truncate the append-only archive to drop every row at or after `cut_id`:
/// delete `messages` (and their `blobs`) WHERE `id >= cut_id`, then rewind the
/// rolling-summary watermark so it can no longer claim to cover dropped content.
///
/// This is the ONE place the otherwise append-only `messages`/`blobs` tables are
/// deleted from. It exists for the message-rewind ("edit a previous message")
/// flow: when the live `Conversation` (and `messages.json`) are truncated to
/// before a user turn, the sqlite archive must be capped at the same boundary so
/// the short-send reshaper (`shape`) can never rehydrate an orphaned blob whose
/// owning message was rewound away. `cut_id` is the sqlite `messages.id` of the
/// FIRST dropped message (the selected user turn); everything `< cut_id` is kept.
///
/// The summary row (id = 1, in the `summary` table) is rewound, NOT deleted: if
/// `covers_up_to >= cut_id` the summary folded in now-dropped messages, so its
/// `covers_up_to`/`sent_start` are clamped to `cut_id - 1` so `shape` only ever
/// rehydrates surviving blobs. The summary TEXT is left as-is (a stale-but-
/// harmless reference; the next fold rewrites it). All in one transaction.
/// Best-effort: callers ignore the error.
pub fn truncate_after(session_dir: &Path, cut_id: i64) -> Result<()> {
    let mut conn = open(session_dir)?;
    let tx = conn.transaction()?;
    // Drop orphaned heavy-blob index rows first (FK-free, but keep it tidy), then
    // the messages themselves. `blobs.msg_id` mirrors `messages.id`.
    tx.execute("DELETE FROM blobs WHERE msg_id >= ?1", rusqlite::params![cut_id])?;
    tx.execute("DELETE FROM messages_fts WHERE rowid >= ?1", rusqlite::params![cut_id])?;
    tx.execute("DELETE FROM messages WHERE id >= ?1", rusqlite::params![cut_id])?;
    // Rewind the summary watermark so it never references a dropped message. Clamp
    // both bookkeeping ids to the last surviving id (`cut_id - 1`, floored at 0).
    let last_kept = (cut_id - 1).max(0);
    tx.execute(
        "UPDATE summary
            SET covers_up_to = MIN(covers_up_to, ?1),
                sent_start   = MIN(sent_start, ?1)
          WHERE id = 1",
        rusqlite::params![last_kept],
    )?;
    tx.commit()?;
    Ok(())
}

/// Full-text search the session's message archive via the FTS5 index.
///
/// `query` is a natural-language search string. Multi-word input is transformed
/// into OR'd prefix terms so the model can search conversationally ("hello test
/// thing" → any message matching "hello*" OR "test*" OR "thing*"). Each term has
/// FTS5 syntax chars stripped and a `*` suffix appended for prefix matching.
/// Results are ranked by BM25 and capped at `limit`. Each result includes a
/// snippet with match context from FTS5's `snippet()`. Best-effort: returns an
/// empty vec on error or empty/whitespace query.
pub fn search_messages(session_dir: &Path, raw_query: &str, limit: i64) -> Vec<MessageMatch> {
    let terms: Vec<String> = raw_query
        .split_whitespace()
        .map(|t| {
            // Strip FTS5 syntax chars so a term like "hello*" or "(test"
            // doesn't break the query. Keep alphanumerics + basic punctuation
            // that carries meaning in natural-language search.
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .map(|t| format!("{}*", t))
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let query = terms.join(" OR ");

    fn inner(session_dir: &Path, query: &str, limit: i64) -> anyhow::Result<Vec<MessageMatch>> {
        let conn = open(session_dir)?;
        // Column 0 = content, column 1 = role. We want snippets from content.
        let mut stmt = conn.prepare(
            "SELECT m.id, m.role, snippet(messages_fts, 0, '', '', '…', 64) AS snip, m.created_at
             FROM messages_fts
             JOIN messages m ON m.id = messages_fts.rowid
             WHERE messages_fts MATCH ?
             ORDER BY rank
             LIMIT ?",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |r| {
            Ok(MessageMatch {
                id: r.get(0)?,
                role: r.get(1)?,
                snippet: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    inner(session_dir, &query, limit).unwrap_or_default()
}

#[cfg(test)]
#[path = "query_test.rs"]
mod query_test;

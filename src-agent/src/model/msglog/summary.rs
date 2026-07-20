//! Rolling-summary read/write for the short-send archive.

use std::path::Path;

use anyhow::Result;
use rusqlite::OptionalExtension;

use super::schema::{now_secs, open};

/// The single rolling-summary row (id = 1). `covers_up_to` is the highest
/// message id folded into `text`; `sent_start` is the first message id that is
/// still sent live (un-summarised). Read by P2/P3.
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
#[derive(Debug, Clone)]
pub struct SummaryRow {
    pub text: String,
    pub covers_up_to: i64,
    pub sent_start: i64,
}

/// Read the single rolling-summary row, or `None` if it hasn't been written
/// yet (or the DB is unreadable — handled by the caller). Best-effort.
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
pub fn read_summary(session_dir: &Path) -> Option<SummaryRow> {
    let conn = open(session_dir).ok()?;
    conn.query_row(
        "SELECT text, covers_up_to, sent_start FROM summary WHERE id = 1",
        [],
        |r| {
            Ok(SummaryRow {
                text: r.get(0)?,
                covers_up_to: r.get(1)?,
                sent_start: r.get(2)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Upsert the single rolling-summary row (id = 1). Best-effort — callers ignore
/// the error. Overwrites the previous summary text + bookkeeping wholesale.
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
pub fn write_summary(
    session_dir: &Path,
    text: &str,
    covers_up_to: i64,
    sent_start: i64,
) -> Result<()> {
    let conn = open(session_dir)?;
    conn.execute(
        "INSERT INTO summary (id, text, covers_up_to, sent_start, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            text         = excluded.text,
            covers_up_to = excluded.covers_up_to,
            sent_start   = excluded.sent_start,
            updated_at   = excluded.updated_at",
        rusqlite::params![text, covers_up_to, sent_start, now_secs()],
    )?;
    Ok(())
}

/// After `/clear`: wipe the rolling short-send summary text and advance the
/// watermark to the current archive tip so the fold step will not re-summarise
/// pre-clear history into a fresh chat. Leaves `messages`/`blobs` untouched
/// (the archive stays). Wrote as empty `text`; short-send `shape` treats empty
/// summary text as absent and skips injection/recall.
pub fn clear_rolling_summary(session_dir: &Path) -> Result<()> {
    let max_id = super::query::max_message_id(session_dir);
    // Empty archive → delete the summary row if present so a new session starts
    // with no summary at all.
    if max_id <= 0 {
        let conn = open(session_dir)?;
        conn.execute("DELETE FROM summary WHERE id = 1", [])?;
        return Ok(());
    }
    // Archive retained: freeze the watermark at the tip + empty the body so fold
    // has nothing new to merge until post-clear tools/messages append past max_id.
    write_summary(session_dir, "", max_id, max_id.saturating_add(1))
}

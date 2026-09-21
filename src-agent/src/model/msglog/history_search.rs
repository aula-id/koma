//! Bounded archive discovery. Search previews never read the reasoning column.
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Latest,
    Oldest,
    Relevance,
}

#[derive(Clone, Debug)]
pub struct Search {
    pub query: Option<String>,
    pub role: Option<String>,
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub order: Order,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub id: i64,
    pub archive_key: Option<String>,
    pub role: String,
    pub excerpt: String,
    pub created_at: Option<i64>,
    pub timestamp: Option<String>,
    pub rank: f64,
}

pub fn fts_query(query: &str) -> Option<String> {
    let words: Vec<_> = query
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
                .collect::<String>()
        })
        .filter(|word| word.chars().count() >= 2)
        .take(5)
        .map(|word| {
            if word.chars().count() >= 3 {
                format!("\"{word}\"*")
            } else {
                format!("\"{word}\"")
            }
        })
        .collect();
    (!words.is_empty()).then(|| format!("content : ({})", words.join(" OR ")))
}

/// Fetch the best `take` candidates from each source before the caller merges
/// sessions. Applying the requested ordering BEFORE LIMIT is essential: sorting
/// a relevance-limited page afterwards cannot produce the latest matches.
pub fn search(session: &Path, options: &Search, take: usize) -> Result<Vec<Hit>> {
    let conn = super::open(session)?;
    let query = options.query.as_deref().and_then(fts_query);
    let mut hits = source(&conn, options, query.as_deref(), take, false)?;
    let recovery_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='drss_recovery_fts')",
        [],
        |row| row.get(0),
    )?;
    // Recovery copies have no reliable original timestamp. A time range must
    // never silently pretend their import time is their conversation time.
    if recovery_exists && options.after.is_none() && options.before.is_none() {
        hits.extend(source(&conn, options, query.as_deref(), take, true)?);
    }
    Ok(hits)
}

fn source(
    conn: &Connection,
    options: &Search,
    query: Option<&str>,
    take: usize,
    recovery: bool,
) -> Result<Vec<Hit>> {
    let (table, fts, id, key, time) = if recovery {
        (
            "drss_recovery",
            "drss_recovery_fts",
            "m.rowid",
            "m.archive_key",
            "NULL",
        )
    } else {
        ("messages", "messages_fts", "m.id", "NULL", "m.created_at")
    };
    let (join, condition, excerpt, rank) = if query.is_some() {
        let passage =
            format!("replace(snippet({fts},0,char(57344),char(57345),' … ',48),char(0),' ')");
        let start = format!("max(1,instr({passage},char(57344))-120)");
        (
            format!("JOIN {fts} ON {fts}.rowid={id}"),
            format!("{fts} MATCH ?1"),
            // FTS chooses a passage near the matching terms, including matches
            // deep in a long message. Center the bounded character slice on
            // the first hit: a preceding 10k-character token must not crowd
            // the matching words out of the returned preview.
            format!("CASE WHEN {start}>1 THEN '… ' ELSE '' END || substr({passage},{start},800)"),
            format!("bm25({fts})"),
        )
    } else {
        (
            String::new(),
            "?1 IS NULL".into(),
            "substr(m.content,1,1600)".into(),
            "0.0".into(),
        )
    };
    let order = match options.order {
        Order::Latest => "created_at IS NULL, created_at DESC, id DESC",
        Order::Oldest => "created_at IS NULL, created_at ASC, id ASC",
        Order::Relevance => "score ASC, created_at IS NULL, created_at DESC, id DESC",
    };
    // All interpolated SQL fragments are fixed internal strings; user values
    // are bound. No original messages or recovery rows are modified.
    let sql = format!(
        "SELECT {id} AS id, {key}, m.role, {excerpt}, {time} AS created_at,
         strftime('%Y-%m-%dT%H:%M:%SZ',{time},'unixepoch'), {rank} AS score
         FROM {table} m {join} WHERE {condition}
         AND (?2 IS NULL OR m.role=?2) AND m.role IN ('user','assistant','tool')
         AND (?3 IS NULL OR {time}>=?3) AND (?4 IS NULL OR {time}<?4)
         ORDER BY {order} LIMIT ?5"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            query,
            options.role,
            options.after,
            options.before,
            take as i64
        ],
        |row| {
            Ok(Hit {
                id: row.get(0)?,
                archive_key: row.get(1)?,
                role: row.get(2)?,
                excerpt: row.get(3)?,
                created_at: row.get(4)?,
                timestamp: row.get(5)?,
                rank: row.get(6)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

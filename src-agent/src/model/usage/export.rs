//! Read-side for the anonymous telemetry drain: rows after a watermark id.
//!
//! One row is one model call. The drain sends `{id, ts, model, tokens_in,
//! tokens_out}` and nothing else (no session, pwd, cost, role).

use rusqlite::Connection;

use super::ledger::open;

/// One ledger row, stripped to the telemetry wire fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryEvent {
    pub id: i64,
    pub ts: i64,
    pub model_id: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

/// Highest `usage.id`, or 0 when the ledger is missing/empty.
pub fn max_usage_id() -> i64 {
    let Some(conn) = open() else {
        return 0;
    };
    max_usage_id_on(&conn)
}

/// Next `limit` rows with `id > after_id`, oldest first.
///
/// **Non-fatal**: empty vec on any DB error.
pub fn events_after(after_id: i64, limit: usize) -> Vec<TelemetryEvent> {
    let Some(conn) = open() else {
        return Vec::new();
    };
    events_after_on(&conn, after_id, limit)
}

pub(crate) fn max_usage_id_on(conn: &Connection) -> i64 {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM usage", [], |r| r.get(0))
        .unwrap_or(0)
}

pub(crate) fn events_after_on(
    conn: &Connection,
    after_id: i64,
    limit: usize,
) -> Vec<TelemetryEvent> {
    let mut stmt = match conn.prepare(
        "SELECT
            id,
            ts,
            COALESCE(model_id, ''),
            COALESCE(tokens_in, 0),
            COALESCE(tokens_out, 0)
         FROM usage
         WHERE id > ?1
         ORDER BY id ASC
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![after_id, limit as i64], |r| {
        Ok(TelemetryEvent {
            id: r.get(0)?,
            ts: r.get(1)?,
            model_id: r.get(2)?,
            tokens_in: r.get(3)?,
            tokens_out: r.get(4)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

#[cfg(test)]
#[path = "export_test.rs"]
mod export_test;

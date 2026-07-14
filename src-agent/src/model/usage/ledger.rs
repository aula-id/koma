use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

/// Ensures [`ensure_schema`] runs at most once per process. `usage_db_path()`
/// resolves through `store::base_dir()` (`~/.koma`), which is a constant
/// per-process (home dir doesn't change mid-run), so every [`open`] call in
/// this process targets the same file — the DDL only needs to run on the
/// first one. [`Once::call_once`] also blocks any concurrent caller until
/// that first run finishes, so nobody queries a not-yet-created table.
static SCHEMA_ONCE: Once = Once::new();

// ── Path + schema helpers ────────────────────────────────────────────────────

/// Path of the global usage ledger: `~/.koma/usage.sqlite`.
pub fn usage_db_path() -> Option<std::path::PathBuf> {
    crate::model::store::base_dir()
        .ok()
        .map(|d| d.join("usage.sqlite"))
}

/// Unix-seconds timestamp, or 0 if the clock is before the epoch.
pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64
}

/// Open the global usage DB and ensure the `usage` table exists.
/// Returns `None` (non-fatal) when the path cannot be resolved or the DB
/// cannot be opened.
pub(crate) fn open() -> Option<Connection> {
    let path = usage_db_path()?;
    // Create parent dirs best-effort so the first call on a clean install works.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path)
        .map_err(|e| {
            crate::model::store::append_global_error_log("usage ledger", &format!("open error: {e}"))
        })
        .ok()?;
    // Run the DDL only on the first `open()` in this process — see
    // `SCHEMA_ONCE`. Every later call reuses the schema created here.
    SCHEMA_ONCE.call_once(|| {
        if let Err(e) = ensure_schema(&conn) {
            crate::model::store::append_global_error_log("usage ledger", &format!("schema error: {e}"));
        }
    });
    Some(conn)
}

/// Create the `usage` table if it does not already exist.
pub(crate) fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage (
            id           INTEGER PRIMARY KEY,
            ts           INTEGER NOT NULL,
            model_id     TEXT,
            role         TEXT,
            session_uuid TEXT,
            pwd_hash     TEXT,
            tokens_in    INTEGER,
            tokens_cached INTEGER,
            tokens_out   INTEGER,
            cost         REAL
        );",
    )
}

// ── Write ────────────────────────────────────────────────────────────────────

/// Record one model call's spend into the global usage ledger.
///
/// **Non-fatal**: any DB error is printed to stderr and silently ignored.
/// The function never panics.
#[allow(clippy::too_many_arguments)]
pub fn record_usage(
    model_id: &str,
    role: &str,
    session_uuid: &str,
    pwd_hash: &str,
    tokens_in: u64,
    tokens_cached: u64,
    tokens_out: u64,
    cost: f64,
) {
    let Some(conn) = open() else { return };
    let ts = now_secs();
    if let Err(e) = conn.execute(
        "INSERT INTO usage
            (ts, model_id, role, session_uuid, pwd_hash,
             tokens_in, tokens_cached, tokens_out, cost)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            ts, model_id, role, session_uuid, pwd_hash,
            tokens_in as i64, tokens_cached as i64, tokens_out as i64, cost
        ],
    ) {
        crate::model::store::append_global_error_log("usage ledger", &format!("insert error: {e}"));
    }
}

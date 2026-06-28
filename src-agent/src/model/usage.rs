//! Global usage ledger: one sqlite at `~/.koma/usage.sqlite` that persists
//! every model-call's token/cost spend across ALL sessions and working dirs.
//!
//! A future `/usage` dashboard can draw heatmaps and top-model spend from
//! this single global file. The ledger is append-only; rows are never updated
//! or deleted.
//!
//! ## Table: `usage`
//!
//! | column          | type    | notes                                    |
//! |-----------------|---------|------------------------------------------|
//! | id              | INTEGER | PRIMARY KEY AUTOINCREMENT                |
//! | ts              | INTEGER | unix seconds (NOT NULL)                  |
//! | model_id        | TEXT    | e.g. `openai/gpt-4o`                     |
//! | role            | TEXT    | `"main"` or `"sub:<agent-name>"`         |
//! | session_uuid    | TEXT    | session id (empty when not in a session) |
//! | pwd_hash        | TEXT    | working-dir bucket key                   |
//! | tokens_in       | INTEGER | prompt tokens for this call              |
//! | tokens_cached   | INTEGER | cached prompt tokens (subset of in)      |
//! | tokens_out      | INTEGER | completion tokens for this call          |
//! | cost            | REAL    | USD cost for this call                   |
//!
//! All writes go through [`record_usage`], which is **non-fatal**: any DB
//! open/insert error is swallowed and logged to stderr; it never panics or
//! returns an `Err` that could interrupt a turn.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Returns the local timezone offset in seconds east of UTC (e.g. UTC+7 → 25200,
/// UTC-5 → -18000). Uses POSIX `localtime_r` via libc. Falls back to 0 on error.
#[allow(unsafe_code)]
pub fn local_utc_offset_secs() -> i64 {
    unsafe {
        let ts = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&ts, &mut tm);
        tm.tm_gmtoff
    }
}

// ── Read-only query types ────────────────────────────────────────────────────

/// Cost for one calendar day, returned by [`daily_costs`].
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DailyCost {
    /// Unix-seconds of midnight UTC for this day (`ts - ts % 86400`).
    pub day_epoch: i64,
    /// Total USD cost recorded on this day.
    pub cost: f64,
}

/// Aggregate spend per model, returned by [`top_models`].
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ModelCost {
    pub model_id: String,
    pub total_cost: f64,
    #[allow(dead_code)]
    pub total_tokens: i64,
    pub call_count: i64,
}

/// Extended per-model row with full token breakdown for range queries.
///
/// Returned by [`top_models_in_range`] and [`session_models`]. Serde-clean (all
/// plain scalars/strings) so it rides the daemon usage-snapshot wire verbatim —
/// the `/usage` dashboard is projected as its already-computed query results so a
/// thin client renders it without any DB access of its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct ModelCostRange {
    pub model_id: String,
    pub total_cost: f64,
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    pub call_count: i64,
}

/// Cost per 7-day window, returned by [`weekly_costs`].
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WeeklyCost {
    /// Unix-seconds of the start of this 7-day bucket (`ts - ts % 604800`).
    pub week_epoch: i64,
    /// Total USD cost in this window.
    pub cost: f64,
}

/// Aggregated totals for a time window, returned by [`range_totals`].
/// Serde-clean so it rides the daemon usage-snapshot wire (see [`ModelCostRange`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct RangeTotals {
    pub cost: f64,
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    /// Number of individual model-call rows in the window.
    pub calls: i64,
}

/// A time-bucketed spend/token sample for heatmaps and sparklines.
///
/// Returned by [`spend_buckets`] and [`session_hourly`]. Serde-clean so the
/// heatmap's already-fetched buckets ride the daemon usage-snapshot wire (see
/// [`ModelCostRange`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct SpendBucket {
    /// Bucket start epoch: floor of the bucket's time unit.
    pub bucket_epoch: i64,
    /// Total USD cost in this bucket.
    pub cost: f64,
    /// Total tokens (in + out) in this bucket.
    pub tokens: i64,
}

/// Role-split aggregate, returned by [`role_split`].
/// Serde-clean so it rides the daemon usage-snapshot wire (see [`ModelCostRange`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct RoleSplit {
    /// USD cost for the `"main"` role.
    pub main_cost: f64,
    /// Call count for the `"main"` role.
    pub main_calls: i64,
    /// USD cost for all `"sub:*"` roles combined.
    pub sub_cost: f64,
    /// Call count for all `"sub:*"` roles combined.
    pub sub_calls: i64,
}

/// Bucket granularity for [`spend_buckets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BucketSize {
    /// 3600-second (1-hour) buckets.
    Hour,
    /// 86400-second (1-day) buckets.
    Day,
    /// 604800-second (7-day) buckets.
    Week,
}

impl BucketSize {
    fn secs(self) -> i64 {
        match self {
            Self::Hour => 3600,
            Self::Day => 86400,
            Self::Week => 604800,
        }
    }
}

// ── Path + schema helpers ────────────────────────────────────────────────────

/// Path of the global usage ledger: `~/.koma/usage.sqlite`.
pub fn usage_db_path() -> Option<std::path::PathBuf> {
    crate::model::store::base_dir()
        .ok()
        .map(|d| d.join("usage.sqlite"))
}

/// Unix-seconds timestamp, or 0 if the clock is before the epoch.
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64
}

/// Open the global usage DB and ensure the `usage` table exists.
/// Returns `None` (non-fatal) when the path cannot be resolved or the DB
/// cannot be opened.
fn open() -> Option<Connection> {
    let path = usage_db_path()?;
    // Create parent dirs best-effort so the first call on a clean install works.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path)
        .map_err(|e| eprintln!("koma: usage ledger open error: {e}"))
        .ok()?;
    ensure_schema(&conn)
        .map_err(|e| eprintln!("koma: usage ledger schema error: {e}"))
        .ok()?;
    Some(conn)
}

/// Create the `usage` table if it does not already exist.
fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
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
        eprintln!("koma: usage ledger insert error: {e}");
    }
}

// ── Read queries (non-fatal, return empty/zero on any DB error) ──────────────

/// Cost per calendar day for the last `days` days (inclusive of today).
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn daily_costs(days: i64) -> Vec<DailyCost> {
    let Some(conn) = open() else { return Vec::new() };
    let cutoff = now_secs() - days * 86400;
    let mut stmt = match conn.prepare(
        "SELECT (ts - ts % 86400) AS day_epoch, COALESCE(SUM(cost), 0.0)
         FROM usage
         WHERE ts >= ?1
         GROUP BY day_epoch
         ORDER BY day_epoch ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![cutoff], |r| {
        Ok(DailyCost {
            day_epoch: r.get(0)?,
            cost: r.get(1)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

/// Top models by total spend, limited to `limit` rows, ordered by cost desc.
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn top_models(limit: i64) -> Vec<ModelCost> {
    let Some(conn) = open() else { return Vec::new() };
    let mut stmt = match conn.prepare(
        "SELECT
            COALESCE(model_id, ''),
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in + tokens_out), 0),
            COUNT(*)
         FROM usage
         GROUP BY model_id
         ORDER BY SUM(cost) DESC
         LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![limit], |r| {
        Ok(ModelCost {
            model_id: r.get(0)?,
            total_cost: r.get(1)?,
            total_tokens: r.get(2)?,
            call_count: r.get(3)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

/// Cost per 7-day window for the last `weeks` weeks, ordered ascending.
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn weekly_costs(weeks: i64) -> Vec<WeeklyCost> {
    let Some(conn) = open() else { return Vec::new() };
    let cutoff = now_secs() - weeks * 604800;
    let mut stmt = match conn.prepare(
        "SELECT (ts - ts % 604800) AS week_epoch, COALESCE(SUM(cost), 0.0)
         FROM usage
         WHERE ts >= ?1
         GROUP BY week_epoch
         ORDER BY week_epoch ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![cutoff], |r| {
        Ok(WeeklyCost {
            week_epoch: r.get(0)?,
            cost: r.get(1)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

// ── New range-aware queries ──────────────────────────────────────────────────

/// Aggregated totals (cost, token breakdown, call count) for rows where
/// `ts >= since_ts`.  Used for the KPI strip.
///
/// **Non-fatal**: returns [`RangeTotals::default()`] (all zeroes) on any DB error.
#[allow(dead_code)]
pub fn range_totals(since_ts: i64) -> RangeTotals {
    let Some(conn) = open() else { return RangeTotals::default() };
    let mut stmt = match conn.prepare(
        "SELECT
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in), 0),
            COALESCE(SUM(tokens_cached), 0),
            COALESCE(SUM(tokens_out), 0),
            COUNT(*)
         FROM usage
         WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return RangeTotals::default(),
    };
    stmt.query_row(rusqlite::params![since_ts], |r| {
        Ok(RangeTotals {
            cost: r.get(0)?,
            tokens_in: r.get(1)?,
            tokens_cached: r.get(2)?,
            tokens_out: r.get(3)?,
            calls: r.get(4)?,
        })
    })
    .unwrap_or_default()
}

/// Top models by spend within a time window (`ts >= since_ts`), with full
/// token breakdown.  Returns at most `limit` rows ordered by cost descending.
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn top_models_in_range(since_ts: i64, limit: i64) -> Vec<ModelCostRange> {
    let Some(conn) = open() else { return Vec::new() };
    let mut stmt = match conn.prepare(
        "SELECT
            COALESCE(model_id, ''),
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in), 0),
            COALESCE(SUM(tokens_cached), 0),
            COALESCE(SUM(tokens_out), 0),
            COUNT(*)
         FROM usage
         WHERE ts >= ?1
         GROUP BY model_id
         ORDER BY SUM(cost) DESC
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![since_ts, limit], |r| {
        Ok(ModelCostRange {
            model_id: r.get(0)?,
            total_cost: r.get(1)?,
            tokens_in: r.get(2)?,
            tokens_cached: r.get(3)?,
            tokens_out: r.get(4)?,
            call_count: r.get(5)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

/// Split spend and call count between the `"main"` role and all `"sub:*"` roles
/// for rows where `ts >= since_ts`.
///
/// **Non-fatal**: returns [`RoleSplit::default()`] (all zeroes) on any DB error.
#[allow(dead_code)]
pub fn role_split(since_ts: i64) -> RoleSplit {
    let Some(conn) = open() else { return RoleSplit::default() };
    let mut stmt = match conn.prepare(
        "SELECT
            COALESCE(SUM(CASE WHEN role = 'main' THEN cost ELSE 0 END), 0.0),
            COALESCE(SUM(CASE WHEN role = 'main' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN role LIKE 'sub:%' THEN cost ELSE 0 END), 0.0),
            COALESCE(SUM(CASE WHEN role LIKE 'sub:%' THEN 1 ELSE 0 END), 0)
         FROM usage
         WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return RoleSplit::default(),
    };
    stmt.query_row(rusqlite::params![since_ts], |r| {
        Ok(RoleSplit {
            main_cost: r.get(0)?,
            main_calls: r.get(1)?,
            sub_cost: r.get(2)?,
            sub_calls: r.get(3)?,
        })
    })
    .unwrap_or_default()
}

/// Time-bucketed spend and token totals for rows where `ts >= since_ts`,
/// grouped into `bucket`-sized windows.
///
/// Only buckets that have at least one row are returned; the caller fills gaps
/// with zeroes as needed.  `_n` is reserved for future limit hints.
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn spend_buckets(since_ts: i64, bucket: BucketSize, _n: usize, tz: i64) -> Vec<SpendBucket> {
    let Some(conn) = open() else { return Vec::new() };
    let secs = bucket.secs();
    // Bucket size is injected as a literal; SQLite does not accept arithmetic
    // expressions in GROUP BY via parameter substitution.
    let sql = format!(
        "SELECT
            ((ts + {tz}) - (ts + {tz}) % {secs} - {tz}) AS bucket_epoch,
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in + tokens_out), 0)
         FROM usage
         WHERE ts >= ?1
         GROUP BY bucket_epoch
         ORDER BY bucket_epoch ASC"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![since_ts], |r| {
        Ok(SpendBucket {
            bucket_epoch: r.get(0)?,
            cost: r.get(1)?,
            tokens: r.get(2)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

/// Per-model spend/token breakdown for a specific session UUID.
/// Used for View B (current-session models table).
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn session_models(session_uuid: &str) -> Vec<ModelCostRange> {
    let Some(conn) = open() else { return Vec::new() };
    let mut stmt = match conn.prepare(
        "SELECT
            COALESCE(model_id, ''),
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in), 0),
            COALESCE(SUM(tokens_cached), 0),
            COALESCE(SUM(tokens_out), 0),
            COUNT(*)
         FROM usage
         WHERE session_uuid = ?1
         GROUP BY model_id
         ORDER BY SUM(cost) DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![session_uuid], |r| {
        Ok(ModelCostRange {
            model_id: r.get(0)?,
            total_cost: r.get(1)?,
            tokens_in: r.get(2)?,
            tokens_cached: r.get(3)?,
            tokens_out: r.get(4)?,
            call_count: r.get(5)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

/// Aggregated totals (cost, token breakdown, call count) for a single session.
/// Used for View B's KPI strip.
///
/// **Non-fatal**: returns [`RangeTotals::default()`] (all zeroes) on any DB error.
#[allow(dead_code)]
pub fn session_totals(session_uuid: &str) -> RangeTotals {
    let Some(conn) = open() else { return RangeTotals::default() };
    let mut stmt = match conn.prepare(
        "SELECT
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in), 0),
            COALESCE(SUM(tokens_cached), 0),
            COALESCE(SUM(tokens_out), 0),
            COUNT(*)
         FROM usage
         WHERE session_uuid = ?1",
    ) {
        Ok(s) => s,
        Err(_) => return RangeTotals::default(),
    };
    stmt.query_row(rusqlite::params![session_uuid], |r| {
        Ok(RangeTotals {
            cost: r.get(0)?,
            tokens_in: r.get(1)?,
            tokens_cached: r.get(2)?,
            tokens_out: r.get(3)?,
            calls: r.get(4)?,
        })
    })
    .unwrap_or_default()
}

/// Hourly spend/token buckets for a specific session UUID, ordered ascending.
/// Used for View B's hourly heatmap.
///
/// **Non-fatal**: returns an empty `Vec` on any DB error.
#[allow(dead_code)]
pub fn session_hourly(session_uuid: &str) -> Vec<SpendBucket> {
    let Some(conn) = open() else { return Vec::new() };
    let mut stmt = match conn.prepare(
        "SELECT
            (ts - ts % 3600) AS bucket_epoch,
            COALESCE(SUM(cost), 0.0),
            COALESCE(SUM(tokens_in + tokens_out), 0)
         FROM usage
         WHERE session_uuid = ?1
         GROUP BY bucket_epoch
         ORDER BY bucket_epoch ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![session_uuid], |r| {
        Ok(SpendBucket {
            bucket_epoch: r.get(0)?,
            cost: r.get(1)?,
            tokens: r.get(2)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().collect()
}

// ── Pre-computed render snapshot (daemon /usage projection) ──────────────────

/// Every ledger query result the `/usage` dashboard renderer reads for ONE frame,
/// gathered into a single plain-data, serde-clean bundle.
///
/// # Why this exists
///
/// The dashboard renderer ([`crate::view::usage`]) normally calls the read queries
/// above DIRECTLY in its draw path (it opens the sqlite ledger every frame). That
/// is fine for a local TUI, but the daemon's THIN attach client renders the same
/// screen from a frozen state projection and has NO database of its own. So the
/// daemon pre-computes this bundle from its ledger and ships it in the snapshot;
/// the client renders the dashboard purely from it. The renderer takes a
/// `&UsageData` and reads it instead of querying, so there is exactly ONE render
/// path (no second hand-rolled client renderer to drift) — see
/// [`crate::view::usage::draw`].
///
/// Only the ACTIVE view's fields are populated by [`collect`](UsageData::collect)
/// (the renderer draws one view at a time); the inactive view's fields stay at
/// their empty/zero default and cost nothing to carry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageData {
    // --- View A (Global) ---
    /// KPI strip totals for the active range (`ts >= since`).
    pub totals: RangeTotals,
    /// Top models by spend in the active range.
    pub top_models: Vec<ModelCostRange>,
    /// Main-vs-sub role split for the active range.
    pub role_split: RoleSplit,
    /// Heatmap buckets for the active range, already fetched at the range's bucket
    /// granularity (hourly for Today, daily for Week/Year). The renderer maps these
    /// absolute `bucket_epoch`s onto its own time grid, so it needs no `since`/DB.
    pub heatmap_buckets: Vec<SpendBucket>,
    // --- View B (Session) ---
    /// Per-model breakdown for the foreground session.
    pub session_models: Vec<ModelCostRange>,
    /// Hourly buckets for the foreground session.
    pub session_hourly: Vec<SpendBucket>,
    /// The foreground session's recorded call count (the only session-totals field
    /// the KPI strip reads from the DB — tokens/cost come from the live runtime
    /// counters, which may be ahead of the ledger mid-turn).
    pub session_calls: i64,
}

impl UsageData {
    /// Gather the dashboard's data for ONE frame from the ledger.
    ///
    /// UI-agnostic on purpose (it takes resolved primitives, not the UI nav enums,
    /// so this `model` layer never depends on the `app`/view layer):
    /// - `session_view` selects which view's fields to populate (`false` = Global A,
    ///   `true` = Session B);
    /// - `since` is the active range's start epoch (Global KPI/models/role-split);
    /// - `(heat_bucket, heat_n)` is the heatmap's bucket granularity + hint for the
    ///   active range (Global only);
    /// - `session_uuid` scopes the Session-view queries.
    ///
    /// All queries are individually non-fatal (empty/zero on a missing ledger), so
    /// this never fails — a fresh install yields an all-default bundle.
    pub fn collect(
        session_view: bool,
        since: i64,
        heat_bucket: BucketSize,
        heat_n: usize,
        session_uuid: &str,
    ) -> Self {
        if session_view {
            UsageData {
                session_models: session_models(session_uuid),
                session_hourly: session_hourly(session_uuid),
                session_calls: session_totals(session_uuid).calls,
                ..Default::default()
            }
        } else {
            UsageData {
                totals: range_totals(since),
                top_models: top_models_in_range(since, 8),
                role_split: role_split(since),
                heatmap_buckets: spend_buckets(since, heat_bucket, heat_n, local_utc_offset_secs()),
                ..Default::default()
            }
        }
    }
}

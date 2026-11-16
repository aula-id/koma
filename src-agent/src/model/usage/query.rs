//! Bounded, read-only usage report over the global ledger.
//!
//! [`run`] answers one [`UsageQuery`]: totals for a time window, plus optional
//! groups (by model, role or session) and optional time buckets (hour, day or
//! ISO week in a caller-chosen UTC offset, empty buckets included).
//!
//! The ledger is opened `SQLITE_OPEN_READ_ONLY` with a short busy timeout, so a
//! missing ledger is never created; it is reported as [`LedgerState::Missing`]
//! with zero totals. Each aggregate is one autocommit statement: a SHARED lock
//! is held only while that statement runs, because a writer's insert fails at
//! once (and its row is dropped) while any reader holds one. The parts of one
//! report may therefore see slightly different row sets.
//!
//! Every query is bounded (window, bucket count, group limit) and runs under a
//! deadline enforced through the connection's interrupt handle. A deadline hit
//! is [`UsageError::Timeout`], never an empty or partial report.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags};

use super::types::{local_utc_offset_secs, BucketSize};

/// Longest window a query may cover: 400 days.
pub const MAX_RANGE_SECS: i64 = 400 * 86_400;
/// Most buckets one query may return.
pub const MAX_BUCKETS: i64 = 1000;
/// Largest group `limit`.
pub const MAX_GROUP_LIMIT: u32 = 500;
/// Group `limit` when the caller gives none.
pub const DEFAULT_GROUP_LIMIT: u32 = 100;
/// Window length when the caller gives no `from_at`: 30 days.
pub const DEFAULT_RANGE_SECS: i64 = 30 * 86_400;
/// Smallest and largest accepted UTC offset, in minutes (UTC-12:00 .. UTC+14:00).
pub const TZ_OFFSET_MINUTES_MIN: i32 = -720;
pub const TZ_OFFSET_MINUTES_MAX: i32 = 840;
/// Latest accepted timestamp (9999-12-31T23:59:59Z). Keeps every offset and
/// bucket computation far from `i64` overflow.
pub const MAX_AT: i64 = 253_402_300_799;
/// Wall-clock budget for one [`run`].
pub const DEADLINE: Duration = Duration::from_secs(2);

/// How long one statement waits on a writer's lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_millis(200);
/// Unix day 4 (1970-01-05) is a Monday: week buckets start on Mondays.
const WEEK_ANCHOR_SECS: i64 = 4 * 86_400;

/// Which column the optional groups aggregate by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)] // `Model` and `Session` are not chosen anywhere in the binary yet
pub enum GroupBy {
    /// No groups.
    #[default]
    None,
    /// One group per `model_id`.
    Model,
    /// One group per raw `role` (`main`, `sub:<agent>`).
    Role,
    /// One group per `session_uuid`.
    Session,
}

/// One usage query. `None` fields take their documented defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageQuery {
    /// Window start, unix seconds, inclusive. Default: `to_at` − 30 days.
    pub from_at: Option<i64>,
    /// Window end, unix seconds, exclusive. Default: the end of the current
    /// second (now + 1), so rows recorded this second are counted.
    pub to_at: Option<i64>,
    /// Only rows of this session id.
    pub session_id: Option<String>,
    /// Only rows recorded in this working directory: an absolute path, mapped
    /// to the ledger's `pwd_hash` by [`crate::model::store::pwd_hash`].
    pub workdir: Option<PathBuf>,
    /// Grouping of the optional groups.
    pub group_by: GroupBy,
    /// Bucket size of the optional time buckets; `None` returns no buckets.
    pub bucket: Option<BucketSize>,
    /// UTC offset of the bucket boundaries, in minutes east of UTC. Default:
    /// the host's current offset.
    pub tz_offset_minutes: Option<i32>,
    /// Most groups returned, 1..=500. Default 100.
    pub limit: Option<u32>,
}

/// Whether the ledger file existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerState {
    /// The ledger was read.
    Ok,
    /// No ledger file exists yet; every aggregate is zero.
    Missing,
}

/// Summed spend of a set of ledger rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageTotals {
    /// Number of model calls (rows).
    pub calls: i64,
    /// Sum of per-call prompt sizes.
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    /// USD.
    pub cost: f64,
}

/// The two kinds of role the ledger records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    /// `main`: the coding Main, or a room's Chief.
    Main,
    /// `sub:<agent>`: a sub-agent, or a room avatar.
    Sub,
}

/// One group of a report.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageGroup {
    /// The grouped column's value (`model_id`, raw `role` or `session_uuid`);
    /// an empty string for rows where it is NULL.
    pub key: String,
    pub totals: UsageTotals,
    /// [`GroupBy::Role`] only: the role's kind; `None` for a role that is
    /// neither `main` nor `sub:<agent>`.
    pub role_kind: Option<RoleKind>,
    /// [`GroupBy::Role`] only: the agent name of a `sub:<agent>` role.
    pub agent: Option<String>,
}

/// One time bucket of a report.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBucket {
    /// Bucket start, unix seconds, aligned to the bucket size in the query's
    /// UTC offset. The first bucket may start before `from_at`; it counts only
    /// rows inside the window.
    pub start_at: i64,
    pub totals: UsageTotals,
}

/// The answer to one [`UsageQuery`].
#[derive(Debug, Clone, PartialEq)]
pub struct UsageReport {
    /// The effective window, defaults applied: `from_at <= ts < to_at`.
    pub from_at: i64,
    pub to_at: i64,
    pub ledger: LedgerState,
    pub totals: UsageTotals,
    /// Present when `group_by` is not [`GroupBy::None`]: sorted by cost
    /// descending, then key ascending, at most `limit` entries.
    pub groups: Option<Vec<UsageGroup>>,
    /// More groups exist than were returned.
    pub groups_truncated: bool,
    /// Present when `bucket` is set: ascending, one per bucket of the window,
    /// empty buckets included.
    pub buckets: Option<Vec<UsageBucket>>,
}

/// Why a query produced no report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageError {
    /// The window is longer than 400 days or spans more than 1000 buckets.
    RangeTooLarge(String),
    /// A parameter is outside its bounds.
    OutOfRange(String),
    /// The deadline passed before every aggregate finished.
    Timeout,
    /// The ledger could not be opened or read.
    Unreadable(String),
}

impl UsageError {
    /// Short machine-readable reason for this error.
    #[allow(dead_code)] // no caller in the binary yet
    pub fn reason(&self) -> &'static str {
        match self {
            Self::RangeTooLarge(_) => "range_too_large",
            Self::OutOfRange(_) => "out_of_range",
            Self::Timeout => "query_timeout",
            Self::Unreadable(_) => "io_error",
        }
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RangeTooLarge(msg) | Self::OutOfRange(msg) => f.write_str(msg),
            Self::Timeout => f.write_str("usage query did not finish within its deadline"),
            Self::Unreadable(msg) => write!(f, "cannot read usage ledger: {msg}"),
        }
    }
}

impl std::error::Error for UsageError {}

/// Run `query` against the global ledger (`~/.koma/usage.sqlite`) under the
/// [`DEADLINE`].
#[allow(dead_code)] // no caller in the binary yet
pub fn run(query: &UsageQuery) -> Result<UsageReport, UsageError> {
    let path = super::ledger::usage_db_path()
        .ok_or_else(|| UsageError::Unreadable("cannot resolve the koma data directory".into()))?;
    run_at(
        &path,
        query,
        super::ledger::now_secs(),
        local_utc_offset_secs(),
        DEADLINE,
    )
}

/// Validated, defaults-applied form of a [`UsageQuery`].
struct Plan {
    from_at: i64,
    to_at: i64,
    session: Option<String>,
    pwd_hash: Option<String>,
    group_by: GroupBy,
    limit: u32,
    buckets: Option<BucketPlan>,
}

/// The bucket grid of a window: `count` buckets of `size` seconds from `origin`.
struct BucketPlan {
    origin: i64,
    size: i64,
    count: i64,
}

/// [`run`] on an explicit ledger path, clock and deadline. `host_tz_secs` is
/// the offset used when the query gives none.
fn run_at(
    path: &Path,
    query: &UsageQuery,
    now: i64,
    host_tz_secs: i64,
    deadline: Duration,
) -> Result<UsageReport, UsageError> {
    let plan = plan(query, now, host_tz_secs)?;
    if is_missing(path) {
        return Ok(empty_report(&plan));
    }
    let conn = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        // Removed between the existence check and the open.
        Err(_) if is_missing(path) => return Ok(empty_report(&plan)),
        Err(e) => return Err(UsageError::Unreadable(e.to_string())),
    };
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| UsageError::Unreadable(e.to_string()))?;
    let guard = Deadline::start(&conn, deadline)?;

    let totals = guard.check(|| totals_on(&conn, &plan))?;
    let (groups, groups_truncated) = match plan.group_by {
        GroupBy::None => (None, false),
        by => {
            let (groups, truncated) = guard.check(|| groups_on(&conn, &plan, by))?;
            (Some(groups), truncated)
        }
    };
    let buckets = match &plan.buckets {
        None => None,
        Some(grid) => Some(guard.check(|| buckets_on(&conn, &plan, grid))?),
    };
    Ok(UsageReport {
        from_at: plan.from_at,
        to_at: plan.to_at,
        ledger: LedgerState::Ok,
        totals,
        groups,
        groups_truncated,
        buckets,
    })
}

fn is_missing(path: &Path) -> bool {
    matches!(std::fs::metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Apply defaults and check every bound before the ledger is touched.
fn plan(query: &UsageQuery, now: i64, host_tz_secs: i64) -> Result<Plan, UsageError> {
    let in_bounds = |name: &str, at: i64| {
        if (0..=MAX_AT).contains(&at) {
            Ok(at)
        } else {
            Err(UsageError::OutOfRange(format!(
                "{name} must be between 0 and {MAX_AT}"
            )))
        }
    };
    let to_at = in_bounds("to_at", query.to_at.unwrap_or(now.saturating_add(1)))?;
    let from_at = in_bounds(
        "from_at",
        query
            .from_at
            .unwrap_or_else(|| to_at.saturating_sub(DEFAULT_RANGE_SECS).max(0)),
    )?;
    if to_at <= from_at {
        return Err(UsageError::OutOfRange(
            "to_at must be greater than from_at".into(),
        ));
    }
    if to_at - from_at > MAX_RANGE_SECS {
        return Err(UsageError::RangeTooLarge(
            "the window may cover at most 400 days".into(),
        ));
    }
    let tz_secs = match query.tz_offset_minutes {
        Some(m) if (TZ_OFFSET_MINUTES_MIN..=TZ_OFFSET_MINUTES_MAX).contains(&m) => {
            i64::from(m) * 60
        }
        Some(_) => {
            return Err(UsageError::OutOfRange(format!(
            "tz_offset_minutes must be between {TZ_OFFSET_MINUTES_MIN} and {TZ_OFFSET_MINUTES_MAX}"
        )))
        }
        None => host_tz_secs.clamp(
            i64::from(TZ_OFFSET_MINUTES_MIN) * 60,
            i64::from(TZ_OFFSET_MINUTES_MAX) * 60,
        ),
    };
    let limit = match query.limit {
        None => DEFAULT_GROUP_LIMIT,
        Some(n) if (1..=MAX_GROUP_LIMIT).contains(&n) => n,
        Some(_) => {
            return Err(UsageError::OutOfRange(format!(
                "limit must be between 1 and {MAX_GROUP_LIMIT}"
            )))
        }
    };
    let pwd_hash = match &query.workdir {
        None => None,
        Some(dir) if dir.is_absolute() => Some(crate::model::store::pwd_hash(dir)),
        Some(_) => {
            return Err(UsageError::OutOfRange(
                "workdir must be an absolute path".into(),
            ))
        }
    };
    let buckets = match query.bucket {
        None => None,
        Some(size) => {
            let secs = size.secs();
            let anchor = match size {
                BucketSize::Week => WEEK_ANCHOR_SECS,
                BucketSize::Hour | BucketSize::Day => 0,
            };
            let align =
                |at: i64| (at + tz_secs - anchor).div_euclid(secs) * secs + anchor - tz_secs;
            let origin = align(from_at);
            let count = (align(to_at - 1) - origin) / secs + 1;
            if count > MAX_BUCKETS {
                return Err(UsageError::RangeTooLarge(format!(
                    "the window spans {count} buckets; at most {MAX_BUCKETS} are allowed"
                )));
            }
            Some(BucketPlan {
                origin,
                size: secs,
                count,
            })
        }
    };
    Ok(Plan {
        from_at,
        to_at,
        session: query.session_id.clone(),
        pwd_hash,
        group_by: query.group_by,
        limit,
        buckets,
    })
}

/// The report of a ledger that does not exist: zero totals, no groups, and
/// every bucket of the window at zero.
fn empty_report(plan: &Plan) -> UsageReport {
    UsageReport {
        from_at: plan.from_at,
        to_at: plan.to_at,
        ledger: LedgerState::Missing,
        totals: UsageTotals::default(),
        groups: (plan.group_by != GroupBy::None).then(Vec::new),
        groups_truncated: false,
        buckets: plan.buckets.as_ref().map(zero_buckets),
    }
}

fn zero_buckets(grid: &BucketPlan) -> Vec<UsageBucket> {
    (0..grid.count)
        .map(|i| UsageBucket {
            start_at: grid.origin + i * grid.size,
            totals: UsageTotals::default(),
        })
        .collect()
}

/// Window and filter clause shared by every aggregate; binds `?1..?4`.
const FILTER: &str = "ts >= ?1 AND ts < ?2
            AND (?3 IS NULL OR session_uuid = ?3)
            AND (?4 IS NULL OR pwd_hash = ?4)";

/// Aggregate columns in [`UsageTotals`] order, read by [`totals_at`].
const SUMS: &str = "COUNT(*),
            COALESCE(SUM(tokens_in), 0),
            COALESCE(SUM(tokens_cached), 0),
            COALESCE(SUM(tokens_out), 0),
            TOTAL(cost)";

/// Read the five [`SUMS`] columns starting at column `first`.
fn totals_at(row: &rusqlite::Row, first: usize) -> rusqlite::Result<UsageTotals> {
    Ok(UsageTotals {
        calls: row.get(first)?,
        tokens_in: row.get(first + 1)?,
        tokens_cached: row.get(first + 2)?,
        tokens_out: row.get(first + 3)?,
        cost: row.get(first + 4)?,
    })
}

fn totals_on(conn: &Connection, plan: &Plan) -> rusqlite::Result<UsageTotals> {
    let sql = format!("SELECT {SUMS} FROM usage WHERE {FILTER}");
    conn.query_row(
        &sql,
        rusqlite::params![plan.from_at, plan.to_at, plan.session, plan.pwd_hash],
        |r| totals_at(r, 0),
    )
}

/// The top `plan.limit` groups, and whether more exist. One row past the
/// limit is fetched to detect truncation.
fn groups_on(
    conn: &Connection,
    plan: &Plan,
    by: GroupBy,
) -> rusqlite::Result<(Vec<UsageGroup>, bool)> {
    let column = match by {
        GroupBy::Model => "model_id",
        GroupBy::Role => "role",
        GroupBy::Session | GroupBy::None => "session_uuid",
    };
    let sql = format!(
        "SELECT COALESCE({column}, '') AS key, {SUMS}
         FROM usage
         WHERE {FILTER}
         GROUP BY key
         ORDER BY TOTAL(cost) DESC, key ASC
         LIMIT ?5"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![
            plan.from_at,
            plan.to_at,
            plan.session,
            plan.pwd_hash,
            i64::from(plan.limit) + 1
        ],
        |r| Ok((r.get::<_, String>(0)?, totals_at(r, 1)?)),
    )?;
    let mut groups = Vec::new();
    let mut truncated = false;
    for row in rows {
        let (key, totals) = row?;
        if groups.len() >= plan.limit as usize {
            truncated = true;
            break;
        }
        let (role_kind, agent) = if by == GroupBy::Role {
            role_parts(&key)
        } else {
            (None, None)
        };
        groups.push(UsageGroup {
            key,
            totals,
            role_kind,
            agent,
        });
    }
    Ok((groups, truncated))
}

/// Split a raw ledger role into its kind and agent name.
fn role_parts(role: &str) -> (Option<RoleKind>, Option<String>) {
    if role == "main" {
        (Some(RoleKind::Main), None)
    } else if let Some(agent) = role.strip_prefix("sub:") {
        (Some(RoleKind::Sub), Some(agent.to_string()))
    } else {
        (None, None)
    }
}

/// Every bucket of `grid`, filled from one GROUP BY over the bucket index.
/// `ts - origin` is never negative because `origin <= from_at <= ts`, so the
/// integer division is a floor.
fn buckets_on(
    conn: &Connection,
    plan: &Plan,
    grid: &BucketPlan,
) -> rusqlite::Result<Vec<UsageBucket>> {
    let sql = format!(
        "SELECT (ts - ?5) / ?6 AS idx, {SUMS}
         FROM usage
         WHERE {FILTER}
         GROUP BY idx
         ORDER BY idx ASC
         LIMIT ?7"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![
            plan.from_at,
            plan.to_at,
            plan.session,
            plan.pwd_hash,
            grid.origin,
            grid.size,
            grid.count
        ],
        |r| Ok((r.get::<_, i64>(0)?, totals_at(r, 1)?)),
    )?;
    let mut buckets = zero_buckets(grid);
    for row in rows {
        let (idx, totals) = row?;
        if let Some(bucket) = usize::try_from(idx).ok().and_then(|i| buckets.get_mut(i)) {
            bucket.totals = totals;
        }
    }
    Ok(buckets)
}

/// Interrupts the connection's running statement once `after` has passed.
///
/// A timer thread waits on a channel; dropping the guard wakes and joins it.
/// `sqlite3_interrupt` is a no-op while no statement runs, so [`Self::check`]
/// also refuses to start a statement once the deadline has fired.
struct Deadline {
    fired: Arc<AtomicBool>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Deadline {
    fn start(conn: &Connection, after: Duration) -> Result<Self, UsageError> {
        let handle = conn.get_interrupt_handle();
        let fired = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&fired);
        let (stop, wait) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("usage-query-deadline".into())
            .spawn(move || {
                if let Err(mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(after) {
                    flag.store(true, Ordering::SeqCst);
                    handle.interrupt();
                }
            })
            .map_err(|e| UsageError::Unreadable(format!("cannot start the query timer: {e}")))?;
        Ok(Self {
            fired,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    fn expired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    /// Run one aggregate unless the deadline has passed, mapping an
    /// interrupted statement to [`UsageError::Timeout`].
    fn check<T>(&self, f: impl FnOnce() -> rusqlite::Result<T>) -> Result<T, UsageError> {
        if self.expired() {
            return Err(UsageError::Timeout);
        }
        f().map_err(|e| {
            let interrupted = e.sqlite_error_code() == Some(ErrorCode::OperationInterrupted);
            if interrupted || self.expired() {
                UsageError::Timeout
            } else {
                UsageError::Unreadable(e.to_string())
            }
        })
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        // Closing the channel wakes the timer before its deadline.
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "00000000-0000-0000-0000-000000000001";
    const S2: &str = "00000000-0000-0000-0000-000000000002";
    const DAY: i64 = 86_400;
    /// 2024-03-04T00:00:00Z, a Monday.
    const MONDAY: i64 = 1_709_510_400;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("koma-usage-query-{}.sqlite", uuid::Uuid::new_v4())),
            )
        }

        fn writer(&self) -> Connection {
            let conn = Connection::open(&self.0).unwrap();
            super::super::ledger::ensure_schema(&conn).unwrap();
            conn
        }

        fn run(&self, query: &UsageQuery) -> Result<UsageReport, UsageError> {
            run_at(&self.0, query, MONDAY + 10 * DAY, 0, DEADLINE)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        conn: &Connection,
        ts: i64,
        model: &str,
        role: &str,
        session: &str,
        pwd: &str,
        tokens: (i64, i64, i64),
        cost: f64,
    ) {
        conn.execute(
            "INSERT INTO usage(ts, model_id, role, session_uuid, pwd_hash,
                               tokens_in, tokens_cached, tokens_out, cost)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![ts, model, role, session, pwd, tokens.0, tokens.1, tokens.2, cost],
        )
        .unwrap();
    }

    /// Six rows: two models, roles `main`/`sub:x`, two sessions, two workdirs.
    fn seeded() -> Fixture {
        let f = Fixture::new();
        let c = f.writer();
        insert(&c, MONDAY, "alpha", "main", S1, "p1", (100, 10, 20), 1.0);
        insert(
            &c,
            MONDAY + 3600,
            "alpha",
            "sub:x",
            S1,
            "p1",
            (50, 0, 5),
            0.5,
        );
        insert(
            &c,
            MONDAY + DAY,
            "beta",
            "main",
            S2,
            "p2",
            (200, 20, 40),
            2.0,
        );
        insert(
            &c,
            MONDAY + DAY + 60,
            "beta",
            "sub:x",
            S2,
            "p2",
            (10, 1, 1),
            0.25,
        );
        insert(
            &c,
            MONDAY + 3 * DAY,
            "alpha",
            "main",
            S2,
            "p1",
            (30, 3, 3),
            0.25,
        );
        insert(
            &c,
            MONDAY + 8 * DAY,
            "beta",
            "main",
            S1,
            "p2",
            (1, 1, 1),
            4.0,
        );
        f
    }

    fn window(from_at: i64, to_at: i64) -> UsageQuery {
        UsageQuery {
            from_at: Some(from_at),
            to_at: Some(to_at),
            ..Default::default()
        }
    }

    fn totals(calls: i64, tokens: (i64, i64, i64), cost: f64) -> UsageTotals {
        UsageTotals {
            calls,
            tokens_in: tokens.0,
            tokens_cached: tokens.1,
            tokens_out: tokens.2,
            cost,
        }
    }

    #[test]
    fn totals_cover_the_half_open_window() {
        let f = seeded();
        let all = f.run(&window(MONDAY, MONDAY + 9 * DAY)).unwrap();
        assert_eq!(all.ledger, LedgerState::Ok);
        assert_eq!(all.totals, totals(6, (391, 35, 70), 8.0));
        assert_eq!(all.groups, None);
        assert_eq!(all.buckets, None);
        // `to_at` is exclusive, `from_at` inclusive.
        let cut = f.run(&window(MONDAY + 3600, MONDAY + 8 * DAY)).unwrap();
        assert_eq!(cut.totals, totals(4, (290, 24, 49), 3.0));
        assert_eq!((cut.from_at, cut.to_at), (MONDAY + 3600, MONDAY + 8 * DAY));
    }

    #[test]
    fn defaults_are_the_last_thirty_days_up_to_now() {
        let f = seeded();
        let now = MONDAY + 10 * DAY;
        let report = f.run(&UsageQuery::default()).unwrap();
        assert_eq!(report.to_at, now + 1);
        assert_eq!(report.from_at, now + 1 - DEFAULT_RANGE_SECS);
        assert_eq!(report.totals.calls, 6);
    }

    #[test]
    fn group_by_model_sorts_by_cost_then_key() {
        let f = seeded();
        let mut q = window(MONDAY, MONDAY + 9 * DAY);
        q.group_by = GroupBy::Model;
        let report = f.run(&q).unwrap();
        let groups = report.groups.unwrap();
        assert!(!report.groups_truncated);
        assert_eq!(
            groups.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(),
            ["beta", "alpha"]
        );
        assert_eq!(groups[0].totals, totals(3, (211, 22, 42), 6.25));
        assert_eq!(groups[1].totals, totals(3, (180, 13, 28), 1.75));
        assert_eq!(
            (groups[0].role_kind, groups[0].agent.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn group_by_role_splits_kind_and_agent() {
        let f = seeded();
        let mut q = window(MONDAY, MONDAY + 9 * DAY);
        q.group_by = GroupBy::Role;
        let groups = f.run(&q).unwrap().groups.unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "main");
        assert_eq!(groups[0].role_kind, Some(RoleKind::Main));
        assert_eq!(groups[0].agent, None);
        assert_eq!(groups[0].totals.calls, 4);
        assert_eq!(groups[0].totals.cost, 7.25);
        assert_eq!(groups[1].key, "sub:x");
        assert_eq!(groups[1].role_kind, Some(RoleKind::Sub));
        assert_eq!(groups[1].agent.as_deref(), Some("x"));
        assert_eq!(groups[1].totals.cost, 0.75);
        assert_eq!(role_parts("planner"), (None, None));
    }

    #[test]
    fn group_by_session_truncates_at_the_limit() {
        let f = seeded();
        let mut q = window(MONDAY, MONDAY + 9 * DAY);
        q.group_by = GroupBy::Session;
        let groups = f.run(&q).unwrap().groups.unwrap();
        assert_eq!(
            groups
                .iter()
                .map(|g| (g.key.as_str(), g.totals.cost))
                .collect::<Vec<_>>(),
            [(S1, 5.5), (S2, 2.5)]
        );
        q.limit = Some(1);
        let report = f.run(&q).unwrap();
        assert!(report.groups_truncated);
        let groups = report.groups.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, S1);
        // The totals are never truncated.
        assert_eq!(report.totals.calls, 6);
        q.limit = Some(2);
        assert!(!f.run(&q).unwrap().groups_truncated);
    }

    #[test]
    fn equal_cost_groups_order_by_key_and_null_keys_are_empty() {
        let f = Fixture::new();
        let c = f.writer();
        insert(&c, MONDAY, "zeta", "main", S1, "p", (1, 0, 1), 1.0);
        insert(&c, MONDAY, "eta", "main", S1, "p", (1, 0, 1), 1.0);
        c.execute(
            "INSERT INTO usage(ts, cost) VALUES (?1, 0.5)",
            rusqlite::params![MONDAY],
        )
        .unwrap();
        let mut q = window(MONDAY, MONDAY + 1);
        q.group_by = GroupBy::Model;
        let report = f.run(&q).unwrap();
        let keys: Vec<_> = report.groups.unwrap().into_iter().map(|g| g.key).collect();
        assert_eq!(keys, ["eta", "zeta", ""]);
        assert_eq!(report.totals, totals(3, (2, 0, 2), 2.5));
    }

    #[test]
    fn day_buckets_follow_the_offset_and_include_empty_days() {
        let f = seeded();
        // UTC+7: local midnight is 17:00Z of the previous UTC day.
        let mut q = window(MONDAY, MONDAY + 4 * DAY);
        q.bucket = Some(BucketSize::Day);
        q.tz_offset_minutes = Some(420);
        let buckets = f.run(&q).unwrap().buckets.unwrap();
        let first = MONDAY - 7 * 3600;
        assert_eq!(
            buckets.iter().map(|b| b.start_at).collect::<Vec<_>>(),
            (0..5).map(|i| first + i * DAY).collect::<Vec<_>>()
        );
        assert_eq!(
            buckets.iter().map(|b| b.totals.calls).collect::<Vec<_>>(),
            [2, 2, 0, 1, 0]
        );
        assert_eq!(buckets[1].totals.cost, 2.25);
        // UTC-00:30: local days start at 00:30Z, which moves the 01:00Z row.
        q.tz_offset_minutes = Some(-30);
        let buckets = f.run(&q).unwrap().buckets.unwrap();
        assert_eq!(buckets[0].start_at, MONDAY - DAY + 1800);
        assert_eq!(
            buckets.iter().map(|b| b.totals.calls).collect::<Vec<_>>(),
            [1, 3, 0, 1, 0]
        );
    }

    #[test]
    fn hour_and_week_buckets() {
        let f = seeded();
        let mut q = window(MONDAY, MONDAY + 3 * 3600);
        q.bucket = Some(BucketSize::Hour);
        let buckets = f.run(&q).unwrap().buckets.unwrap();
        assert_eq!(
            buckets
                .iter()
                .map(|b| (b.start_at - MONDAY, b.totals.calls))
                .collect::<Vec<_>>(),
            [(0, 1), (3600, 1), (7200, 0)]
        );
        // Weeks start on Monday in the query's offset.
        let mut q = window(MONDAY - DAY, MONDAY + 9 * DAY);
        q.bucket = Some(BucketSize::Week);
        q.tz_offset_minutes = Some(0);
        let buckets = f.run(&q).unwrap().buckets.unwrap();
        assert_eq!(
            buckets
                .iter()
                .map(|b| (b.start_at, b.totals.calls))
                .collect::<Vec<_>>(),
            [(MONDAY - 7 * DAY, 0), (MONDAY, 5), (MONDAY + 7 * DAY, 1)]
        );
    }

    #[test]
    fn filters_by_session_and_workdir() {
        let f = seeded();
        let mut q = window(MONDAY, MONDAY + 9 * DAY);
        q.session_id = Some(S2.into());
        assert_eq!(f.run(&q).unwrap().totals.calls, 3);
        q.session_id = Some("nobody".into());
        assert_eq!(f.run(&q).unwrap().totals, UsageTotals::default());

        let dir = std::env::temp_dir();
        let hash = crate::model::store::pwd_hash(&dir);
        let c = f.writer();
        insert(&c, MONDAY + 5, "gamma", "main", S1, &hash, (7, 0, 7), 0.125);
        let mut q = window(MONDAY, MONDAY + 9 * DAY);
        q.workdir = Some(dir);
        q.group_by = GroupBy::Model;
        q.bucket = Some(BucketSize::Day);
        let report = f.run(&q).unwrap();
        assert_eq!(report.totals, totals(1, (7, 0, 7), 0.125));
        assert_eq!(report.groups.unwrap()[0].key, "gamma");
        let buckets = report.buckets.unwrap();
        assert_eq!(buckets.len(), 9);
        assert_eq!(buckets.iter().map(|b| b.totals.calls).sum::<i64>(), 1);
        q.session_id = Some(S2.into());
        assert_eq!(f.run(&q).unwrap().totals.calls, 0);
    }

    #[test]
    fn missing_ledger_is_zero_and_never_created() {
        let f = Fixture::new();
        let mut q = window(MONDAY, MONDAY + 3 * DAY);
        q.group_by = GroupBy::Role;
        q.bucket = Some(BucketSize::Day);
        let report = f.run(&q).unwrap();
        assert_eq!(report.ledger, LedgerState::Missing);
        assert_eq!(report.totals, UsageTotals::default());
        assert_eq!(report.groups, Some(Vec::new()));
        assert!(!report.groups_truncated);
        let buckets = report.buckets.unwrap();
        assert_eq!(buckets.len(), 3);
        assert!(buckets.iter().all(|b| b.totals == UsageTotals::default()));
        assert!(!f.0.exists());
        // Bounds are still checked first.
        let err = f.run(&window(MONDAY, MONDAY)).unwrap_err();
        assert_eq!(err.reason(), "out_of_range");
        assert!(!f.0.exists());
    }

    #[test]
    fn reading_never_writes_the_ledger() {
        let f = seeded();
        let before = std::fs::read(&f.0).unwrap();
        let mut q = window(MONDAY, MONDAY + 9 * DAY);
        q.group_by = GroupBy::Session;
        q.bucket = Some(BucketSize::Hour);
        f.run(&q).unwrap();
        assert_eq!(before, std::fs::read(&f.0).unwrap());
    }

    #[test]
    fn unreadable_ledger_is_an_error() {
        let f = Fixture::new();
        std::fs::write(&f.0, b"not a database").unwrap();
        let err = f.run(&window(MONDAY, MONDAY + DAY)).unwrap_err();
        assert_eq!(err.reason(), "io_error");
        assert_eq!(std::fs::read(&f.0).unwrap(), b"not a database");
    }

    #[test]
    fn bounds_are_refused() {
        let f = seeded();
        let reason = |q: UsageQuery| f.run(&q).unwrap_err().reason();
        assert_eq!(reason(window(0, MAX_RANGE_SECS + 1)), "range_too_large");
        assert!(f.run(&window(0, MAX_RANGE_SECS)).is_ok());
        let mut q = window(MONDAY, MONDAY + 1001 * 3600);
        q.bucket = Some(BucketSize::Hour);
        assert_eq!(reason(q.clone()), "range_too_large");
        q.to_at = Some(MONDAY + 1000 * 3600);
        assert_eq!(f.run(&q).unwrap().buckets.unwrap().len(), 1000);
        assert_eq!(reason(window(MONDAY, MONDAY - 1)), "out_of_range");
        assert_eq!(reason(window(-1, MONDAY)), "out_of_range");
        assert_eq!(reason(window(MAX_AT, MAX_AT + 1)), "out_of_range");
        for limit in [0, MAX_GROUP_LIMIT + 1] {
            let mut q = window(MONDAY, MONDAY + DAY);
            q.limit = Some(limit);
            assert_eq!(reason(q), "out_of_range");
        }
        for tz in [TZ_OFFSET_MINUTES_MIN - 1, TZ_OFFSET_MINUTES_MAX + 1] {
            let mut q = window(MONDAY, MONDAY + DAY);
            q.tz_offset_minutes = Some(tz);
            assert_eq!(reason(q), "out_of_range");
        }
        let mut q = window(MONDAY, MONDAY + DAY);
        q.workdir = Some(PathBuf::from("relative/dir"));
        assert_eq!(reason(q), "out_of_range");
    }

    /// A ledger whose `usage` is an endless view, so every aggregate runs
    /// until it is interrupted.
    fn endless() -> Fixture {
        let f = Fixture::new();
        let c = Connection::open(&f.0).unwrap();
        c.execute_batch(
            "CREATE VIEW usage(id, ts, model_id, role, session_uuid, pwd_hash,
                               tokens_in, tokens_cached, tokens_out, cost) AS
             WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n)
             SELECT x, 100, 'm', 'main', 's', 'p', 1, 0, 1, 0.0 FROM n;",
        )
        .unwrap();
        f
    }

    #[test]
    fn deadline_interrupts_a_running_statement() {
        let f = endless();
        let started = std::time::Instant::now();
        let err = run_at(&f.0, &window(0, 200), 1000, 0, Duration::from_millis(100)).unwrap_err();
        assert_eq!(err, UsageError::Timeout);
        assert_eq!(err.reason(), "query_timeout");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn expired_deadline_refuses_the_next_statement() {
        let f = seeded();
        let conn = Connection::open(&f.0).unwrap();
        let guard = Deadline::start(&conn, Duration::ZERO).unwrap();
        while !guard.expired() {
            std::thread::yield_now();
        }
        let mut ran = false;
        let result = guard.check(|| {
            ran = true;
            Ok(1)
        });
        assert_eq!(result, Err(UsageError::Timeout));
        assert!(!ran);
    }

    #[test]
    fn finished_query_stops_its_timer() {
        let f = seeded();
        let started = std::time::Instant::now();
        let report = run_at(
            &f.0,
            &window(MONDAY, MONDAY + DAY),
            MONDAY,
            0,
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(report.totals.calls, 2);
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn host_offset_is_the_default() {
        let f = seeded();
        let mut q = window(MONDAY, MONDAY + DAY);
        q.bucket = Some(BucketSize::Day);
        let report = run_at(&f.0, &q, MONDAY, 3600, DEADLINE).unwrap();
        assert_eq!(report.buckets.unwrap()[0].start_at, MONDAY - 3600);
        q.tz_offset_minutes = Some(0);
        let report = run_at(&f.0, &q, MONDAY, 3600, DEADLINE).unwrap();
        assert_eq!(report.buckets.unwrap()[0].start_at, MONDAY);
    }
}

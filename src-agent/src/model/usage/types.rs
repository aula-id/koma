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
    pub(crate) fn secs(self) -> i64 {
        match self {
            Self::Hour => 3600,
            Self::Day => 86400,
            Self::Week => 604800,
        }
    }
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
        use super::queries::{
            range_totals, role_split, session_hourly, session_models, session_totals,
            spend_buckets, top_models_in_range,
        };
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

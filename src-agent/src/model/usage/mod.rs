//! Global usage ledger: one sqlite at `~/.koma/usage.sqlite` that persists
//! every model-call's token/cost spend across ALL sessions and working dirs.
//!
//! See the submodules for details: `types` for data structs, `ledger` for
//! write/DB plumbing, `queries` for the dashboard read functions, and `query`
//! for the bounded, deadline-limited usage report.

#![allow(unused_imports)]

mod cache;
mod export;
mod ledger;
mod queries;
pub mod query;
pub(crate) mod reader;
mod types;

// Re-export the entire public surface so every external path is unchanged.
pub use export::{events_after, max_usage_id, TelemetryEvent};
pub use ledger::{record_usage, usage_db_path};
pub use queries::{
    daily_costs, range_totals, range_totals_scoped, role_split, role_split_scoped, session_hourly,
    session_models, session_totals, spend_buckets, spend_buckets_scoped, top_models,
    top_models_in_range, top_models_in_range_scoped, weekly_costs,
};
pub use query::{
    GroupBy, LedgerState, RoleKind, UsageBucket, UsageError, UsageGroup, UsageQuery, UsageReport,
    UsageTotals,
};
pub use types::{
    local_utc_offset_secs, BucketSize, DailyCost, ModelCost, ModelCostRange, RangeTotals,
    RoleSplit, SpendBucket, UsageData, WeeklyCost,
};

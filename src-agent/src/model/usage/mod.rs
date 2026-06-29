//! Global usage ledger: one sqlite at `~/.koma/usage.sqlite` that persists
//! every model-call's token/cost spend across ALL sessions and working dirs.
//!
//! See the submodules for details: `types` for data structs, `ledger` for
//! write/DB plumbing, and `queries` for all read functions.

#![allow(unused_imports)]

mod ledger;
mod queries;
mod types;

// Re-export the entire public surface so every external path is unchanged.
pub use ledger::{record_usage, usage_db_path};
pub use queries::{
    daily_costs, range_totals, role_split, session_hourly, session_models, session_totals,
    spend_buckets, top_models, top_models_in_range, weekly_costs,
};
pub use types::{
    BucketSize, DailyCost, ModelCost, ModelCostRange, RangeTotals, RoleSplit, SpendBucket,
    UsageData, WeeklyCost, local_utc_offset_secs,
};

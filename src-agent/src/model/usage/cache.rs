//! One-second TTL cache around [`UsageData::collect`](super::types::UsageData::collect).
//!
//! The `/usage` dashboard's ledger projection is recomputed from scratch on
//! every call — each query it runs opens its own sqlite connection via
//! `ledger::open()` — but callers hit it far faster than the ledger can
//! usefully change:
//! - the local TUI arm re-collects it every frame (up to ~125Hz during a
//!   stream, see `view::mod::draw`);
//! - the daemon's snapshot projection re-derives it on its ~100ms tick (see
//!   `ipc::snapshot::projection::modes`).
//!
//! This cache keys on the exact query parameters (view + range + bucketing +
//! session uuid) and serves a clone of the last result while it is younger
//! than [`USAGE_CACHE_TTL`]. Unlike the MCP status cache
//! ([`crate::app::mcp::McpManager::server_status_cached`]), there is no
//! off-thread refresh branch here: opening a few sqlite connections inline is
//! cheap enough to absorb on the caller's own thread — the win is only killing
//! the PER-FRAME connection storm, not hiding IO latency.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::types::{BucketSize, UsageData};

/// How long a cached entry is served before being recomputed.
const USAGE_CACHE_TTL: Duration = Duration::from_secs(1);

/// Exact query parameters `/usage` is collected for: `(session_view, since,
/// heat_bucket, heat_n, session_uuid)`. Two calls with the same key within
/// [`USAGE_CACHE_TTL`] reuse the same [`UsageData`].
type CacheKey = (bool, i64, BucketSize, usize, String);

static CACHE: OnceLock<Mutex<Option<(Instant, CacheKey, UsageData)>>> = OnceLock::new();

/// Return a cached [`UsageData`] for `key` if one exists and is younger than
/// [`USAGE_CACHE_TTL`]; otherwise call `compute`, cache the fresh result, and
/// return it.
pub(super) fn get_or_compute(key: CacheKey, compute: impl FnOnce() -> UsageData) -> UsageData {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());

    if let Some((at, cached_key, data)) = guard.as_ref() {
        if *cached_key == key && at.elapsed() < USAGE_CACHE_TTL {
            return data.clone();
        }
    }

    let fresh = compute();
    *guard = Some((Instant::now(), key, fresh.clone()));
    fresh
}

#[cfg(test)]
#[path = "cache_test.rs"]
mod cache_test;

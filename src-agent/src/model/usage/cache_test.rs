#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::cell::Cell;
use std::sync::Mutex;

// `get_or_compute` is the pure seam this whole cache turns on: same key within
// the TTL window must reuse the cached value (never re-invoking `compute`),
// while a different key must always recompute. The wall-clock TTL expiry
// itself isn't exercised here (it would need a real 1s sleep) — the key-match
// branch is the part worth locking down as a fast unit test.
//
// `CACHE` is a single process-wide slot (by design — see the module docs), so
// the two tests below serialize on `TEST_LOCK` to keep cargo's parallel test
// threads from stomping on each other's entry mid-assertion.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn same_key_reuses_cached_value_without_recomputing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let key: CacheKey = (false, 1000, BucketSize::Day, 7, "sess-a".to_string());
    let calls = Cell::new(0);

    let first = get_or_compute(key.clone(), || {
        calls.set(calls.get() + 1);
        UsageData { session_calls: 1, ..Default::default() }
    });
    let second = get_or_compute(key, || {
        calls.set(calls.get() + 1);
        UsageData { session_calls: 2, ..Default::default() }
    });

    assert_eq!(calls.get(), 1, "second call with the same key must not recompute");
    assert_eq!(first.session_calls, second.session_calls, "cached value must be reused as-is");
}

#[test]
fn different_key_always_recomputes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let calls = Cell::new(0);

    let key_a: CacheKey = (false, 1000, BucketSize::Day, 7, "sess-b".to_string());
    let data_a = get_or_compute(key_a, || {
        calls.set(calls.get() + 1);
        UsageData::default()
    });

    let key_b: CacheKey = (true, 2000, BucketSize::Hour, 24, "sess-b".to_string());
    let expected_b = UsageData { session_calls: 42, ..Default::default() };
    let data_b = get_or_compute(key_b, || {
        calls.set(calls.get() + 1);
        UsageData { session_calls: 42, ..Default::default() }
    });

    assert_eq!(calls.get(), 2, "a differing key must trigger a fresh compute");
    assert_eq!(data_a.session_calls, 0);
    assert_eq!(data_b.session_calls, expected_b.session_calls);
}

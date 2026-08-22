#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use reqwest::StatusCode;

#[test]
fn retryable_statuses() {
    // Retryable: server errors + rate limit
    for code in [500, 502, 503, 520, 529, 429] {
        let s = StatusCode::from_u16(code).unwrap();
        assert!(is_retryable_status(s), "expected {code} to be retryable");
    }
    // NOT retryable: success + permanent client errors
    for code in [200, 201, 204, 400, 401, 403, 404, 405, 422] {
        let s = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(s),
            "expected {code} to NOT be retryable"
        );
    }
}

#[test]
fn backoff_delay_is_monotonic_and_bounded() {
    let d1 = backoff_delay(1);
    let d2 = backoff_delay(2);
    let d3 = backoff_delay(3);
    // Each base is larger than the previous.
    assert!(d2 > d1, "d2={d2:?} should be > d1={d1:?}");
    assert!(d3 > d2, "d3={d3:?} should be > d2={d2:?}");
    // Upper bound: base + max jitter.
    assert!(d3 <= std::time::Duration::from_millis(4000 + JITTER_MS));
    // Lower bound: base + 0 jitter.
    assert!(d1 >= std::time::Duration::from_millis(1000));
}

#[test]
fn max_attempts_is_three() {
    assert_eq!(MAX_ATTEMPTS, 3);
}

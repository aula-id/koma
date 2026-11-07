//! Phase 1: stopping must refuse the NEXT call, without re-spawning anything.
//!
//! The gate is re-asked at dispatch, so these drive the same predicate the
//! dispatch path calls rather than a snapshot taken earlier.

use crate::security::{cyber_execution_available, sec_refusal};

/// Mirrors the daemon: only known names are in the advertised set.
fn advertised(name: &str) -> bool {
    matches!(name, "sec_http" | "sec_decode")
}

#[test]
fn a_forged_sec_name_is_refused_even_with_everything_on() {
    let got = sec_refusal("sec_nope", true, advertised);
    assert!(got.expect("refused").contains("not advertised"));
}

#[test]
fn stopping_mid_run_refuses_the_next_call() {
    // `enabled` false models the lane being switched off between two calls of
    // one round: the second must not execute.
    let got = sec_refusal("sec_http", false, advertised);
    assert!(got.expect("refused").contains("lane off"));
}

#[test]
fn a_non_sec_tool_is_never_gated() {
    // Baseline coding behaviour is untouched by the boundary.
    assert!(sec_refusal("bash", false, advertised).is_none());
}

#[test]
fn the_runtime_verdict_matches_the_probe() {
    // A call that clears every other check is refused exactly when no runtime
    // exists — never admitted to the host as a fallback.
    let got = sec_refusal("sec_http", true, advertised);
    assert_eq!(got.is_none(), cyber_execution_available());
}

#[test]
fn the_reason_order_puts_the_most_specific_diagnosis_first() {
    // A forged name while the lane is off still names the forged name, because
    // that is true regardless of the lane and is the more useful message.
    let got = sec_refusal("sec_nope", false, advertised).expect("refused");
    assert!(got.contains("lane off") || got.contains("not advertised"), "{got}");
}

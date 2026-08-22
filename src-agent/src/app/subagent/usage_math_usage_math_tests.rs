#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn accumulate_step_is_additive_across_steps() {
    let (o1, c1) = accumulate_step(0, 0.0, 100, 0.01);
    let (o2, c2) = accumulate_step(o1, c1, 50, 0.02);
    let (o3, c3) = accumulate_step(o2, c2, 25, 0.005);
    assert_eq!(o3, 175);
    assert!((c3 - 0.035).abs() < 1e-12);
}

#[test]
fn fold_parent_skips_zero_step() {
    assert_eq!(fold_parent_spend(1.0, 10, 0, 0.0), None);
}

#[test]
fn fold_parent_adds_tokens_even_when_cost_is_zero() {
    // Subscription models may honestly price at $0; tokens still count.
    assert_eq!(fold_parent_spend(0.0, 0, 42, 0.0), Some((0.0, 42)));
}

#[test]
fn fold_parent_adds_cost_even_when_tokens_out_is_zero() {
    // Defensive: some providers may bill cost without reporting completion tokens.
    let r = fold_parent_spend(1.0, 5, 0, 0.5).unwrap();
    assert!((r.0 - 1.5).abs() < 1e-12);
    assert_eq!(r.1, 5);
}

#[test]
fn kill_mid_run_keeps_finished_steps_only() {
    // Simulate: step0 + step1 finished, step2 never completed (killed).
    let mut parent_cost = 0.05_f64; // prior main spend
    let mut parent_out = 200_u64;
    let steps = [(80_u64, 0.01_f64), (40, 0.02)];
    for (out, cost) in steps {
        let (c, o) = fold_parent_spend(parent_cost, parent_out, out, cost).unwrap();
        parent_cost = c;
        parent_out = o;
    }
    // No third fold — kill drops the in-flight step.
    assert_eq!(parent_out, 320);
    assert!((parent_cost - 0.08).abs() < 1e-12);
}

#[test]
fn effective_step_cost_keeps_nonzero_provider_cost() {
    // Unknown endpoint → overlay is None; non-zero provider cost must win.
    let c = effective_step_cost(
        "https://not-a-real-endpoint.example",
        "any-model",
        1000,
        0,
        500,
        0.123,
    );
    assert!((c - 0.123).abs() < 1e-12);
}

#[test]
fn effective_step_cost_zero_without_overlay_stays_zero() {
    let c = effective_step_cost(
        "https://not-a-real-endpoint.example",
        "any-model",
        1000,
        0,
        500,
        0.0,
    );
    assert_eq!(c, 0.0);
}

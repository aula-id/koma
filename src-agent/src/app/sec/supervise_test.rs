use super::*;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};

fn fixed_clock() -> (Box<dyn Fn() -> Instant + Send + Sync>, Arc<Mutex<Duration>>) {
    let base = Instant::now();
    let offset = Arc::new(Mutex::new(Duration::ZERO));
    let o = offset.clone();
    (
        Box::new(move || base + *o.lock().expect("lock")),
        offset,
    )
}

#[test]
fn the_hypothesis_cap_refuses_a_sixth_open_one() {
    let mut s = Supervisor::new(5, 40, None, None);
    for i in 0..5 {
        s.open_hypothesis(&format!("h{i}")).expect("room");
    }
    assert_eq!(s.open_hypothesis("h6"), Err(Pause::HypothesisCap(5)));
}

#[test]
fn closing_one_frees_a_slot() {
    let mut s = Supervisor::new(1, 40, None, None);
    s.open_hypothesis("a").expect("room");
    assert!(s.open_hypothesis("b").is_err());
    s.close_hypothesis("a");
    assert!(s.open_hypothesis("b").is_ok());
    assert_eq!(s.live_hypotheses(), vec!["b"]);
}

#[test]
fn closing_an_unknown_hypothesis_is_harmless() {
    let mut s = Supervisor::new(2, 10, None, None);
    s.close_hypothesis("nope");
    assert!(s.open_hypothesis("a").is_ok());
}

#[test]
fn the_warning_fires_once_at_the_soft_limit() {
    // Budget 10 puts the warn point (8 left) on turn 2, so a short loop reaches
    // it. The point of the test is "exactly once", not the absolute number.
    let mut s = Supervisor::new(5, 10, None, None);
    let mut warnings = 0;
    for _ in 0..9 {
        if s.turn().is_some() {
            warnings += 1;
        }
    }
    assert_eq!(warnings, 1, "warned {} times", warnings);
}

#[test]
fn the_turn_budget_pauses_the_run() {
    let mut s = Supervisor::new(5, 3, None, None);
    for _ in 0..3 {
        s.turn();
    }
    assert!(s.is_paused());
    assert_eq!(s.gate(), Err(Pause::TurnBudget(3)));
}

#[test]
fn every_pause_has_a_reason() {
    for p in [
        Pause::HypothesisCap(5),
        Pause::TurnBudget(40),
        Pause::Rate(2.0),
        Pause::RequestTotal(10),
    ] {
        assert!(!p.reason().is_empty(), "{p:?}");
    }
}

#[test]
fn the_request_total_pauses_once_spent() {
    let mut s = Supervisor::new(5, 40, None, Some(2));
    assert!(s.gate().is_ok());
    assert!(s.gate().is_ok());
    assert_eq!(s.gate(), Err(Pause::RequestTotal(2)));
    assert!(s.is_paused());
}

#[test]
fn the_rate_cap_refuses_within_the_window_and_clears_after_it() {
    let (clock, offset) = fixed_clock();
    let mut s = Supervisor::new(5, 40, Some(2.0), None).with_clock(clock);
    assert!(s.gate().is_ok());
    assert!(s.gate().is_ok());
    assert_eq!(s.gate(), Err(Pause::Rate(2.0)));
    // A rate refusal is not a pause: it clears on its own.
    assert!(!s.is_paused());
    *offset.lock().expect("lock") += Duration::from_secs(1);
    assert!(s.gate().is_ok());
}

#[test]
fn a_refused_request_does_not_spend_the_total() {
    let (clock, _) = fixed_clock();
    let mut s = Supervisor::new(5, 40, Some(1.0), Some(5)).with_clock(clock);
    assert!(s.gate().is_ok());
    for _ in 0..3 {
        assert!(s.gate().is_err()); // all rate refusals
    }
    assert_eq!(s.sent(), 1);
}

#[test]
fn resuming_grants_turns_rather_than_resetting_the_count() {
    let mut s = Supervisor::new(5, 2, None, None);
    for _ in 0..2 {
        s.turn();
    }
    assert!(s.is_paused());
    s.resume(10);
    assert!(!s.is_paused());
    assert_eq!(s.turns(), 2, "the spent turns were reinstated");
    assert!(s.gate().is_ok());
}

#[test]
fn resuming_does_not_clear_a_request_total_pause() {
    // Only a turn pause is the operator's to lift; the total is a scope fact.
    let mut s = Supervisor::new(5, 40, None, Some(1));
    s.gate().expect("first");
    s.gate().expect_err("spent");
    s.resume(10);
    assert!(s.is_paused());
}

#[test]
fn counters_are_stable_under_a_burst() {
    static _UNUSED: AtomicU64 = AtomicU64::new(0);
    let mut s = Supervisor::new(100, 100, None, Some(1000));
    for _ in 0..1000 {
        s.gate().expect("within total");
    }
    assert_eq!(s.sent(), 1000);
}

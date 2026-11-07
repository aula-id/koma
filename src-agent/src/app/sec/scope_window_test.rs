//! Phase 3: a confirmed, active, in-target scope must still be refused outside
//! its window. A timing limit that can be stepped around is not a limit.

use super::*;

fn scoped(window: Window) -> Scope {
    Scope {
        version: 1,
        state: State::Active,
        auth_mode: Mode::None,
        targets: vec![Host::parse("example.com").expect("valid")],
        exclusions: Vec::new(),
        caps: Caps::default(),
        window,
        confirmed_by: Some("tester".into()),
    }
}

#[test]
fn an_unset_window_admits_at_any_time() {
    let s = scoped(Window::default());
    for now in [0, 1_000_000, i64::MAX] {
        assert!(
            s.admit_at("example.com", 443, Some(1), now).is_ok(),
            "{now}"
        );
    }
}

#[test]
fn before_the_start_is_refused() {
    let s = scoped(Window {
        from: Some(100),
        until: None,
    });
    assert_eq!(
        s.admit_at("example.com", 443, Some(1), 99),
        Err(Refusal::OutsideWindow)
    );
}

#[test]
fn at_and_after_the_start_is_admitted() {
    let s = scoped(Window {
        from: Some(100),
        until: None,
    });
    assert!(s.admit_at("example.com", 443, Some(1), 100).is_ok());
    assert!(s.admit_at("example.com", 443, Some(1), 101).is_ok());
}

#[test]
fn the_end_is_exclusive() {
    // Half-open, so a window ending at 200 permits 199 and not 200. Pinned
    // because an off-by-one here silently extends every run by its last second.
    let s = scoped(Window {
        from: None,
        until: Some(200),
    });
    assert!(s.admit_at("example.com", 443, Some(1), 199).is_ok());
    assert_eq!(
        s.admit_at("example.com", 443, Some(1), 200),
        Err(Refusal::OutsideWindow)
    );
}

#[test]
fn inside_a_closed_range_works() {
    let s = scoped(Window {
        from: Some(100),
        until: Some(200),
    });
    assert!(s.admit_at("example.com", 443, Some(1), 150).is_ok());
}

#[test]
fn the_window_refusal_wins_over_a_host_diagnosis() {
    // Outside the window every target is refused, so reporting "not in scope"
    // for an out-of-target host would blame the wrong thing.
    let s = scoped(Window {
        from: Some(100),
        until: Some(200),
    });
    assert_eq!(
        s.admit_at("elsewhere.test", 443, Some(1), 300),
        Err(Refusal::OutsideWindow)
    );
}

#[test]
fn the_window_is_checked_after_activation_and_confirmation() {
    // A closed window on a paused scope should read as "not active", not as
    // a window problem — the earlier condition is the real one.
    let mut s = scoped(Window {
        from: Some(100),
        until: Some(200),
    });
    s.state = State::Paused;
    assert_eq!(
        s.admit_at("example.com", 443, Some(1), 300),
        Err(Refusal::NotActive)
    );
}

#[test]
fn an_empty_window_is_not_worth_checking() {
    assert!(!Window::default().is_set());
    assert!(Window {
        from: Some(1),
        until: None
    }
    .is_set());
}

#[test]
fn the_reason_is_specific() {
    assert!(Refusal::OutsideWindow.reason().contains("window"));
}

#[test]
fn a_window_survives_a_round_trip() {
    let s = scoped(Window {
        from: Some(1),
        until: Some(2),
    });
    let json = serde_json::to_string(&s).expect("serialise");
    let back: Scope = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back.window, s.window);
}

#[test]
fn an_older_scope_document_without_a_window_still_loads() {
    // Every scope written before this field must keep working, and an absent
    // window means "no timing limit".
    let s = scoped(Window::default());
    let mut v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&s).expect("ser")).expect("parse");
    v.as_object_mut().expect("object").remove("window");
    let back: Scope = serde_json::from_value(v).expect("load");
    assert!(!back.window.is_set());
}

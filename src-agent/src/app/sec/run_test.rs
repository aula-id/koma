use super::*;
use crate::app::sec::scope;

fn scope(v: u64) -> scope::Scope {
    scope::Scope {
        version: v,
        state: scope::State::Active,
        auth_mode: scope::Mode::None,
        targets: vec![scope::Host::parse("a.example").expect("valid")],
        exclusions: Vec::new(),
        caps: scope::Caps::default(),
        window: scope::Window::default(),
        confirmed_by: Some("tester".into()),
    }
}

fn run(lane: Lane, v: u64) -> RunContext {
    RunContext {
        id: "run-x".into(),
        session: "s".into(),
        cwd: PathBuf::from("/w"),
        lane,
        scope_version: v,
        role_model: "m".into(),
    }
}

#[test]
fn a_full_context_starts() {
    let s = scope(1);
    let got = RunContext::start(
        true, "s", std::path::Path::new("/w"), Some(Lane::Red), Some("m"), Some(&s), true,
    );
    assert!(got.is_ok(), "{got:?}");
}

#[test]
fn each_precondition_refuses_with_its_own_reason() {
    let s = scope(1);
    let p = std::path::Path::new("/w");
    let cases = [
        (false, Some(Lane::Red), Some("m"), Some(&s), true, StartRefusal::Disabled),
        (true, None, Some("m"), Some(&s), true, StartRefusal::NoRole),
        (true, Some(Lane::Red), None, Some(&s), true, StartRefusal::NoModel),
        (true, Some(Lane::Red), Some(""), Some(&s), true, StartRefusal::NoModel),
        (true, Some(Lane::Red), Some("m"), None, true, StartRefusal::NoScope),
        (true, Some(Lane::Red), Some("m"), Some(&s), false, StartRefusal::NoBoundary),
    ];
    for (en, lane, model, sc, ok, want) in cases {
        let got = RunContext::start(en, "s", p, lane, model, sc, ok);
        assert_eq!(got.err(), Some(want), "{want:?}");
        assert!(!want.reason().is_empty());
    }
}

#[test]
fn an_inactive_or_unconfirmed_scope_refuses() {
    let p = std::path::Path::new("/w");
    let mut inactive = scope(1);
    inactive.state = scope::State::Paused;
    assert_eq!(
        RunContext::start(true, "s", p, Some(Lane::Red), Some("m"), Some(&inactive), true).err(),
        Some(StartRefusal::NoScope)
    );
    let mut unconfirmed = scope(1);
    unconfirmed.confirmed_by = None;
    assert_eq!(
        RunContext::start(true, "s", p, Some(Lane::Red), Some("m"), Some(&unconfirmed), true).err(),
        Some(StartRefusal::NoScope)
    );
}

#[test]
fn an_edit_makes_the_run_stale_and_stops_the_next_call() {
    let r = run(Lane::Red, 1);
    let edited = scope(2);
    assert!(r.is_stale(&edited));
    let err = r.permit("sec_http", Some(&edited)).expect_err("stale");
    assert!(err.contains("edited"), "{err}");
}

#[test]
fn an_unchanged_version_still_permits() {
    let r = run(Lane::Red, 1);
    assert_eq!(r.permit("sec_http", Some(&scope(1))), Ok(()));
}

#[test]
fn the_lane_gate_refuses_a_network_tool_in_research() {
    let r = run(Lane::Research, 1);
    let err = r.permit("sec_http", None).expect_err("refused");
    assert!(err.contains("research"), "{err}");
}

#[test]
fn the_lane_gate_allows_a_compute_tool_in_research() {
    assert_eq!(run(Lane::Research, 1).permit("sec_decode", None), Ok(()));
}

#[test]
fn run_ids_differ() {
    assert_ne!(new_run_id(), new_run_id());
}

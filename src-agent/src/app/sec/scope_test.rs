use super::*;

fn scope() -> Scope {
    Scope {
        version: 1,
        state: State::Active,
        auth_mode: Mode::None,
        targets: vec![Host::parse("example.com").expect("valid")],
        exclusions: Vec::new(),
        caps: Caps::default(),
        window: Window::default(),
        confirmed_by: Some("alice".into()),
    }
}

#[test]
fn a_bare_host_is_admitted_on_any_port() {
    assert_eq!(scope().admit("example.com", 443, None), Ok(()));
    assert_eq!(scope().admit("example.com", 8080, None), Ok(()));
}

#[test]
fn subdomains_are_covered_but_a_lookalike_is_not() {
    assert_eq!(scope().admit("api.example.com", 443, None), Ok(()));
    assert!(scope().admit("notexample.com", 443, None).is_err());
}

#[test]
fn a_leading_dot_means_subdomains_only() {
    let mut s = scope();
    s.targets = vec![Host::parse(".example.com").expect("valid")];
    assert!(s.admit("api.example.com", 443, None).is_ok());
    assert!(s.admit("example.com", 443, None).is_err());
}

#[test]
fn ip_cells_never_suffix_match() {
    let mut s = scope();
    s.targets = vec![Host::parse("10.0.0.1").expect("valid")];
    assert!(s.admit("10.0.0.1", 80, None).is_ok());
    assert!(s.admit("20.10.0.1", 80, None).is_err());
}

#[test]
fn a_bare_ipv6_literal_is_not_a_port() {
    assert_eq!(split_host_port("fe80::1"), Some(("fe80::1".into(), None)));
    assert_eq!(
        split_host_port("[fe80::1]:8080"),
        Some(("fe80::1".into(), Some(8080)))
    );
    assert_eq!(
        split_host_port("localhost:8080"),
        Some(("localhost".into(), Some(8080)))
    );
}

#[test]
fn an_exclusion_beats_a_broader_target() {
    let mut s = scope();
    s.exclusions = vec![Host::parse("api.example.com").expect("valid")];
    assert_eq!(
        s.admit("api.example.com", 443, None),
        Err(Refusal::Excluded)
    );
    assert_eq!(s.admit("example.com", 443, None), Ok(()));
}

#[test]
fn a_port_narrowed_host_reports_the_port_not_the_host() {
    let mut s = scope();
    s.targets = vec![Host::parse("example.com:8080").expect("valid")];
    assert_eq!(
        s.admit("example.com", 80, None),
        Err(Refusal::PortNotAllowed)
    );
    assert_eq!(s.admit("example.com", 8080, None), Ok(()));
}

#[test]
fn exclusion_wins_even_on_a_port_the_target_allows() {
    let mut s = scope();
    s.targets = vec![Host::parse("example.com").expect("valid")];
    s.exclusions = vec![Host::parse("example.com:8443").expect("valid")];
    assert_eq!(s.admit("example.com", 8443, None), Err(Refusal::Excluded));
    assert_eq!(s.admit("example.com", 443, None), Ok(()));
}

#[test]
fn an_inactive_unconfirmed_or_absent_scope_admits_nothing() {
    let mut s = scope();
    s.state = State::Paused;
    assert_eq!(s.admit("example.com", 443, None), Err(Refusal::NotActive));
    let mut s = scope();
    s.confirmed_by = None;
    assert_eq!(
        s.admit("example.com", 443, None),
        Err(Refusal::NotConfirmed)
    );
}

#[test]
fn a_stale_version_is_refused_rather_than_upgraded() {
    let s = scope();
    assert_eq!(
        s.admit("example.com", 443, Some(2)),
        Err(Refusal::NotActive)
    );
    assert_eq!(s.admit("example.com", 443, Some(1)), Ok(()));
}

#[test]
fn a_redirect_hop_is_re_admitted() {
    let mut s = scope();
    s.exclusions = vec![Host::parse("evil.example").expect("valid")];
    assert_eq!(
        s.admit_after_hop("evil.example", 443),
        Err(Refusal::Excluded)
    );
    assert_eq!(
        s.admit_after_hop("other.example", 443),
        Err(Refusal::OutOfScope)
    );
}

#[test]
fn host_normalisation_folds_case_the_trailing_dot_and_ipv6() {
    assert_eq!(normalise("EXAMPLE.com."), Some("example.com".into()));
    assert_eq!(normalise("[::1]"), Some("::1".into()));
    assert_eq!(normalise("0:0:0:0:0:0:0:1"), normalise("::1"));
}

#[test]
fn a_url_endpoint_defaults_its_port_from_the_scheme_not_the_host() {
    let e = scope().endpoint("https://example.com/x").expect("endpoint");
    assert_eq!((e.port, e.transport), (443, Transport::Https));
    let e = scope().endpoint("example.com").expect("endpoint");
    assert_eq!((e.port, e.transport), (443, Transport::Https));
}

#[test]
fn a_defaulted_port_still_has_to_be_admitted() {
    let mut s = scope();
    s.targets = vec![Host::parse("example.com:8080").expect("valid")];
    let e = s.endpoint("https://example.com").expect("endpoint");
    assert_eq!(s.admit(&e.host, e.port, None), Err(Refusal::PortNotAllowed));
}

#[test]
fn an_unusable_host_parses_to_none() {
    assert!(Host::parse("").is_none());
    assert!(Host::parse("   ").is_none());
    assert!(Host::parse("bad host!").is_none());
}

#[test]
fn every_refusal_has_a_reason() {
    for r in [
        Refusal::NoScope,
        Refusal::NotActive,
        Refusal::NotConfirmed,
        Refusal::OutOfScope,
        Refusal::Excluded,
        Refusal::PortNotAllowed,
        Refusal::NoDestination,
    ] {
        assert!(!r.reason().is_empty());
    }
}

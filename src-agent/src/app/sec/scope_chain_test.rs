use super::*;

fn scoped() -> Scope {
    Scope {
        version: 1,
        state: State::Active,
        auth_mode: Mode::None,
        targets: vec![Host::parse("a.example").expect("valid")],
        exclusions: Vec::new(),
        caps: Caps::default(),
        window: Window::default(),
        confirmed_by: Some("t".into()),
    }
}

#[test]
fn a_chain_inside_scope_is_admitted() {
    assert_eq!(
        scoped().admit_chain(["https://a.example/1", "https://a.example/2"]),
        Ok(())
    );
}

#[test]
fn the_hop_that_leaves_scope_is_the_one_named() {
    let got = scoped().admit_chain(["https://a.example/1", "https://b.example/2"]);
    let (hop, refusal) = got.expect_err("refused");
    assert_eq!(hop, "https://b.example/2");
    assert_eq!(refusal, Refusal::OutOfScope);
}

#[test]
fn an_unparseable_hop_refuses_rather_than_skipping() {
    let (_, refusal) = scoped().admit_chain([""]).expect_err("refused");
    assert_eq!(refusal, Refusal::Unparseable);
}

#[test]
fn the_first_refusal_wins() {
    let (hop, _) = scoped()
        .admit_chain(["https://b.example/", "https://c.example/"])
        .expect_err("refused");
    assert_eq!(hop, "https://b.example/");
}

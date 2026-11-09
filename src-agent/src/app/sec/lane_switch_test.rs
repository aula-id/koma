use super::*;

#[test]
fn request_overrides_until_the_next_context_after_it_is_saved() {
    let id = "lane-switch-test-session";
    assert_eq!(override_for(id), None);

    request(id, Some(Lane::Red));
    assert_eq!(override_for(id), Some(Some(Lane::Red)));

    // Not yet persisted: a new context must keep the override.
    clear_applied(id);
    assert_eq!(override_for(id), Some(Some(Lane::Red)));

    let drained = drain_unapplied();
    assert!(drained.contains(&(id.to_string(), Some(Lane::Red))));
    // Drained once only.
    assert!(!drain_unapplied().iter().any(|(sid, _)| sid == id));
    // Still in force for the rest of the turn.
    assert_eq!(override_for(id), Some(Some(Lane::Red)));

    clear_applied(id);
    assert_eq!(override_for(id), None);
}

#[test]
fn parse_accepts_lanes_and_none_and_rejects_others() {
    assert_eq!(parse("Research"), Ok(Some(Lane::Research)));
    assert_eq!(parse(" red "), Ok(Some(Lane::Red)));
    assert_eq!(parse("blue"), Ok(Some(Lane::Blue)));
    assert_eq!(parse("none"), Ok(None));
    assert!(parse("purple").unwrap_err().contains("research, red, blue or none"));
}

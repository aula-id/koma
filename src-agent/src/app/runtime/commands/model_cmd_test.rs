#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn parse_role_main() {
    assert_eq!(parse_role("main"), Some(ModelRole::Main));
    assert_eq!(parse_role("Main"), Some(ModelRole::Main));
    assert_eq!(parse_role("MAIN"), Some(ModelRole::Main));
}

#[test]
fn parse_role_awareness() {
    assert_eq!(parse_role("awareness"), Some(ModelRole::Awareness));
    assert_eq!(parse_role("Awareness"), Some(ModelRole::Awareness));
}

#[test]
fn parse_role_planner() {
    assert_eq!(parse_role("planner"), Some(ModelRole::Planner));
}

#[test]
fn parse_role_compactor() {
    assert_eq!(parse_role("compactor"), Some(ModelRole::Compactor));
}

#[test]
fn parse_role_safeguard() {
    assert_eq!(parse_role("safeguard"), Some(ModelRole::Safeguard));
}

#[test]
fn parse_role_unknown() {
    assert_eq!(parse_role("unknown"), None);
    assert_eq!(parse_role(""), None);
    assert_eq!(parse_role("mainn"), None);
    assert_eq!(parse_role(" main"), None);
}

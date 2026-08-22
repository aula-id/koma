#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn sanitize_rejects_bad_names() {
    assert!(sanitize_branch_name("").is_err());
    assert!(sanitize_branch_name("   ").is_err());
    assert!(sanitize_branch_name("-bad").is_err());
    assert!(sanitize_branch_name("a..b").is_err());
    assert!(sanitize_branch_name("has space").is_err());
    assert!(sanitize_branch_name("feat/ok-name").is_ok());
    assert_eq!(sanitize_branch_name("  fix/bug-1  ").unwrap(), "fix/bug-1");
}

#[test]
fn classify_picks_prefix_from_goal() {
    let b = classify_mission_branch("fix crash on startup", "standard", &[]);
    assert!(b.starts_with("fix/"), "{b}");
    let b = classify_mission_branch("add new feature for login", "full", &[]);
    assert!(b.starts_with("feat/"), "{b}");
    let b = classify_mission_branch("chore deps bump", "express", &[]);
    assert!(b.starts_with("chore/"), "{b}");
    let b = classify_mission_branch("update readme docs", "standard", &[]);
    assert!(b.starts_with("docs/"), "{b}");
    let b = classify_mission_branch("something vague", "standard", &[]);
    assert!(b.starts_with("feat/"), "{b}");
}

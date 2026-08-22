#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn slug_basic() {
    assert_eq!(
        slug_from_path("/home/user/my-project"),
        "home-user-my-project"
    );
    // Drive letter stripped (pi projectSlugFromPath).
    assert_eq!(slug_from_path("C:\\Users\\me\\code"), "users-me-code");
    assert_eq!(slug_from_path("/"), "project");
    assert_eq!(slug_from_path(""), "project");
}

#[test]
fn today_is_reasonable() {
    let d = today_date_string();
    // Must be YYYY-MM-DD format and a reasonable year.
    assert_eq!(d.len(), 10);
    assert_eq!(&d[4..5], "-");
    assert_eq!(&d[7..8], "-");
    let year: i32 = d[..4].parse().unwrap();
    assert!((2024..=2100).contains(&year));
}

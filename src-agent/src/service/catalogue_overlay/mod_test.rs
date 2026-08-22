#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn bundled_default_parses() {
    let table: OverlayTable =
        serde_json::from_str(BUNDLED_DEFAULT).expect("bundled models.json must parse");
    assert!(
        !table.is_empty(),
        "bundled table should have at least one endpoint"
    );
}

#[test]
fn lookup_unknown_returns_none() {
    let _ = OVERLAY.set(RwLock::new(load_initial()));
    assert!(lookup("https://not-a-real-endpoint", "nope").is_none());
}

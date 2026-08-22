#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::json;

#[test]
fn menu_entries_union_and_skips_blank_ids() {
    let screen = json!({
        "title": "Home",
        "body": [
            { "t": "text", "text": "pick one" },
            { "t": "menu", "items": [
                { "id": "a", "label": "Alpha" },
                { "id": "", "label": "skipme" }
            ]},
            { "t": "divider" },
            { "t": "menu", "items": [ { "id": "b", "label": "Beta" } ] }
        ]
    });
    let mut st = ExtScreenState::new("x".into(), "s".into(), "Home".into());
    st.screen = Some(screen);
    let entries = st.menu_entries();
    assert_eq!(
        entries,
        vec![
            ("a".to_string(), "Alpha".to_string()),
            ("b".to_string(), "Beta".to_string()),
        ]
    );
    // Cursor clamps + selects across the union.
    st.menu_cursor = 5;
    st.clamp_menu();
    assert_eq!(st.menu_cursor, 1);
    assert_eq!(st.selected_menu_item().as_deref(), Some("b"));
}

#[test]
fn no_menu_is_empty_and_selects_nothing() {
    let st = ExtScreenState::new("x".into(), "s".into(), "t".into());
    assert!(st.menu_entries().is_empty());
    assert!(st.selected_menu_item().is_none());
}

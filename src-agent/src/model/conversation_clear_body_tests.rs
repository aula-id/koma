#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn clear_body_keeps_system_drops_rest() {
    let mut c = Conversation::new("sys");
    c.push_user("hi");
    c.push_assistant("hello", None, false);
    c.clear_body();
    assert_eq!(c.messages().len(), 1);
    assert_eq!(c.messages()[0].role, Role::System);
    assert_eq!(c.messages()[0].content, "sys");
}

#[test]
fn clear_body_empties_when_no_system() {
    let mut c = Conversation::from_messages(vec![]);
    c.push_user("hi");
    c.push_assistant("hello", None, false);
    c.clear_body();
    assert!(c.messages().is_empty());
}

#[test]
fn clear_body_on_empty_is_noop() {
    let mut c = Conversation::from_messages(vec![]);
    c.clear_body();
    assert!(c.messages().is_empty());
}

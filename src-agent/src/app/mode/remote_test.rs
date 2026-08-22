use super::*;

fn make_test_host(id: &str, name: &str) -> crate::remote::hosts::RemoteHost {
    crate::remote::hosts::RemoteHost {
        id: id.into(),
        name: name.into(),
        user: "root".into(),
        host: "10.0.0.1".into(),
        port: 22,
        key_path: None,
        last_connected: None,
        tags: vec![],
    }
}

#[test]
fn manage_intent_starts_at_browse() {
    let hosts = vec![make_test_host("h1", "srv")];
    let state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
    assert_eq!(state.intent, RemoteIntent::Manage);
    assert_eq!(state.view, RemoteView::Browse);
}

#[test]
fn resume_intent_starts_at_browse() {
    let state = RemoteState::for_intent(vec![], RemoteIntent::Resume);
    assert_eq!(state.intent, RemoteIntent::Resume);
    assert_eq!(state.view, RemoteView::Browse);
}

#[test]
fn new_intent_starts_at_browse() {
    let state = RemoteState::for_intent(vec![], RemoteIntent::New);
    assert_eq!(state.intent, RemoteIntent::New);
    assert_eq!(state.view, RemoteView::Browse);
}

#[test]
fn new_alias_starts_at_browse() {
    let state = RemoteState::new(vec![]);
    assert_eq!(state.intent, RemoteIntent::Manage);
    assert_eq!(state.view, RemoteView::Browse);
}

#[test]
fn enter_create_transitions_to_edit() {
    let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
    state.enter_create();
    assert_eq!(state.view, RemoteView::Edit);
    assert!(state.editor.is_some());
    assert!(!state.editing_field);
}

#[test]
fn enter_edit_transitions_to_edit() {
    let hosts = vec![make_test_host("h1", "srv")];
    let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
    state.enter_edit();
    assert_eq!(state.view, RemoteView::Edit);
    assert!(state.editor.is_some());
    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.edit_id.as_deref(), Some("h1"));
    assert_eq!(editor.name, "srv");
}

#[test]
fn cancel_edit_returns_to_browse() {
    let hosts = vec![make_test_host("h1", "srv")];
    let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
    state.enter_edit();
    assert_eq!(state.view, RemoteView::Edit);
    state.cancel_edit();
    assert_eq!(state.view, RemoteView::Browse);
    assert!(state.editor.is_none());
}

#[test]
fn validate_editor_rejects_empty_fields() {
    let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
    state.enter_create();
    assert!(!state.validate_editor());
    assert!(state.editor.as_ref().unwrap().error.is_some());
}

#[test]
fn validate_editor_rejects_invalid_port() {
    let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
    state.enter_create();
    if let Some(ref mut editor) = state.editor {
        editor.name = "test".into();
        editor.user = "root".into();
        editor.host = "10.0.0.1".into();
        editor.port = "not_a_number".into();
    }
    assert!(!state.validate_editor());
    assert_eq!(
        state.editor.as_ref().unwrap().error.as_deref(),
        Some("port must be a number")
    );
}

#[test]
fn validate_editor_accepts_valid_fields() {
    let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
    state.enter_create();
    if let Some(ref mut editor) = state.editor {
        editor.name = "test".into();
        editor.user = "root".into();
        editor.host = "10.0.0.1".into();
        editor.port = "22".into();
    }
    assert!(state.validate_editor());
    assert!(state.editor.as_ref().unwrap().error.is_none());
}

#[test]
fn build_host_from_editor() {
    let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
    state.enter_create();
    if let Some(ref mut editor) = state.editor {
        editor.name = "prod".into();
        editor.user = "deploy".into();
        editor.host = "example.com".into();
        editor.port = "2222".into();
        editor.key_path = "/tmp/key".into();
    }
    let host = state.build_host().expect("build_host should succeed");
    assert_eq!(host.name, "prod");
    assert_eq!(host.user, "deploy");
    assert_eq!(host.host, "example.com");
    assert_eq!(host.port, 2222);
    assert_eq!(host.key_path.as_deref(), Some("/tmp/key"));
}

#[test]
fn build_host_empty_key_path_is_none() {
    let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
    state.enter_create();
    if let Some(ref mut editor) = state.editor {
        editor.name = "test".into();
        editor.user = "root".into();
        editor.host = "10.0.0.1".into();
        editor.port = "22".into();
        editor.key_path = "  ".into(); // whitespace only
    }
    let host = state.build_host().expect("build_host should succeed");
    assert!(host.key_path.is_none());
}

#[test]
fn move_up_down_clamps() {
    let hosts = vec![make_test_host("h1", "a"), make_test_host("h2", "b")];
    let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
    // Starts at 0.
    assert_eq!(state.selected, 0);
    state.move_up(); // Already at top, no-op.
    assert_eq!(state.selected, 0);
    state.move_down();
    assert_eq!(state.selected, 1);
    state.move_down(); // Already at bottom, no-op.
    assert_eq!(state.selected, 1);
    state.move_up();
    assert_eq!(state.selected, 0);
}

#[test]
fn selected_host_returns_correct_host() {
    let hosts = vec![
        make_test_host("h1", "first"),
        make_test_host("h2", "second"),
    ];
    let state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
    assert_eq!(
        state.selected_host().map(|h| h.name.as_str()),
        Some("first")
    );
}

#[test]
fn host_edit_field_cycle() {
    assert_eq!(HostEditField::Name.next(), HostEditField::User);
    assert_eq!(HostEditField::User.next(), HostEditField::Host);
    assert_eq!(HostEditField::Host.next(), HostEditField::Port);
    assert_eq!(HostEditField::Port.next(), HostEditField::KeyPath);
    assert_eq!(HostEditField::KeyPath.next(), HostEditField::Name);

    assert_eq!(HostEditField::Name.prev(), HostEditField::KeyPath);
    assert_eq!(HostEditField::User.prev(), HostEditField::Name);
    assert_eq!(HostEditField::Host.prev(), HostEditField::User);
    assert_eq!(HostEditField::Port.prev(), HostEditField::Host);
    assert_eq!(HostEditField::KeyPath.prev(), HostEditField::Port);
}

#[test]
fn connection_state_transitions_covered() {
    // Verify all ConnectionState variants are present and distinct.
    let states = [
        ConnectionState::Disconnected,
        ConnectionState::Resolving,
        ConnectionState::Authenticating,
        ConnectionState::AuthRequired {
            host_id: "x".into(),
            user: "u".into(),
            host: "h".into(),
        },
        ConnectionState::Bootstrapping,
        ConnectionState::Connecting,
        ConnectionState::Connected {
            session_id: "s".into(),
        },
        ConnectionState::Error {
            message: "e".into(),
        },
    ];
    // All distinct (clone + eq check).
    for (i, a) in states.iter().enumerate() {
        for (j, b) in states.iter().enumerate() {
            if i != j {
                assert_ne!(a, b);
            }
        }
    }
}

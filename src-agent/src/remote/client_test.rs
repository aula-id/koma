use super::session_id_for;

#[test]
fn existing_remote_session_id_is_preserved() {
    assert_eq!(session_id_for(Some("remote-session")), "remote-session");
}

#[test]
fn new_remote_session_gets_a_uuid() {
    let id = session_id_for(None);
    assert!(!id.is_empty());
    assert!(uuid::Uuid::parse_str(&id).is_ok());
}

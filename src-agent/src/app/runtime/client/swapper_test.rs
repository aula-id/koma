use super::*;

fn remote_source() -> DiscoverySource {
    DiscoverySource::Remote {
        target: crate::remote::RemoteTarget {
            user: "alice".to_string(),
            host: "example.test".to_string(),
            port: None,
            key: None,
        },
        password: None,
    }
}

#[test]
fn remote_snapshot_tags_rows_and_preserves_foreground_session() {
    let current_id = "remote-current";
    let mut hub = hub_from_remote_snapshot(Vec::new(), "alice@example.test", Some(current_id));
    let source = remote_source();
    apply_snapshot(
        &mut hub,
        vec![
            SessionStatus {
                session_id: current_id.to_string(),
                name: "Current".to_string(),
                pwd: String::new(),
                working: true,
            },
            SessionStatus {
                session_id: "remote-other".to_string(),
                name: "Other".to_string(),
                pwd: String::new(),
                working: false,
            },
        ],
        Some(current_id),
        &source,
    );

    assert!(hub.history.is_empty());
    assert!(hub.history_filtered.is_empty());
    assert_eq!(hub.cooking.len(), 3);
    assert!(hub
        .cooking
        .iter()
        .all(|row| { row.remote_host.as_deref() == Some("alice@example.test") }));
    assert!(hub.cooking[0].session_id.is_none());
    assert!(!hub.cooking[0].is_foreground);
    assert_eq!(
        hub.cooking
            .iter()
            .find(|row| row.session_id.as_deref() == Some(current_id))
            .map(|row| row.is_foreground),
        Some(true)
    );
}

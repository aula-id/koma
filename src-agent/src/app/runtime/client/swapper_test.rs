use super::*;
use crate::remote::sessions::{DiscoveredHistorySession, DiscoveredSession, DiscoveredSessions};

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
    let mut hub = hub_from_remote_discovery(
        DiscoveredSessions::default(),
        "alice@example.test",
        Some(current_id),
    );
    let source = remote_source();
    apply_snapshot(
        &mut hub,
        ProbeSnap::Remote(DiscoveredSessions {
            live: vec![
                DiscoveredSession {
                    session_id: current_id.to_string(),
                    name: "Current".to_string(),
                    pwd: "/home/alice/proj".to_string(),
                    working: true,
                    is_foreground: false,
                },
                DiscoveredSession {
                    session_id: "remote-other".to_string(),
                    name: "Other".to_string(),
                    pwd: "/tmp".to_string(),
                    working: false,
                    is_foreground: false,
                },
            ],
            history: vec![DiscoveredHistorySession {
                session_id: "hist-1".to_string(),
                name: "Past".to_string(),
                pwd: "/home/alice/old".to_string(),
                updated_at: 1_700_000_000,
                dir_label: "old".to_string(),
            }],
        }),
        Some(current_id),
        &source,
    );

    assert_eq!(hub.history.len(), 1);
    assert_eq!(hub.history_filtered.len(), 1);
    assert_eq!(hub.history[0].name, "Past");
    assert_eq!(hub.history[0].dir_label, "old");
    assert_eq!(
        hub.history[0].remote_host.as_deref(),
        Some("alice@example.test")
    );
    assert_eq!(
        hub.history[0]
            .path
            .file_name()
            .and_then(|n| n.to_str()),
        Some("hist-1")
    );
    // Live ids must not also appear in history even if the payload double-listed them.
    assert!(!hub
        .history
        .iter()
        .any(|h| h.path.file_name().and_then(|n| n.to_str()) == Some(current_id)));

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
    assert_eq!(
        hub.cooking
            .iter()
            .find(|row| row.session_id.as_deref() == Some(current_id))
            .map(|row| row.dir_label.as_str()),
        Some("proj")
    );
}

#[test]
fn remote_history_dedups_live_ids() {
    let hub = hub_from_remote_discovery(
        DiscoveredSessions {
            live: vec![DiscoveredSession {
                session_id: "same-id".to_string(),
                name: "Live".to_string(),
                pwd: "/x".to_string(),
                working: false,
                is_foreground: false,
            }],
            history: vec![DiscoveredHistorySession {
                session_id: "same-id".to_string(),
                name: "Also".to_string(),
                pwd: "/x".to_string(),
                updated_at: 1,
                dir_label: "x".to_string(),
            }],
        },
        "alice@example.test",
        None,
    );
    assert!(hub.history.is_empty());
    assert_eq!(hub.cooking.len(), 2); // new + live
}

#[test]
fn remote_history_row_carries_host_and_uuid_filename() {
    // resolve_enter reads path.file_name() + remote_host — this is the contract
    // the history Enter path depends on (remote resume over SSH, no laptop stub).
    let hub = hub_from_remote_discovery(
        DiscoveredSessions {
            live: Vec::new(),
            history: vec![DiscoveredHistorySession {
                session_id: "dead-uuid".to_string(),
                name: "Dead".to_string(),
                pwd: "/proj".to_string(),
                updated_at: 42,
                dir_label: "proj".to_string(),
            }],
        },
        "alice@example.test",
        None,
    );
    assert_eq!(hub.history.len(), 1);
    let row = &hub.history[0];
    assert_eq!(
        row.path.file_name().and_then(|n| n.to_str()),
        Some("dead-uuid")
    );
    assert_eq!(row.remote_host.as_deref(), Some("alice@example.test"));
    assert!(!row
        .path
        .starts_with(std::path::Path::new("/"))); // synthetic, not a real laptop path root
}

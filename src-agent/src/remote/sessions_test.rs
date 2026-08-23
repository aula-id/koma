use super::*;

#[test]
fn kill_session_args_shape() {
    let args = kill_session_args("abc-123").unwrap();
    assert_eq!(
        args,
        vec![
            "daemon".to_string(),
            "kill".to_string(),
            "--session".to_string(),
            "abc-123".to_string(),
        ]
    );
}

#[test]
fn kill_session_args_reject_empty() {
    assert!(kill_session_args("").is_err());
    assert!(kill_session_args("a\0b").is_err());
}

#[test]
fn delete_session_args_shape() {
    let args = delete_session_args("dead-sess").unwrap();
    assert_eq!(
        args,
        vec![
            "daemon".to_string(),
            "delete".to_string(),
            "--session".to_string(),
            "dead-sess".to_string(),
        ]
    );
}

#[test]
fn delete_session_args_reject_empty() {
    assert!(delete_session_args("").is_err());
    assert!(delete_session_args("a\0b").is_err());
}

#[test]
fn parse_sessions_json_object_form() {
    let raw = r#"{
        "live": [
            {
                "session_id": "live-1",
                "name": "Live",
                "pwd": "/tmp/proj",
                "working": true,
                "is_foreground": false
            }
        ],
        "history": [
            {
                "session_id": "hist-1",
                "name": "Old",
                "pwd": "/tmp/old",
                "updated_at": 1700000000,
                "dir_label": "old"
            }
        ]
    }"#;
    let parsed = parse_sessions_json(raw).unwrap();
    assert_eq!(parsed.live.len(), 1);
    assert_eq!(parsed.live[0].session_id, "live-1");
    assert_eq!(parsed.live[0].pwd, "/tmp/proj");
    assert!(parsed.live[0].working);
    assert_eq!(parsed.history.len(), 1);
    assert_eq!(parsed.history[0].session_id, "hist-1");
    assert_eq!(parsed.history[0].updated_at, 1_700_000_000);
    assert_eq!(parsed.history[0].dir_label, "old");
}

#[test]
fn parse_sessions_json_legacy_array_fallback() {
    let raw = r#"[
        {
            "session_id": "only-live",
            "name": "Live",
            "pwd": "/home/u",
            "working": false,
            "is_foreground": false
        }
    ]"#;
    let parsed = parse_sessions_json(raw).unwrap();
    assert_eq!(parsed.live.len(), 1);
    assert_eq!(parsed.live[0].session_id, "only-live");
    assert!(parsed.history.is_empty());
}

#[test]
fn parse_sessions_json_empty() {
    let parsed = parse_sessions_json("").unwrap();
    assert!(parsed.live.is_empty());
    assert!(parsed.history.is_empty());
    let parsed = parse_sessions_json("   \n").unwrap();
    assert!(parsed.live.is_empty());
    assert!(parsed.history.is_empty());
}

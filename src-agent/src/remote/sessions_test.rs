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

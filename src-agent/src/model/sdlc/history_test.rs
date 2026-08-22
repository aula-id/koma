#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn make_call(name: &str, args: &str) -> crate::dto::chat::ToolCall {
    crate::dto::chat::ToolCall {
        id: "call-1".into(),
        kind: "function".into(),
        function: crate::dto::chat::FunctionCall {
            name: name.into(),
            arguments: args.into(),
        },
    }
}

#[test]
fn extract_write_success() {
    let call = make_call("write", r#"{"path":"src/foo.rs","content":"x"}"#);
    let candidate = extract_edit_audit(&call, "ok", Some("t1"), "batch-1").unwrap();
    assert_eq!(candidate.tool, "write");
    assert_eq!(candidate.path, "src/foo.rs");
    assert_eq!(candidate.node_id.as_deref(), Some("t1"));
    assert_eq!(candidate.batch_id, "batch-1");
}

#[test]
fn extract_edit_success() {
    let call = make_call("edit", r#"{"path":"src/bar.rs","old":"a","new":"b"}"#);
    let candidate = extract_edit_audit(&call, "ok", None, "batch-2").unwrap();
    assert_eq!(candidate.tool, "edit");
    assert_eq!(candidate.path, "src/bar.rs");
    assert!(candidate.node_id.is_none());
}

#[test]
fn extract_delete_success() {
    let call = make_call("delete", r#"{"path":"src/old.rs"}"#);
    let candidate = extract_edit_audit(&call, "deleted", Some("t2"), "batch-3").unwrap();
    assert_eq!(candidate.tool, "delete");
}

#[test]
fn extract_skips_error_result() {
    let call = make_call("write", r#"{"path":"src/x.rs","content":"y"}"#);
    assert!(extract_edit_audit(&call, "error: denied", Some("t1"), "b1").is_none());
}

#[test]
fn extract_skips_non_edit_tool() {
    let call = make_call("read", r#"{"path":"src/x.rs"}"#);
    assert!(extract_edit_audit(&call, "ok", None, "b1").is_none());
}

#[test]
fn extract_skips_missing_path() {
    let call = make_call("write", r#"{"content":"x"}"#);
    assert!(extract_edit_audit(&call, "ok", None, "b1").is_none());
}

#[test]
fn parse_historian_valid() {
    let reply = r#"{"purpose":"Added error handling to the authentication module"}"#;
    let rec =
        parse_historian_reply(reply, "b1", Some("t1"), vec!["src/auth.rs".into()]).unwrap();
    assert_eq!(rec.batch_id, "b1");
    assert_eq!(
        rec.purpose,
        "Added error handling to the authentication module"
    );
    assert_eq!(rec.paths, vec!["src/auth.rs"]);
    assert_eq!(rec.node_id.as_deref(), Some("t1"));
}

#[test]
fn parse_historian_truncates_long_purpose() {
    let long_purpose = "x".repeat(500);
    let reply = format!(r#"{{"purpose":"{long_purpose}"}}"#);
    let rec = parse_historian_reply(&reply, "b1", None, vec![]).unwrap();
    assert!(rec.purpose.len() <= PURPOSE_MAX_LEN);
}

#[test]
fn parse_historian_rejects_empty_purpose() {
    let reply = r#"{"purpose":"  "}"#;
    assert!(parse_historian_reply(reply, "b1", None, vec![]).is_none());
}

#[test]
fn parse_historian_rejects_malformed() {
    assert!(parse_historian_reply("not json", "b1", None, vec![]).is_none());
    assert!(parse_historian_reply(r#"{"other":"x"}"#, "b1", None, vec![]).is_none());
}

#[test]
fn build_prompt_contains_goal_and_paths() {
    let audits = vec![
        EditAuditRecord {
            tool: "write".into(),
            path: "src/a.rs".into(),
            node_id: Some("t1".into()),
            batch_id: "b1".into(),
        },
        EditAuditRecord {
            tool: "edit".into(),
            path: "src/b.rs".into(),
            node_id: Some("t1".into()),
            batch_id: "b1".into(),
        },
    ];
    let (system, user) =
        build_historian_prompt("ship feature X", "execute", Some("implement"), &audits);
    assert!(system.contains("SDLC historian"));
    assert!(user.contains("ship feature X"));
    assert!(user.contains("execute"));
    assert!(user.contains("implement"));
    assert!(user.contains("src/a.rs"));
    assert!(user.contains("src/b.rs"));
}

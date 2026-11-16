#![allow(clippy::unwrap_used)]

use super::*;

fn parse(text: &str) -> Parsed {
    parse_servers_json(text, None).unwrap()
}

#[test]
fn claude_code_stdio_http_and_sse() {
    let p = parse(
        r#"{"mcpServers":{
            "fs":{"command":"npx","args":["-y","srv","."],"env":{"B":"2","A":"1"}},
            "dw":{"type":"http","url":"https://x/mcp"},
            "old":{"type":"sse","url":"https://x/sse"},
            "infer":{"url":"https://y/mcp"}}}"#,
    );
    assert_eq!(p.servers.len(), 4);
    let by = |n: &str| p.servers.iter().find(|s| s.name == n).unwrap();
    assert_eq!(by("fs").transport, McpTransport::Stdio);
    assert_eq!(by("fs").args, vec!["-y", "srv", "."]);
    assert_eq!(
        by("fs").env,
        vec![("A".into(), "1".into()), ("B".into(), "2".into())]
    );
    assert_eq!(by("dw").transport, McpTransport::Http);
    assert_eq!(by("old").transport, McpTransport::Http);
    assert_eq!(by("infer").transport, McpTransport::Http);
    assert!(by("fs").enabled);
    assert!(by("fs").uuid.is_empty());
    assert_eq!(p.warnings.len(), 1);
    assert!(p.warnings[0].contains("legacy SSE"));
}

#[test]
fn vscode_servers_and_bare_map() {
    let a = parse(r#"{"servers":{"a":{"type":"stdio","command":"x"}}}"#);
    assert_eq!(a.servers[0].name, "a");
    let b = parse(r#"{"b":{"command":"y"}}"#);
    assert_eq!(b.servers[0].name, "b");
}

#[test]
fn single_object_needs_a_name() {
    assert!(parse_servers_json(r#"{"command":"x"}"#, None).is_err());
    let p = parse_servers_json(r#"{"command":"x"}"#, Some("solo")).unwrap();
    assert_eq!(p.servers[0].name, "solo");
}

#[test]
fn readme_fragment_without_braces() {
    let p =
        parse(r#""github": {"command":"npx","args":["-y","s"],"env":{"T":"${GITHUB_TOKEN}"}},"#);
    assert_eq!(p.servers[0].name, "github");
    assert_eq!(
        p.servers[0].env,
        vec![("T".into(), "${GITHUB_TOKEN}".into())]
    );
}

#[test]
fn a_bad_entry_is_a_warning_the_rest_are_kept() {
    let p = parse(
        r#"{"mcpServers":{"bad":{"command":"x","env":{"N":1}},"ok":{"command":"y"},
            "empty":{"type":"stdio"},"weird":{"type":"carrier-pigeon","url":"u"}}}"#,
    );
    assert_eq!(p.servers.len(), 1);
    assert_eq!(p.servers[0].name, "ok");
    assert_eq!(p.warnings.len(), 3);
}

#[test]
fn nothing_importable_is_an_error() {
    assert!(parse_servers_json("not json at all", None).is_err());
    assert!(parse_servers_json("", None).is_err());
    assert!(parse_servers_json(r#"{"mcpServers":{}}"#, None).is_err());
}

#[test]
fn options_and_aliases() {
    let p = parse(
        r#"{"mcpServers":{"s":{"command":"x","cwd":"/w","startupTimeoutMs":5000,
            "toolTimeoutMs":0,"enabledTools":["a*"],"disabledTools":["b"],
            "headers":{"X":"y"},"disabled":true}}}"#,
    );
    let s = &p.servers[0];
    assert_eq!(s.cwd.as_deref(), Some("/w"));
    assert_eq!(s.startup_timeout_ms, Some(5000));
    assert_eq!(s.tool_timeout_ms, None);
    assert_eq!(s.allowed_tools, vec!["a*"]);
    assert_eq!(s.disabled_tools, vec!["b"]);
    assert_eq!(s.headers, vec![("X".into(), "y".into())]);
    assert!(!s.enabled);
}

#[test]
fn import_into_replaces_skips_and_mints() {
    let row = |uuid: &str, name: &str, ext: Option<&str>, cmd: &str| McpServerEntry {
        uuid: uuid.into(),
        name: name.into(),
        command: cmd.into(),
        ext_id: ext.map(str::to_string),
        ..Default::default()
    };
    let mut list = vec![
        row("u1", "a", None, "old"),
        row("u2", "b", Some("ext"), "ext"),
    ];
    let mut warnings = Vec::new();
    let n = import_into(
        &mut list,
        vec![
            row("", "a", None, "new"),
            row("", "b", None, "imp"),
            row("", "c", None, "c"),
        ],
        &mut warnings,
    );
    assert_eq!(n, 2);
    assert_eq!(list[0].uuid, "u1");
    assert_eq!(list[0].command, "new");
    assert_eq!(list[1].command, "ext");
    assert_eq!(list.len(), 3);
    assert!(!list[2].uuid.is_empty());
    assert_eq!(warnings.len(), 1);
}

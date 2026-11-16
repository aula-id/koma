#![allow(clippy::unwrap_used)]

use std::time::Duration;

use super::*;

fn server(allowed: &[&str], disabled: &[&str]) -> McpServerEntry {
    McpServerEntry {
        uuid: "u".into(),
        name: "everything".into(),
        enabled: true,
        allowed_tools: allowed.iter().map(|s| s.to_string()).collect(),
        disabled_tools: disabled.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn tool(name: &str) -> RmcpTool {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "description": "d",
        "inputSchema": {"type": "object"}
    }))
    .unwrap()
}

#[test]
fn no_lists_offers_everything() {
    assert!(tool_offered(&server(&[], &[]), "echo"));
}

#[test]
fn allow_only_restricts_to_matches() {
    let s = server(&["echo"], &[]);
    assert!(tool_offered(&s, "echo"));
    assert!(!tool_offered(&s, "add"));
}

#[test]
fn deny_only_hides_matches() {
    let s = server(&[], &["echo"]);
    assert!(!tool_offered(&s, "echo"));
    assert!(tool_offered(&s, "add"));
}

#[test]
fn deny_wins_over_allow() {
    let s = server(&["read_*"], &["read_secret"]);
    assert!(tool_offered(&s, "read_file"));
    assert!(!tool_offered(&s, "read_secret"));
    assert!(!tool_offered(&s, "write_file"));
}

#[test]
fn invalid_glob_matches_by_equality() {
    let s = server(&["[bad"], &[]);
    assert!(tool_offered(&s, "[bad"));
    assert!(!tool_offered(&s, "bad"));
}

#[test]
fn namespace_tools_drops_filtered_tools() {
    let s = server(&[], &["echo"]);
    let out = namespace_tools(&s, &[tool("echo"), tool("add")]);
    let names: Vec<_> = out.iter().map(|t| t.namespaced.as_str()).collect();
    assert_eq!(names, vec!["mcp__everything__add"]);
}

#[test]
fn timeouts_default_and_clamp() {
    let mut s = server(&[], &[]);
    assert_eq!(startup_timeout(&s), Duration::from_secs(20));
    assert_eq!(tool_timeout(&s), Duration::from_secs(60));
    s.startup_timeout_ms = Some(0);
    s.tool_timeout_ms = Some(0);
    assert_eq!(startup_timeout(&s), DEFAULT_STARTUP);
    assert_eq!(tool_timeout(&s), DEFAULT_TOOL);
    s.startup_timeout_ms = Some(10);
    s.tool_timeout_ms = Some(5000);
    assert_eq!(startup_timeout(&s), Duration::from_secs(1));
    assert_eq!(tool_timeout(&s), Duration::from_secs(5));
    s.startup_timeout_ms = Some(u64::MAX);
    s.tool_timeout_ms = Some(u64::MAX);
    assert_eq!(startup_timeout(&s), Duration::from_secs(600));
    assert_eq!(tool_timeout(&s), Duration::from_secs(3600));
}

#[test]
fn proxy_call_timeout_uses_longest_enabled_plus_margin_with_floor() {
    assert_eq!(proxy_call_timeout(&[]), Duration::from_secs(65));
    let mut slow = server(&[], &[]);
    slow.tool_timeout_ms = Some(300_000);
    let mut off = server(&[], &[]);
    off.enabled = false;
    off.tool_timeout_ms = Some(900_000);
    let mut quick = server(&[], &[]);
    quick.tool_timeout_ms = Some(2000);
    assert_eq!(
        proxy_call_timeout(&[slow, off, quick]),
        Duration::from_secs(305)
    );
    let mut only_quick = server(&[], &[]);
    only_quick.tool_timeout_ms = Some(2000);
    assert_eq!(proxy_call_timeout(&[only_quick]), Duration::from_secs(65));
}

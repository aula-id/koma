#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn legacy_entry_round_trips_with_exactly_the_same_keys() {
    let legacy = serde_json::json!({
        "uuid": "u1",
        "name": "fs",
        "enabled": true,
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "srv"],
        "env": [["A", "1"]],
        "url": ""
    });
    let entry: McpServerEntry = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&entry).unwrap(), legacy);
}

#[test]
fn new_fields_round_trip() {
    let entry = McpServerEntry {
        uuid: "u1".into(),
        name: "gh".into(),
        transport: McpTransport::Http,
        url: "https://x/mcp".into(),
        headers: vec![("Authorization".into(), "Bearer ${T}".into())],
        cwd: Some("/w".into()),
        startup_timeout_ms: Some(5000),
        tool_timeout_ms: Some(7000),
        allowed_tools: vec!["read_*".into()],
        disabled_tools: vec!["rm".into()],
        ..Default::default()
    };
    let back: McpServerEntry =
        serde_json::from_str(&serde_json::to_string(&entry).unwrap()).unwrap();
    assert_eq!(back, entry);
}

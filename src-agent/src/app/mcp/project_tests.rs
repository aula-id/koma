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

fn approval(root: &str, name: &str, fp: &str) -> McpProjectApproval {
    McpProjectApproval {
        root: root.into(),
        name: name.into(),
        fingerprint: fp.into(),
    }
}

const FILE: &str = r#"{"mcpServers":{
    "fs":{"command":"npx","args":["-y","srv"]},
    "rel":{"command":"x","cwd":"sub"},
    "var":{"command":"x","cwd":"${HOME}/p"},
    "web":{"type":"http","url":"https://x/mcp"}}}"#;

fn layer(approvals: &[McpProjectApproval]) -> ProjectLayer {
    layer_from_text(Path::new("/proj"), "/proj", FILE, approvals)
}

#[test]
fn project_uuid_is_stable_and_root_dependent() {
    assert_eq!(project_uuid("/a", "fs"), project_uuid("/a", "fs"));
    assert_ne!(project_uuid("/a", "fs"), project_uuid("/b", "fs"));
    assert_ne!(project_uuid("/a", "fs"), project_uuid("/a", "web"));
}

#[test]
fn fingerprint_tracks_what_runs_only() {
    let base = McpServerEntry {
        command: "npx".into(),
        args: vec!["a".into()],
        ..Default::default()
    };
    let mut changed = base.clone();
    changed.args.push("b".into());
    assert_ne!(fingerprint(&base), fingerprint(&changed));
    let mut renamed = base.clone();
    renamed.name = "other".into();
    renamed.enabled = true;
    renamed.uuid = "x".into();
    renamed.tool_timeout_ms = Some(5);
    assert_eq!(fingerprint(&base), fingerprint(&renamed));
}

#[test]
fn rows_start_pending_and_disabled() {
    let l = layer(&[]);
    assert_eq!(l.rows.len(), 4);
    assert!(l
        .rows
        .iter()
        .all(|r| r.approval == Approval::Pending && !r.entry.enabled));
    assert!(l.active().is_empty());
    assert!(l.hidden_prefixes().is_empty());
}

#[test]
fn approval_enables_and_an_edit_marks_changed() {
    let fp = layer(&[])
        .rows
        .iter()
        .find(|r| r.entry.name == "fs")
        .unwrap()
        .fingerprint
        .clone();
    let ok = layer(&[approval("/proj", "fs", &fp)]);
    let fs = ok.rows.iter().find(|r| r.entry.name == "fs").unwrap();
    assert_eq!(fs.approval, Approval::Approved);
    assert!(fs.entry.enabled);
    assert_eq!(ok.active().len(), 1);
    assert_eq!(ok.hidden_prefixes(), vec!["mcp__fs__".to_string()]);
    assert!(ok.overrides("FS"));
    assert!(!ok.overrides("web"));

    let stale = layer(&[approval("/proj", "fs", "old")]);
    let fs = stale.rows.iter().find(|r| r.entry.name == "fs").unwrap();
    assert_eq!(fs.approval, Approval::Changed);
    assert!(!fs.entry.enabled);
    let other_root = layer(&[approval("/elsewhere", "fs", &fp)]);
    assert!(other_root.active().is_empty());
}

#[test]
fn stdio_cwd_defaults_to_root() {
    let l = layer(&[]);
    let cwd = |n: &str| {
        l.rows
            .iter()
            .find(|r| r.entry.name == n)
            .unwrap()
            .entry
            .cwd
            .clone()
    };
    assert_eq!(cwd("fs").as_deref(), Some("/proj"));
    assert_eq!(cwd("rel").as_deref(), Some("/proj/sub"));
    assert_eq!(cwd("var").as_deref(), Some("${HOME}/p"));
    assert_eq!(cwd("web"), None);
}

#[test]
fn invalid_json_sets_error_and_no_rows() {
    let l = layer_from_text(Path::new("/p"), "/p", "{nope", &[]);
    assert!(l.error.is_some());
    assert!(l.rows.is_empty());
    assert!(l.snapshot().error.is_some());
}

#[test]
fn load_reads_file_and_rejects_oversize() {
    let dir = std::env::temp_dir().join(format!("koma-mcp-proj-{}", new_uuid()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(load_project_layer(&dir, &[]).is_none());
    std::fs::write(dir.join(".mcp.json"), FILE).unwrap();
    let l = load_project_layer(&dir, &[]).unwrap();
    assert_eq!(l.rows.len(), 4);
    assert!(l.error.is_none());
    std::fs::write(dir.join(".mcp.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    let big = load_project_layer(&dir, &[]).unwrap();
    assert!(big.error.unwrap().contains("1 MiB"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn load_rejects_a_file_outside_the_root() {
    let base = std::env::temp_dir().join(format!("koma-mcp-link-{}", new_uuid()));
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(base.join("real.json"), FILE).unwrap();
    std::os::unix::fs::symlink(base.join("real.json"), root.join(".mcp.json")).unwrap();
    let l = load_project_layer(&root, &[]).unwrap();
    assert!(l.error.unwrap().contains("outside"));
    assert!(l.rows.is_empty());
    std::fs::remove_dir_all(&base).unwrap();
}

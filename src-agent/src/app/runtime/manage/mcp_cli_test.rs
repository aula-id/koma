#![allow(clippy::unwrap_used)]

use super::*;
use crate::model::app_config::new_uuid;

fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("koma-mcp-cli-{}", new_uuid()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(cmd: McpCli, cfg: &mut AppConfig, cwd: &Path) -> (Outcome, String, String) {
    let (mut out, mut err) = (String::new(), String::new());
    let o = execute(cmd, cfg, cwd, &mut out, &mut err);
    (o, out, err)
}

fn add(name: &str, scope: McpScope, target: &[&str]) -> McpCli {
    McpCli::Add {
        name: name.into(),
        scope,
        transport: None,
        env: vec!["TOKEN=secret".into()],
        headers: Vec::new(),
        cwd: None,
        disabled: false,
        target: target.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn mask_keeps_only_pure_references() {
    assert_eq!(mask_value("${TOKEN}"), "${TOKEN}");
    assert_eq!(mask_value("Bearer ${TOKEN}"), "***");
    assert_eq!(mask_value("${A}${B}"), "***");
    assert_eq!(mask_value("plain"), "***");
}

#[test]
fn add_user_stdio_then_duplicate_is_refused() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    let (o, _, _) = run(
        add("fs", McpScope::User, &["npx", "-y", "srv"]),
        &mut cfg,
        &cwd,
    );
    assert_eq!(o.code, 0);
    assert!(o.config_changed && o.servers_changed);
    let e = &cfg.mcp_servers[0];
    assert_eq!((e.command.as_str(), e.args.len()), ("npx", 2));
    assert_eq!(e.env, vec![("TOKEN".to_string(), "secret".to_string())]);
    assert!(!e.uuid.is_empty() && e.enabled);

    let (o, _, err) = run(add("fs", McpScope::User, &["x"]), &mut cfg, &cwd);
    assert_eq!(o.code, 1);
    assert!(err.contains("koma mcp remove fs"), "{err}");
    assert_eq!(cfg.mcp_servers.len(), 1);
}

#[test]
fn add_http_infers_transport_and_sse_warns() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    let mut cmd = add("web", McpScope::User, &["https://x/mcp"]);
    if let McpCli::Add { env, headers, .. } = &mut cmd {
        env.clear();
        headers.push("Authorization: Bearer ${T}".into());
    }
    let (o, _, _) = run(cmd, &mut cfg, &cwd);
    assert_eq!(o.code, 0);
    assert_eq!(cfg.mcp_servers[0].transport, McpTransport::Http);
    assert_eq!(
        cfg.mcp_servers[0].headers,
        vec![("Authorization".to_string(), "Bearer ${T}".to_string())]
    );

    let mut sse = add("old", McpScope::User, &["https://x/sse"]);
    if let McpCli::Add { transport, env, .. } = &mut sse {
        *transport = Some("sse".into());
        env.clear();
    }
    let (o, _, err) = run(sse, &mut cfg, &cwd);
    assert_eq!(o.code, 0);
    assert!(err.contains("SSE"), "{err}");
}

#[test]
fn add_rejects_env_on_http_and_bad_pairs() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    let (o, _, err) = run(add("w", McpScope::User, &["https://x/mcp"]), &mut cfg, &cwd);
    assert_eq!(o.code, 1, "{err}");
    let mut bad = add("b", McpScope::User, &["cmd"]);
    if let McpCli::Add { env, .. } = &mut bad {
        *env = vec!["NOEQUALS".into()];
    }
    let (o, _, _) = run(bad, &mut cfg, &cwd);
    assert_eq!(o.code, 1);
    assert!(cfg.mcp_servers.is_empty());
}

#[test]
fn add_json_single_uses_name_and_map_imports_all() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    let cmd = McpCli::AddJson {
        name: "one".into(),
        scope: McpScope::User,
        json: r#"{"command":"npx","args":["a"]}"#.into(),
    };
    let (o, _, _) = run(cmd, &mut cfg, &cwd);
    assert_eq!(o.code, 0);
    assert_eq!(cfg.mcp_servers[0].name, "one");
    let cmd = McpCli::AddJson {
        name: "ignored".into(),
        scope: McpScope::User,
        json: r#"{"mcpServers":{"a":{"command":"x"},"b":{"url":"https://y/mcp"}}}"#.into(),
    };
    let (o, _, _) = run(cmd, &mut cfg, &cwd);
    assert_eq!(o.code, 0);
    assert_eq!(cfg.mcp_servers.len(), 3);
    let bad = McpCli::AddJson {
        name: "n".into(),
        scope: McpScope::User,
        json: "not json".into(),
    };
    assert_eq!(run(bad, &mut cfg, &cwd).0.code, 1);
}

#[test]
fn remove_user_and_extension_rows() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    run(add("fs", McpScope::User, &["x"]), &mut cfg, &cwd);
    let mut ext = cfg.mcp_servers[0].clone();
    ext.name = "extsrv".into();
    ext.uuid = new_uuid();
    ext.ext_id = Some("com.x.y".into());
    cfg.mcp_servers.push(ext);

    let rm = |n: &str| McpCli::Remove {
        name: n.into(),
        scope: McpScope::User,
    };
    let (o, _, err) = run(rm("extsrv"), &mut cfg, &cwd);
    assert_eq!(o.code, 1);
    assert!(err.contains("com.x.y"), "{err}");
    assert_eq!(run(rm("nope"), &mut cfg, &cwd).0.code, 1);
    let (o, _, _) = run(rm("fs"), &mut cfg, &cwd);
    assert_eq!((o.code, o.servers_changed), (0, true));
    assert_eq!(cfg.mcp_servers.len(), 1);
}

#[test]
fn list_and_get_mask_secrets() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    run(add("fs", McpScope::User, &["npx"]), &mut cfg, &cwd);
    let (_, out, _) = run(McpCli::List { json: false }, &mut cfg, &cwd);
    assert!(out.contains("fs  stdio  enabled  npx"), "{out}");
    let (_, out, _) = run(McpCli::List { json: true }, &mut cfg, &cwd);
    assert!(out.contains("***") && !out.contains("secret"), "{out}");
    let (o, out, _) = run(McpCli::Get { name: "fs".into() }, &mut cfg, &cwd);
    assert_eq!(o.code, 0);
    assert!(out.contains("\"fs\"") && out.contains("***") && !out.contains("secret"));
    assert_eq!(
        run(McpCli::Get { name: "zz".into() }, &mut cfg, &cwd)
            .0
            .code,
        1
    );
}

#[test]
fn project_scope_writes_mcp_json_and_approve_cycle() {
    let cwd = dir();
    let mut cfg = AppConfig::default();
    let (o, _, _) = run(
        add("fs", McpScope::Project, &["npx", "-y", "srv"]),
        &mut cfg,
        &cwd,
    );
    assert_eq!(o.code, 0);
    assert!(o.project_changed && !o.config_changed);
    assert!(cfg.mcp_servers.is_empty());
    let text = std::fs::read_to_string(cwd.join(".mcp.json")).unwrap();
    assert!(
        text.contains("mcpServers") && text.contains("\"fs\""),
        "{text}"
    );

    // Duplicate in the file is refused.
    assert_eq!(
        run(add("fs", McpScope::Project, &["x"]), &mut cfg, &cwd)
            .0
            .code,
        1
    );

    // Pending in the list, approved after `approve`, changed after the file is edited.
    let (_, out, _) = run(McpCli::List { json: false }, &mut cfg, &cwd);
    assert!(out.contains("needs approval"), "{out}");
    let approve = |all| McpCli::Approve {
        name: if all { None } else { Some("fs".into()) },
        all,
        revoke: false,
    };
    let (o, _, _) = run(approve(false), &mut cfg, &cwd);
    assert!(o.config_changed);
    assert_eq!(cfg.mcp_project_approvals.len(), 1);
    let (_, out, _) = run(McpCli::List { json: false }, &mut cfg, &cwd);
    assert!(out.contains("approved"), "{out}");
    run(approve(false), &mut cfg, &cwd);
    assert_eq!(
        cfg.mcp_project_approvals.len(),
        1,
        "approving twice must not duplicate"
    );

    run(
        McpCli::AddJson {
            name: "fs".into(),
            scope: McpScope::Project,
            json: r#"{"command":"other"}"#.into(),
        },
        &mut cfg,
        &cwd,
    );
    let (_, out, _) = run(McpCli::List { json: false }, &mut cfg, &cwd);
    assert!(out.contains("changed since approval"), "{out}");

    // Revoke, then remove from the file.
    let revoke = McpCli::Approve {
        name: Some("fs".into()),
        all: false,
        revoke: true,
    };
    run(revoke, &mut cfg, &cwd);
    assert!(cfg.mcp_project_approvals.is_empty());
    let rm = McpCli::Remove {
        name: "fs".into(),
        scope: McpScope::Project,
    };
    assert_eq!(run(rm, &mut cfg, &cwd).0.code, 0);
    let (_, out, _) = run(McpCli::List { json: false }, &mut cfg, &cwd);
    assert!(!out.contains("fs  stdio"), "{out}");
}

#[test]
fn project_scope_keeps_other_servers_and_rejects_bad_files() {
    let cwd = dir();
    std::fs::write(
        cwd.join(".mcp.json"),
        r#"{"mcpServers":{"keep":{"command":"a"}}}"#,
    )
    .unwrap();
    let mut cfg = AppConfig::default();
    run(add("new", McpScope::Project, &["b"]), &mut cfg, &cwd);
    let text = std::fs::read_to_string(cwd.join(".mcp.json")).unwrap();
    assert!(text.contains("keep") && text.contains("new"), "{text}");

    std::fs::write(cwd.join(".mcp.json"), "{ broken").unwrap();
    let (o, _, err) = run(add("x", McpScope::Project, &["b"]), &mut cfg, &cwd);
    assert_eq!(o.code, 1, "{err}");
    assert_eq!(
        std::fs::read_to_string(cwd.join(".mcp.json")).unwrap(),
        "{ broken"
    );

    let none = dir();
    let (o, _, _) = run(
        McpCli::Approve {
            name: None,
            all: true,
            revoke: false,
        },
        &mut cfg,
        &none,
    );
    assert_eq!(o.code, 1);
}

#[test]
fn usage_exits_two() {
    let (o, _, err) = run(McpCli::Usage, &mut AppConfig::default(), &dir());
    assert_eq!(o.code, 2);
    assert!(err.contains("usage: koma mcp"));
}

#[test]
fn scrub_url_hides_userinfo_and_query() {
    use super::scrub_url;
    assert_eq!(scrub_url("https://x/mcp"), "https://x/mcp");
    assert_eq!(scrub_url("https://x/mcp?key=abc"), "https://x/mcp?***");
    assert_eq!(scrub_url("https://u:p@h:8/m"), "https://***@h:8/m");
    assert_eq!(scrub_url("https://h/a@b"), "https://h/a@b");
}

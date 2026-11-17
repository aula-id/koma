//! `koma mcp <list|get|add|add-json|remove|approve>`: manage MCP servers from the shell,
//! without a TUI or a session. [`execute`] holds the behaviour and takes the config and
//! the working directory as arguments; [`run_mcp_cli`] loads, saves and notifies.

use std::fmt::Write as _;
use std::path::Path;

use crate::app::mcp::cli::{McpCli, McpScope, USAGE};
use crate::app::mcp::project::{
    self, entry_json, load_project_layer, remove_server_from_text, root_key, set_server_in_text,
    Approval, ProjectLayer,
};
use crate::model::app_config::{AppConfig, McpProjectApproval, McpServerEntry, McpTransport};

/// What [`execute`] did, so the caller knows what to save and announce.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub code: i32,
    /// `config.json` must be saved.
    pub config_changed: bool,
    /// The user server list changed (the global daemon needs the new set).
    pub servers_changed: bool,
    /// `./.mcp.json` was written.
    pub project_changed: bool,
}

/// Replace a secret with `***`; a pure `${VAR}` reference is not a secret and stays.
pub(crate) fn mask_value(v: &str) -> String {
    let pure_ref = v.starts_with("${") && v.ends_with('}') && v.matches("${").count() == 1;
    if pure_ref {
        v.to_string()
    } else {
        "***".to_string()
    }
}

/// `url` without credentials: userinfo and the query string (where `?key=…` tokens live)
/// are replaced by `***`.
fn scrub_url(url: &str) -> String {
    let (base, has_query) = match url.split_once(['?', '#']) {
        Some((b, _)) => (b, true),
        None => (url, false),
    };
    let base = match base.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = match rest.find('/') {
                Some(i) => rest.split_at(i),
                None => (rest, ""),
            };
            match authority.rsplit_once('@') {
                Some((_, host)) => format!("{scheme}://***@{host}{path}"),
                None => base.to_string(),
            }
        }
        None => base.to_string(),
    };
    if has_query {
        format!("{base}?***")
    } else {
        base
    }
}

fn masked(e: &McpServerEntry) -> McpServerEntry {
    let mut e = e.clone();
    e.url = scrub_url(&e.url);
    for (_, v) in e.env.iter_mut().chain(e.headers.iter_mut()) {
        *v = mask_value(v);
    }
    e
}

fn approval_text(a: Approval) -> &'static str {
    match a {
        Approval::Approved => "approved",
        Approval::Pending => "needs approval",
        Approval::Changed => "changed since approval",
    }
}

fn target_text(e: &McpServerEntry) -> String {
    match e.transport {
        McpTransport::Stdio => std::iter::once(e.command.as_str())
            .chain(e.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        McpTransport::Http => scrub_url(&e.url),
    }
}

fn transport_text(e: &McpServerEntry) -> &'static str {
    match e.transport {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
    }
}

fn parse_env(pairs: &[String]) -> Result<Vec<(String, String)>, String> {
    pairs
        .iter()
        .map(|p| match p.split_once('=') {
            Some((k, v)) if !k.trim().is_empty() => Ok((k.trim().to_string(), v.to_string())),
            _ => Err(format!("--env expects KEY=VALUE, got '{p}'")),
        })
        .collect()
}

fn parse_headers(pairs: &[String]) -> Result<Vec<(String, String)>, String> {
    pairs
        .iter()
        .map(|p| match p.split_once(':') {
            Some((k, v)) if !k.trim().is_empty() => {
                Ok((k.trim().to_string(), v.trim().to_string()))
            }
            _ => Err(format!("--header expects 'Name: value', got '{p}'")),
        })
        .collect()
}

/// The flags of `koma mcp add` after parsing.
struct AddArgs<'a> {
    name: &'a str,
    transport: Option<&'a str>,
    env: &'a [String],
    headers: &'a [String],
    cwd: Option<&'a str>,
    disabled: bool,
    target: &'a [String],
}

/// Build the entry for `koma mcp add`. A warning is pushed for `sse`.
fn build_entry(a: &AddArgs<'_>, warnings: &mut Vec<String>) -> Result<McpServerEntry, String> {
    let first = a.target.first().map(String::as_str).unwrap_or("");
    let transport = match a
        .transport
        .map(|t| t.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") => {
            if a.target.len() == 1
                && (first.starts_with("http://") || first.starts_with("https://"))
            {
                McpTransport::Http
            } else {
                McpTransport::Stdio
            }
        }
        Some("stdio") => McpTransport::Stdio,
        Some("http") | Some("streamable-http") => McpTransport::Http,
        Some("sse") => {
            warnings.push(
                "legacy SSE is not supported; added as streamable HTTP (often a URL ending in /mcp)"
                    .to_string(),
            );
            McpTransport::Http
        }
        Some(other) => return Err(format!("unknown transport '{other}' (use stdio or http)")),
    };
    if first.trim().is_empty() {
        return Err("a command or URL is required after --".to_string());
    }
    let mut entry = McpServerEntry {
        uuid: String::new(),
        name: a.name.trim().to_string(),
        enabled: !a.disabled,
        transport,
        command: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        url: String::new(),
        headers: Vec::new(),
        cwd: a
            .cwd
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(String::from),
        startup_timeout_ms: None,
        tool_timeout_ms: None,
        allowed_tools: Vec::new(),
        disabled_tools: Vec::new(),
        ext_id: None,
    };
    if entry.name.is_empty() {
        return Err("the server name is empty".to_string());
    }
    match transport {
        McpTransport::Stdio => {
            entry.command = first.trim().to_string();
            entry.args = a.target.iter().skip(1).cloned().collect();
            entry.env = parse_env(a.env)?;
            if !a.headers.is_empty() {
                return Err("--header applies to http servers only".to_string());
            }
        }
        McpTransport::Http => {
            if a.target.len() != 1 {
                return Err("an http server takes exactly one URL".to_string());
            }
            entry.url = first.trim().to_string();
            entry.headers = parse_headers(a.headers)?;
            if !a.env.is_empty() {
                return Err("--env applies to stdio servers only".to_string());
            }
        }
    }
    Ok(entry)
}

fn err_out(err: &mut String, msg: impl std::fmt::Display) -> Outcome {
    let _ = writeln!(err, "error: {msg}");
    Outcome {
        code: 1,
        ..Outcome::default()
    }
}

fn do_list(cfg: &AppConfig, cwd: &Path, json: bool, out: &mut String) -> Outcome {
    let layer = load_project_layer(cwd, &cfg.mcp_project_approvals);
    if json {
        let user: Vec<_> = cfg
            .mcp_servers
            .iter()
            .map(|s| {
                let mut v = entry_json(&masked(s));
                if let Some(o) = v.as_object_mut() {
                    o.insert("name".into(), s.name.clone().into());
                    o.insert("enabled".into(), s.enabled.into());
                    if let Some(id) = &s.ext_id {
                        o.insert("extension".into(), id.clone().into());
                    }
                    let shadowed = layer.as_ref().is_some_and(|l| l.overrides(&s.name));
                    o.insert("overridden".into(), shadowed.into());
                }
                v
            })
            .collect();
        let project = layer.as_ref().map(|l| {
            let servers: Vec<_> = l
                .rows
                .iter()
                .map(|r| {
                    let mut v = entry_json(&masked(&r.entry));
                    if let Some(o) = v.as_object_mut() {
                        o.insert("name".into(), r.entry.name.clone().into());
                        o.insert("approval".into(), r.approval.token().into());
                    }
                    v
                })
                .collect();
            serde_json::json!({
                "file": l.file.to_string_lossy(),
                "error": l.error,
                "servers": servers,
            })
        });
        let doc = serde_json::json!({ "user": user, "project": project });
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&doc).unwrap_or_default()
        );
        return Outcome::default();
    }
    if cfg.mcp_servers.is_empty() {
        let _ = writeln!(out, "No user MCP servers.");
    }
    for s in &cfg.mcp_servers {
        let mut line = format!(
            "{}  {}  {}  {}",
            s.name,
            transport_text(s),
            if s.enabled { "enabled" } else { "disabled" },
            target_text(s)
        );
        if let Some(id) = &s.ext_id {
            let _ = write!(line, "  [ext {id}]");
        }
        if layer.as_ref().is_some_and(|l| l.overrides(&s.name)) {
            line.push_str("  [overridden by .mcp.json]");
        }
        let _ = writeln!(out, "{line}");
    }
    if let Some(l) = &layer {
        let _ = writeln!(out, "\nProject servers ({}):", l.file.display());
        if let Some(e) = &l.error {
            let _ = writeln!(out, "  error: {e}");
        }
        for r in &l.rows {
            let _ = writeln!(
                out,
                "  {}  {}  {}  {}",
                r.entry.name,
                transport_text(&r.entry),
                approval_text(r.approval),
                target_text(&r.entry)
            );
        }
    }
    Outcome::default()
}

fn do_get(cfg: &AppConfig, cwd: &Path, name: &str, out: &mut String, err: &mut String) -> Outcome {
    let layer: Option<ProjectLayer> = load_project_layer(cwd, &cfg.mcp_project_approvals);
    let found = cfg.mcp_servers.iter().find(|s| s.name == name).or_else(|| {
        layer
            .as_ref()
            .and_then(|l| l.rows.iter().find(|r| r.entry.name == name))
            .map(|r| &r.entry)
    });
    let Some(entry) = found else {
        return err_out(err, format!("no MCP server named '{name}'"));
    };
    let doc = serde_json::json!({ name: entry_json(&masked(entry)) });
    let _ = writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    );
    Outcome::default()
}

/// Add already-built entries to the chosen scope. `replace` lets an existing row of the
/// same name be overwritten (`add-json`); `add` passes false and refuses.
#[allow(clippy::too_many_arguments)]
fn store_entries(
    cfg: &mut AppConfig,
    cwd: &Path,
    scope: McpScope,
    entries: Vec<McpServerEntry>,
    replace: bool,
    warnings: &mut Vec<String>,
    out: &mut String,
    err: &mut String,
) -> Outcome {
    match scope {
        McpScope::User => {
            if !replace {
                if let Some(e) = entries
                    .iter()
                    .find(|e| cfg.mcp_servers.iter().any(|s| s.name == e.name))
                {
                    return err_out(
                        err,
                        format!(
                            "an MCP server named '{}' already exists; run `koma mcp remove {}` first",
                            e.name, e.name
                        ),
                    );
                }
            }
            let applied = project::import_into(&mut cfg.mcp_servers, entries, warnings);
            let _ = writeln!(out, "Added {applied} MCP server(s) to the user config.");
            Outcome {
                config_changed: applied > 0,
                servers_changed: applied > 0,
                ..Outcome::default()
            }
        }
        McpScope::Project => {
            let mut text = match project::read_project_file(cwd) {
                Ok(t) => t,
                Err(e) => return err_out(err, e),
            };
            let mut count = 0;
            for entry in &entries {
                if entry
                    .env
                    .iter()
                    .chain(entry.headers.iter())
                    .any(|(_, v)| mask_value(v) != *v)
                {
                    warnings.push(format!(
                        "'{}': .mcp.json is meant to be committed; prefer ${{VAR}} references over literal secrets",
                        entry.name
                    ));
                }
                let exists = text
                    .as_deref()
                    .and_then(|t| remove_server_from_text(t, &entry.name).ok().flatten())
                    .is_some();
                if exists && !replace {
                    return err_out(
                        err,
                        format!(
                            "'{}' is already in .mcp.json; run `koma mcp remove {} --scope project` first",
                            entry.name, entry.name
                        ),
                    );
                }
                match set_server_in_text(text.as_deref(), entry) {
                    Ok(t) => text = Some(t),
                    Err(e) => return err_out(err, e),
                }
                count += 1;
            }
            let Some(text) = text else {
                return err_out(err, "nothing to write");
            };
            if let Err(e) = project::write_project_file(cwd, &text) {
                return err_out(err, e);
            }
            let _ = writeln!(
                out,
                "Added {count} server(s) to {}. Run `koma mcp approve <name>` to let them start.",
                cwd.join(".mcp.json").display()
            );
            Outcome {
                project_changed: count > 0,
                ..Outcome::default()
            }
        }
    }
}

fn do_remove(
    cfg: &mut AppConfig,
    cwd: &Path,
    name: &str,
    scope: McpScope,
    out: &mut String,
    err: &mut String,
) -> Outcome {
    match scope {
        McpScope::User => {
            let Some(row) = cfg.mcp_servers.iter().find(|s| s.name == name) else {
                return err_out(err, format!("no user MCP server named '{name}'"));
            };
            if let Some(id) = &row.ext_id {
                return err_out(
                    err,
                    format!("'{name}' is provided by extension {id}; uninstall the extension"),
                );
            }
            cfg.mcp_servers.retain(|s| s.name != name);
            let _ = writeln!(out, "Removed '{name}' from the user config.");
            Outcome {
                config_changed: true,
                servers_changed: true,
                ..Outcome::default()
            }
        }
        McpScope::Project => {
            let text = match project::read_project_file(cwd) {
                Ok(Some(t)) => t,
                Ok(None) => return err_out(err, "there is no .mcp.json here"),
                Err(e) => return err_out(err, e),
            };
            let new_text = match remove_server_from_text(&text, name) {
                Ok(Some(t)) => t,
                Ok(None) => return err_out(err, format!("no server named '{name}' in .mcp.json")),
                Err(e) => return err_out(err, e),
            };
            if let Err(e) = project::write_project_file(cwd, &new_text) {
                return err_out(err, e);
            }
            let key = root_key(cwd);
            let before = cfg.mcp_project_approvals.len();
            cfg.mcp_project_approvals
                .retain(|a| !(a.root == key && a.name == name));
            let _ = writeln!(out, "Removed '{name}' from .mcp.json.");
            Outcome {
                config_changed: cfg.mcp_project_approvals.len() != before,
                project_changed: true,
                ..Outcome::default()
            }
        }
    }
}

fn do_approve(
    cfg: &mut AppConfig,
    cwd: &Path,
    name: Option<&str>,
    revoke: bool,
    out: &mut String,
    err: &mut String,
) -> Outcome {
    let Some(layer) = load_project_layer(cwd, &cfg.mcp_project_approvals) else {
        return err_out(err, format!("no .mcp.json in {}", cwd.display()));
    };
    if let Some(e) = &layer.error {
        return err_out(err, e);
    }
    let rows: Vec<_> = layer
        .rows
        .iter()
        .filter(|r| name.is_none_or(|n| r.entry.name == n))
        .collect();
    if rows.is_empty() {
        return err_out(
            err,
            match name {
                Some(n) => format!("no server named '{n}' in .mcp.json"),
                None => "no servers in .mcp.json".to_string(),
            },
        );
    }
    for r in &rows {
        cfg.mcp_project_approvals
            .retain(|a| !(a.root == layer.root_key && a.name == r.entry.name));
        if !revoke {
            cfg.mcp_project_approvals.push(McpProjectApproval {
                root: layer.root_key.clone(),
                name: r.entry.name.clone(),
                fingerprint: r.fingerprint.clone(),
            });
        }
        let _ = writeln!(
            out,
            "{} '{}'",
            if revoke { "Revoked" } else { "Approved" },
            r.entry.name
        );
    }
    Outcome {
        config_changed: true,
        ..Outcome::default()
    }
}

/// Run one command against `cfg` and `cwd`, writing to `out` / `err`. Touches
/// `./.mcp.json` for project scope; never `config.json`.
pub(crate) fn execute(
    cmd: McpCli,
    cfg: &mut AppConfig,
    cwd: &Path,
    out: &mut String,
    err: &mut String,
) -> Outcome {
    match cmd {
        McpCli::Usage => {
            err.push_str(USAGE);
            Outcome {
                code: 2,
                ..Outcome::default()
            }
        }
        McpCli::List { json } => do_list(cfg, cwd, json, out),
        McpCli::Get { name } => do_get(cfg, cwd, &name, out, err),
        McpCli::Add {
            name,
            scope,
            transport,
            env,
            headers,
            cwd: server_cwd,
            disabled,
            target,
        } => {
            let mut warnings = Vec::new();
            let args = AddArgs {
                name: &name,
                transport: transport.as_deref(),
                env: &env,
                headers: &headers,
                cwd: server_cwd.as_deref(),
                disabled,
                target: &target,
            };
            let entry = match build_entry(&args, &mut warnings) {
                Ok(e) => e,
                Err(e) => return err_out(err, e),
            };
            let outcome =
                store_entries(cfg, cwd, scope, vec![entry], false, &mut warnings, out, err);
            for w in warnings {
                let _ = writeln!(err, "warning: {w}");
            }
            outcome
        }
        McpCli::AddJson { name, scope, json } => {
            let parsed = match project::parse_servers_json(&json, Some(&name)) {
                Ok(p) => p,
                Err(e) => return err_out(err, e),
            };
            let mut warnings = parsed.warnings;
            let outcome = store_entries(
                cfg,
                cwd,
                scope,
                parsed.servers,
                true,
                &mut warnings,
                out,
                err,
            );
            for w in warnings {
                let _ = writeln!(err, "warning: {w}");
            }
            outcome
        }
        McpCli::Remove { name, scope } => do_remove(cfg, cwd, &name, scope, out, err),
        McpCli::Approve { name, all, revoke } => {
            let target = if all { None } else { name.as_deref() };
            do_approve(cfg, cwd, target, revoke, out, err)
        }
    }
}

/// Entry point for `koma mcp …`. Returns the process exit code.
pub fn run_mcp_cli(cmd: McpCli) -> i32 {
    let mut cfg = match AppConfig::try_load() {
        Ok(c) => c,
        // First run: no config.json yet. Anything else (corrupt file) must not be saved over.
        Err(_)
            if crate::model::store::base_dir()
                .map(|d| !d.join("config.json").exists())
                .unwrap_or(false) =>
        {
            AppConfig::default()
        }
        Err(e) => {
            eprintln!("error: config.json is not valid: {e:#}");
            return 1;
        }
    };
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read the current directory: {e}");
            return 1;
        }
    };
    let (mut out, mut err) = (String::new(), String::new());
    let outcome = execute(cmd, &mut cfg, &cwd, &mut out, &mut err);
    print!("{out}");
    eprint!("{err}");
    if outcome.code != 0 {
        return outcome.code;
    }
    if outcome.config_changed {
        if let Err(e) = cfg.save() {
            eprintln!("error: could not save config.json: {e:#}");
            return 1;
        }
    }
    if outcome.config_changed || outcome.project_changed {
        let report = super::probe::broadcast_reload_global_catalogue_report();
        println!("updated {} running session(s)", report.reached);
        if report.failed > 0 {
            eprintln!("warning: {} session(s) could not be reached", report.failed);
        }
    }
    if outcome.servers_changed && super::mcp::mcp_daemon_alive() {
        match crate::app::mcp::reconnect_global_daemon(&cfg.mcp_servers) {
            Ok(()) => println!("global MCP daemon reconnected"),
            Err(e) => eprintln!("warning: could not reach the global MCP daemon: {e:#}"),
        }
    }
    0
}

#[cfg(test)]
#[path = "mcp_cli_test.rs"]
mod mcp_cli_test;

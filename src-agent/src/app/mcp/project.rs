//! Parsing of the MCP JSON shapes people paste from READMEs (Claude Code, Cursor,
//! VS Code) into [`McpServerEntry`] rows, and merging them into the user list.
//! Pure functions; the GUI import request and the pre-session config path share them.

use std::collections::BTreeMap;

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::ipc::proto::{McpProjectServerSnapshot, McpProjectSnapshot};
use crate::model::app_config::{new_uuid, McpProjectApproval, McpServerEntry, McpTransport};

use super::util::sanitize_server_name;

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct JsonServer {
    #[serde(rename = "type")]
    kind: Option<String>,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    url: String,
    headers: BTreeMap<String, String>,
    cwd: Option<String>,
    enabled: Option<bool>,
    disabled: Option<bool>,
    startup_timeout_ms: Option<u64>,
    tool_timeout_ms: Option<u64>,
    #[serde(alias = "enabledTools")]
    allowed_tools: Vec<String>,
    disabled_tools: Vec<String>,
}

pub(crate) struct Parsed {
    pub servers: Vec<McpServerEntry>,
    pub warnings: Vec<String>,
}

/// Parse any common MCP JSON shape:
///  `{"mcpServers":{name:{..}}}` (Claude Code / Cursor), `{"servers":{name:{..}}}` (VS Code),
///  a bare `{name:{..}}` map, a single server object (`command` or `url`; needs `name_hint`),
///  or a README fragment `"name": {..}` without outer braces (retried wrapped in `{}`).
/// Each server is decoded independently: a bad entry becomes a warning, not a failure.
/// Returned entries have an EMPTY uuid.
pub(crate) fn parse_servers_json(text: &str, name_hint: Option<&str>) -> Result<Parsed, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("nothing to import".to_string());
    }
    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(first) => serde_json::from_str(&format!("{{{}}}", trimmed.trim_end_matches(',')))
            .map_err(|_| format!("not valid JSON: {first}"))?,
    };
    let serde_json::Value::Object(root) = value else {
        return Err("expected a JSON object".to_string());
    };

    let mut warnings = Vec::new();
    let named = |key: &str| match root.get(key) {
        Some(serde_json::Value::Object(map)) => Some(map.clone()),
        _ => None,
    };
    let is_single = |v: &serde_json::Value| matches!(v, serde_json::Value::String(_));
    let map = if let Some(map) = named("mcpServers").or_else(|| named("servers")) {
        map
    } else if root.get("command").is_some_and(is_single) || root.get("url").is_some_and(is_single) {
        let name = name_hint
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                "a single server needs a name; wrap it as {\"name\": {...}}".to_string()
            })?;
        let mut map = serde_json::Map::new();
        map.insert(name.to_string(), serde_json::Value::Object(root));
        map
    } else {
        root
    };

    let mut servers = Vec::new();
    for (name, raw) in map {
        match entry_from_json(&name, raw, &mut warnings) {
            Ok(Some(entry)) => servers.push(entry),
            Ok(None) => {}
            Err(msg) => warnings.push(format!("'{name}': {msg}")),
        }
    }
    if servers.is_empty() {
        return Err(match warnings.first() {
            Some(w) => format!("no server could be imported: {w}"),
            None => "no servers found".to_string(),
        });
    }
    Ok(Parsed { servers, warnings })
}

fn entry_from_json(
    name: &str,
    raw: serde_json::Value,
    warnings: &mut Vec<String>,
) -> Result<Option<McpServerEntry>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("empty server name".to_string());
    }
    let j: JsonServer = serde_json::from_value(raw).map_err(|e| format!("invalid entry: {e}"))?;
    let kind = j
        .kind
        .as_deref()
        .map(|k| k.trim().to_ascii_lowercase().replace(['-', '_'], ""));
    let transport = match kind.as_deref() {
        None | Some("") => {
            if !j.url.trim().is_empty() && j.command.trim().is_empty() {
                McpTransport::Http
            } else {
                McpTransport::Stdio
            }
        }
        Some("stdio") => McpTransport::Stdio,
        Some("http") | Some("streamablehttp") => McpTransport::Http,
        Some("sse") => {
            warnings.push(format!(
                "'{name}': legacy SSE transport is not supported; imported as streamable HTTP \
                 (many servers serve both, often at a URL ending in /mcp)"
            ));
            McpTransport::Http
        }
        Some(other) => {
            warnings.push(format!("'{name}': unknown transport '{other}'; skipped"));
            return Ok(None);
        }
    };
    match transport {
        McpTransport::Stdio if j.command.trim().is_empty() => {
            warnings.push(format!("'{name}': no command; skipped"));
            return Ok(None);
        }
        McpTransport::Http if j.url.trim().is_empty() => {
            warnings.push(format!("'{name}': no url; skipped"));
            return Ok(None);
        }
        _ => {}
    }
    Ok(Some(McpServerEntry {
        uuid: String::new(),
        name: name.to_string(),
        enabled: j.disabled != Some(true) && j.enabled != Some(false),
        transport,
        command: j.command.trim().to_string(),
        args: j.args,
        env: j.env.into_iter().collect(),
        url: j.url.trim().to_string(),
        headers: j.headers.into_iter().collect(),
        cwd: j.cwd.filter(|c| !c.trim().is_empty()),
        startup_timeout_ms: j.startup_timeout_ms.filter(|ms| *ms > 0),
        tool_timeout_ms: j.tool_timeout_ms.filter(|ms| *ms > 0),
        allowed_tools: j.allowed_tools,
        disabled_tools: j.disabled_tools,
        ext_id: None,
    }))
}

/// Merge imported servers into the user list: a user row (no `ext_id`) with the same
/// name is replaced in place keeping its uuid; an extension-owned row with the same
/// name is left alone and the import is skipped with a warning; otherwise a new uuid is
/// minted. Returns how many servers were added or replaced.
pub(crate) fn import_into(
    list: &mut Vec<McpServerEntry>,
    imported: Vec<McpServerEntry>,
    warnings: &mut Vec<String>,
) -> usize {
    let mut applied = 0;
    for mut entry in imported {
        match list.iter_mut().find(|s| s.name == entry.name) {
            Some(slot) if slot.ext_id.is_some() => warnings.push(format!(
                "'{}': an extension already provides a server with this name; skipped",
                entry.name
            )),
            Some(slot) => {
                entry.uuid = slot.uuid.clone();
                *slot = entry;
                applied += 1;
            }
            None => {
                entry.uuid = new_uuid();
                list.push(entry);
                applied += 1;
            }
        }
    }
    applied
}

/// Whether the user allowed a project server to run, as of the file's current content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Approval {
    Approved,
    Pending,
    Changed,
}

impl Approval {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Approval::Approved => "approved",
            Approval::Pending => "pending",
            Approval::Changed => "changed",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectRow {
    pub entry: McpServerEntry,
    pub approval: Approval,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectLayer {
    pub file: PathBuf,
    pub root_key: String,
    pub error: Option<String>,
    pub rows: Vec<ProjectRow>,
}

/// Largest `.mcp.json` read, the same cap `.koma/coding.json` uses.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// A v5 uuid from root + name, so a project server keeps one id across reloads.
pub(crate) fn project_uuid(root_key: &str, name: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("koma-mcp-project:{root_key}:{name}").as_bytes(),
    )
    .to_string()
}

/// sha256 hex over everything that decides what runs: transport, command, args, env,
/// url, headers and cwd. Not the name, the enabled flag or the timeouts and tool lists.
pub(crate) fn fingerprint(e: &McpServerEntry) -> String {
    #[derive(serde::Serialize)]
    struct Runs<'a> {
        transport: &'static str,
        command: &'a str,
        args: &'a [String],
        env: &'a [(String, String)],
        url: &'a str,
        headers: &'a [(String, String)],
        cwd: Option<&'a str>,
    }
    let runs = Runs {
        transport: match e.transport {
            McpTransport::Stdio => "stdio",
            McpTransport::Http => "http",
        },
        command: &e.command,
        args: &e.args,
        env: &e.env,
        url: &e.url,
        headers: &e.headers,
        cwd: e.cwd.as_deref(),
    };
    let bytes = serde_json::to_vec(&runs).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

/// Build the layer from the file text. Every server gets a stable uuid, a stdio server
/// runs in the project root unless it names a cwd, and it is only `enabled` once its
/// fingerprint matches an approval for the same root and name.
pub(crate) fn layer_from_text(
    root: &Path,
    root_key: &str,
    text: &str,
    approvals: &[McpProjectApproval],
) -> ProjectLayer {
    let mut layer = ProjectLayer {
        file: root.join(".mcp.json"),
        root_key: root_key.to_string(),
        error: None,
        rows: Vec::new(),
    };
    let parsed = match parse_servers_json(text, None) {
        Ok(p) => p,
        Err(e) => {
            layer.error = Some(e);
            return layer;
        }
    };
    for mut entry in parsed.servers {
        let fingerprint = fingerprint(&entry);
        let approval = match approvals
            .iter()
            .find(|a| a.root == root_key && a.name == entry.name)
        {
            Some(a) if a.fingerprint == fingerprint => Approval::Approved,
            Some(_) => Approval::Changed,
            None => Approval::Pending,
        };
        entry.uuid = project_uuid(root_key, &entry.name);
        if entry.transport == McpTransport::Stdio {
            entry.cwd = Some(match entry.cwd.take() {
                Some(c) if c.starts_with("${") || Path::new(&c).is_absolute() => c,
                Some(c) => root.join(c).to_string_lossy().into_owned(),
                None => root.to_string_lossy().into_owned(),
            });
        }
        entry.enabled = entry.enabled && approval == Approval::Approved;
        layer.rows.push(ProjectRow {
            entry,
            approval,
            fingerprint,
        });
    }
    layer
}

/// The canonical form of a project root, used as the approval key.
pub(crate) fn root_key(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// The text of `<root>/.mcp.json`: `Ok(None)` when it does not exist, an error when it
/// resolves outside the root (a symlink), is larger than 1 MiB or cannot be read.
pub(crate) fn read_project_file(root: &Path) -> Result<Option<String>, String> {
    let file = root.join(".mcp.json");
    // `symlink_metadata` so a dangling symlink counts as present (and then fails the
    // canonicalize below) instead of letting a write follow it out of the root.
    if file.symlink_metadata().is_err() {
        return Ok(None);
    }
    let root_real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let actual = file
        .canonicalize()
        .map_err(|e| format!("cannot read .mcp.json: {e}"))?;
    if !actual.starts_with(&root_real) {
        return Err(".mcp.json is outside the project folder".to_string());
    }
    let meta = actual
        .metadata()
        .map_err(|e| format!("cannot read .mcp.json: {e}"))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(".mcp.json is larger than 1 MiB".to_string());
    }
    std::fs::read_to_string(&actual)
        .map(Some)
        .map_err(|e| format!("cannot read .mcp.json: {e}"))
}

/// `None` when `<root>/.mcp.json` does not exist. A file that resolves outside the root
/// (a symlink) or is larger than 1 MiB yields a layer carrying only an error.
pub(crate) fn load_project_layer(
    root: &Path,
    approvals: &[McpProjectApproval],
) -> Option<ProjectLayer> {
    let key = root_key(root);
    match read_project_file(root) {
        Ok(None) => None,
        Ok(Some(text)) => Some(layer_from_text(root, &key, &text, approvals)),
        Err(error) => Some(ProjectLayer {
            file: root.join(".mcp.json"),
            root_key: key,
            error: Some(error),
            rows: Vec::new(),
        }),
    }
}

/// One server in the Claude Code JSON shape (`type`, `command`, `args`, `env`, `url`,
/// `headers`, `cwd`, plus the koma extras when set). Values are written as stored.
pub(crate) fn entry_json(e: &McpServerEntry) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let pairs = |p: &[(String, String)]| -> Value {
        Value::Object(
            p.iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect::<Map<String, Value>>(),
        )
    };
    let mut o = Map::new();
    match e.transport {
        McpTransport::Stdio => {
            o.insert("type".into(), json!("stdio"));
            o.insert("command".into(), json!(e.command));
            if !e.args.is_empty() {
                o.insert("args".into(), json!(e.args));
            }
            if !e.env.is_empty() {
                o.insert("env".into(), pairs(&e.env));
            }
        }
        McpTransport::Http => {
            o.insert("type".into(), json!("http"));
            o.insert("url".into(), json!(e.url));
            if !e.headers.is_empty() {
                o.insert("headers".into(), pairs(&e.headers));
            }
        }
    }
    if let Some(c) = &e.cwd {
        o.insert("cwd".into(), json!(c));
    }
    if !e.enabled {
        o.insert("disabled".into(), json!(true));
    }
    if let Some(ms) = e.startup_timeout_ms {
        o.insert("startupTimeoutMs".into(), json!(ms));
    }
    if let Some(ms) = e.tool_timeout_ms {
        o.insert("toolTimeoutMs".into(), json!(ms));
    }
    if !e.allowed_tools.is_empty() {
        o.insert("allowedTools".into(), json!(e.allowed_tools));
    }
    if !e.disabled_tools.is_empty() {
        o.insert("disabledTools".into(), json!(e.disabled_tools));
    }
    Value::Object(o)
}

/// Which object of a parsed `.mcp.json` holds the servers. `Err` when the file is a
/// single bare server (no name to edit under) or not an object.
fn servers_slot(
    root: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<&mut serde_json::Map<String, serde_json::Value>, String> {
    use serde_json::Value;
    let key = if root.get("mcpServers").is_some_and(Value::is_object) {
        Some("mcpServers")
    } else if root.get("servers").is_some_and(Value::is_object) {
        Some("servers")
    } else if root.is_empty() {
        root.insert("mcpServers".into(), Value::Object(Default::default()));
        Some("mcpServers")
    } else if root.get("command").is_some_and(Value::is_string)
        || root.get("url").is_some_and(Value::is_string)
    {
        return Err(
            "the file holds a single unnamed server; wrap it as {\"mcpServers\": {...}} first"
                .into(),
        );
    } else {
        None
    };
    match key {
        Some(k) => match root.get_mut(k) {
            Some(Value::Object(m)) => Ok(m),
            _ => Err("cannot edit .mcp.json".to_string()),
        },
        None => Ok(root),
    }
}

fn parse_root(text: Option<&str>) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(Default::default()),
        Some(t) => match serde_json::from_str::<serde_json::Value>(t) {
            Ok(serde_json::Value::Object(m)) => Ok(m),
            Ok(_) => Err(".mcp.json is not a JSON object".to_string()),
            Err(e) => Err(format!(".mcp.json is not valid JSON: {e}")),
        },
    }
}

fn to_pretty(root: serde_json::Map<String, serde_json::Value>) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .map(|mut s| {
            s.push('\n');
            s
        })
        .map_err(|e| e.to_string())
}

/// The file text with `entry` added or replaced under its name. `text` is the current
/// file (`None` for a new one). Other servers and keys are kept.
pub(crate) fn set_server_in_text(
    text: Option<&str>,
    entry: &McpServerEntry,
) -> Result<String, String> {
    let mut root = parse_root(text)?;
    servers_slot(&mut root)?.insert(entry.name.clone(), entry_json(entry));
    to_pretty(root)
}

/// The file text without server `name`; `Ok(None)` when no such server is in it.
pub(crate) fn remove_server_from_text(text: &str, name: &str) -> Result<Option<String>, String> {
    let mut root = parse_root(Some(text))?;
    if servers_slot(&mut root)?.remove(name).is_none() {
        return Ok(None);
    }
    to_pretty(root).map(Some)
}

/// Write `<root>/.mcp.json`, applying the same checks as reading it: an existing file
/// must not resolve outside the root, and the text must fit the size cap.
pub(crate) fn write_project_file(root: &Path, text: &str) -> Result<(), String> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(".mcp.json would be larger than 1 MiB".to_string());
    }
    read_project_file(root)?;
    let file = root.join(".mcp.json");
    let target = file.canonicalize().unwrap_or(file);
    std::fs::write(&target, text).map_err(|e| format!("cannot write .mcp.json: {e}"))
}

impl ProjectLayer {
    /// The servers that may run now (approved and enabled).
    pub(crate) fn active(&self) -> Vec<McpServerEntry> {
        self.rows
            .iter()
            .filter(|r| r.entry.enabled)
            .map(|r| r.entry.clone())
            .collect()
    }

    /// Tool-name prefixes of the user servers a project server shadows in this session.
    pub(crate) fn hidden_prefixes(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.entry.enabled)
            .map(|r| format!("mcp__{}__", sanitize_server_name(&r.entry.name)))
            .collect()
    }

    /// Whether an active project server overrides the user server called `user_name`.
    pub(crate) fn overrides(&self, user_name: &str) -> bool {
        let wanted = sanitize_server_name(user_name);
        self.rows
            .iter()
            .any(|r| r.entry.enabled && sanitize_server_name(&r.entry.name) == wanted)
    }

    pub(crate) fn snapshot(&self) -> McpProjectSnapshot {
        McpProjectSnapshot {
            file: self.file.to_string_lossy().into_owned(),
            error: self.error.clone(),
            servers: self
                .rows
                .iter()
                .map(|r| McpProjectServerSnapshot {
                    entry: r.entry.clone(),
                    approval: r.approval.token().to_string(),
                })
                .collect(),
        }
    }
}

/// The toast text for a finished import: the count plus every warning.
pub(crate) fn import_summary(applied: usize, warnings: &[String]) -> String {
    let mut text = format!("Imported {applied} MCP server(s).");
    for w in warnings {
        text.push(' ');
        text.push_str(w);
    }
    text
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod project_tests;

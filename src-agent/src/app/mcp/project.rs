//! Parsing of the MCP JSON shapes people paste from READMEs (Claude Code, Cursor,
//! VS Code) into [`McpServerEntry`] rows, and merging them into the user list.
//! Pure functions; the GUI import request and the pre-session config path share them.

use std::collections::BTreeMap;

use crate::model::app_config::{new_uuid, McpServerEntry, McpTransport};

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

//! Naming + result-flattening helpers for the MCP client. Split out of
//! [`super`] (the `mcp` module) for file size; all three functions are bumped
//! to `pub(super)` — each is called directly from `McpManager` methods in the
//! parent module (not just internally within this file). No behaviour change.

use std::time::Duration;

use rmcp::model::Tool as RmcpTool;

use crate::model::app_config::McpServerEntry;

use super::{DiscoveredTool, ToolSource, PROXY_IO_TIMEOUT};

/// How long a server connect (spawn + initialize + first tool list) may take when
/// the entry sets no `startup_timeout_ms`.
pub(super) const DEFAULT_STARTUP: Duration = Duration::from_secs(20);
/// How long one `call_tool` round-trip may take when the entry sets no `tool_timeout_ms`.
pub(super) const DEFAULT_TOOL: Duration = Duration::from_secs(60);
const MIN_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_STARTUP: Duration = Duration::from_secs(600);
const MAX_TOOL: Duration = Duration::from_secs(3600);

fn clamped(ms: Option<u64>, default: Duration, max: Duration) -> Duration {
    match ms {
        None | Some(0) => default,
        Some(ms) => Duration::from_millis(ms).clamp(MIN_TIMEOUT, max),
    }
}

/// Connect budget for `s`: unset or 0 is the default, anything else clamps to [1 s, 10 min].
pub(super) fn startup_timeout(s: &McpServerEntry) -> Duration {
    clamped(s.startup_timeout_ms, DEFAULT_STARTUP, MAX_STARTUP)
}

/// Per tool-call budget for `s`: unset or 0 is the default, anything else clamps to [1 s, 1 h].
pub(super) fn tool_timeout(s: &McpServerEntry) -> Duration {
    clamped(s.tool_timeout_ms, DEFAULT_TOOL, MAX_TOOL)
}

/// Proxy read budget for a `Call`: the longest enabled server tool timeout plus 5 s so
/// the daemon's own timeout message arrives first, never below [`PROXY_IO_TIMEOUT`].
pub(super) fn proxy_call_timeout(servers: &[McpServerEntry]) -> Duration {
    servers
        .iter()
        .filter(|s| s.enabled)
        .map(|s| tool_timeout(s) + Duration::from_secs(5))
        .fold(PROXY_IO_TIMEOUT, Duration::max)
}

/// Whether the tool with ORIGINAL name `original` is offered: it must match
/// `allowed_tools` (when non-empty) and must not match `disabled_tools`. Patterns are
/// globs; a pattern that does not compile only matches by equality.
pub(super) fn tool_offered(s: &McpServerEntry, original: &str) -> bool {
    let hit = |pat: &String| match globset::Glob::new(pat) {
        Ok(g) => g.compile_matcher().is_match(original),
        Err(_) => pat == original,
    };
    (s.allowed_tools.is_empty() || s.allowed_tools.iter().any(hit))
        && !s.disabled_tools.iter().any(hit)
}

/// Turn a server's raw rmcp tools into namespaced [`DiscoveredTool`]s.
pub(super) fn namespace_tools(server: &McpServerEntry, tools: &[RmcpTool]) -> Vec<DiscoveredTool> {
    let prefix = sanitize_server_name(&server.name);
    tools
        .iter()
        .filter(|t| tool_offered(server, &t.name))
        .map(|t| {
            let original = t.name.to_string();
            DiscoveredTool {
                namespaced: format!("mcp__{prefix}__{original}"),
                description: t
                    .description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
                // `input_schema` is an `Arc<JsonObject>` (serde_json::Map); wrap it
                // back into a `Value::Object` so it rides the wire as the tool's
                // raw JSON-Schema `parameters`, exactly like a built-in tool.
                parameters: serde_json::Value::Object((*t.input_schema).clone()),
                source: ToolSource::McpServer(server.uuid.clone()),
                original,
            }
        })
        .collect()
}

/// Turn an extension's `contributes.tools` into namespaced [`DiscoveredTool`]s,
/// mirroring [`namespace_tools`] but for `app::ext`-owned tools: the `<server>`
/// segment is the sanitized extension id, and `source` is
/// [`ToolSource::Extension`] so [`super::McpManager::execute_blocking`] routes
/// calls through `ExtHostManager::invoke` instead of an rmcp `Peer`. See
/// [`super::McpManager::register_extension_tools`].
pub(super) fn namespace_ext_tools(
    ext_id: &str,
    tools: &[koma_extension::protocol::ToolDef],
) -> Vec<DiscoveredTool> {
    let prefix = sanitize_server_name(ext_id);
    tools
        .iter()
        .map(|t| DiscoveredTool {
            namespaced: format!("mcp__{prefix}__{}", t.name),
            description: t.description.clone(),
            parameters: t.input_schema.clone(),
            source: ToolSource::Extension(ext_id.to_string()),
            original: t.name.clone(),
        })
        .collect()
}

/// Sanitise a server name into the `<server>` segment of a namespaced tool name:
/// lowercase, and collapse every run of non-`[a-z0-9_]` characters to a single
/// `_`, trimming leading/trailing `_`. An empty/garbage name degrades to
/// `"server"` so the namespaced tool name is always well-formed.
pub(super) fn sanitize_server_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_underscore = false;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            prev_underscore = c == '_';
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "server".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Flatten an [`rmcp::model::CallToolResult`] into a single string for the model.
///
/// Text content blocks are concatenated (newline-separated); non-text blocks are
/// noted by kind so the model knows something non-textual came back. When the
/// server flagged the result as an error, the flattened text is returned as
/// `Err(...)` so the dispatcher renders it as a tool error.
pub(super) fn flatten_result(res: rmcp::model::CallToolResult) -> Result<String, String> {
    use rmcp::model::RawContent;

    let mut parts: Vec<String> = Vec::new();
    for c in &res.content {
        // `Content` derefs to `RawContent`; match the underlying variant.
        match &c.raw {
            RawContent::Text(t) => parts.push(t.text.clone()),
            RawContent::Image(_) => parts.push("[image content]".to_string()),
            RawContent::Audio(_) => parts.push("[audio content]".to_string()),
            RawContent::Resource(_) => parts.push("[embedded resource]".to_string()),
            RawContent::ResourceLink(_) => parts.push("[resource link]".to_string()),
        }
    }
    // Fall back to structured content if there were no content blocks at all.
    if parts.is_empty() {
        if let Some(sc) = &res.structured_content {
            parts.push(sc.to_string());
        }
    }
    let text = parts.join("\n");

    if res.is_error == Some(true) {
        Err(if text.is_empty() {
            "tool reported an error".to_string()
        } else {
            text
        })
    } else {
        Ok(text)
    }
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod util_tests;

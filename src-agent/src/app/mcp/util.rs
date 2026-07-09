//! Naming + result-flattening helpers for the MCP client. Split out of
//! [`super`] (the `mcp` module) for file size; all three functions are bumped
//! to `pub(super)` — each is called directly from `McpManager` methods in the
//! parent module (not just internally within this file). No behaviour change.

use rmcp::model::Tool as RmcpTool;

use crate::model::app_config::McpServerEntry;

use super::DiscoveredTool;

/// Turn a server's raw rmcp tools into namespaced [`DiscoveredTool`]s.
pub(super) fn namespace_tools(server: &McpServerEntry, tools: &[RmcpTool]) -> Vec<DiscoveredTool> {
    let prefix = sanitize_server_name(&server.name);
    tools
        .iter()
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
                server_uuid: server.uuid.clone(),
                original,
            }
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

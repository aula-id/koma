//! `dir_list` tool — list directory contents from the indexed file tree.

use crate::tool::{resolve, Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};

/// List directory contents from the indexed file tree (cache-backed, multi-path).
pub struct DirList;

impl Tool for DirList {
    fn name(&self) -> &'static str {
        "dir_list"
    }
    fn description(&self) -> &'static str {
        "List the contents of one or more workspace directories from the indexed file tree (folders end with '/'). Pass `paths` (an array) to list several directories in one call — prefer this over many separate calls."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Workspace-relative directory paths to list. Use [\".\"] for the workspace root. Prefix with [N] to target workspace N (e.g. \"[1]src\")."
                },
                "path": { "type": "string", "description": "A single directory (alternative to `paths`)." }
            }
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        // Accept `paths` (array) or a single `path`; default to the workspace root.
        let mut dirs: Vec<String> = Vec::new();
        if let Some(arr) = args.get("paths").and_then(Value::as_array) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    dirs.push(s.to_string());
                }
            }
        }
        if let Some(s) = args.get("path").and_then(Value::as_str) {
            dirs.push(s.to_string());
        }
        if dirs.is_empty() {
            dirs.push(".".to_string());
        }

        const MAX_ENTRIES: usize = 500;

        let cache = ctx
            .dir_cache
            .read()
            .map_err(|_| anyhow::anyhow!("dir cache unavailable"))?;
        let mut out = String::new();
        let mut total_entries = 0usize;
        let mut truncated = false;
        'outer: for dir in &dirs {
            // sandbox the path even though we read from the cache, to reject escapes
            let _ = resolve(&ctx.workspaces, dir)?;
            let (ws_idx, bare) = super::super::parse_ws_prefix(dir);
            let children = cache.children(bare, ws_idx);
            let label = if bare.is_empty() || bare == "." {
                if ws_idx == 0 {
                    ".".to_string()
                } else {
                    format!("[{ws_idx}].")
                }
            } else {
                dir.to_string()
            };
            out.push_str(&format!("{label}:\n"));
            if children.is_empty() {
                out.push_str("  (empty or not indexed)\n");
            } else {
                for c in &children {
                    if total_entries >= MAX_ENTRIES {
                        truncated = true;
                        break 'outer;
                    }
                    out.push_str("  ");
                    out.push_str(c);
                    out.push('\n');
                    total_entries += 1;
                }
            }
            out.push('\n');
        }
        if truncated {
            out.push_str("... (truncated at 500 entries; use a more specific path)");
        }
        Ok(out.trim_end().to_string())
    }
}

//! `write` tool — create or overwrite a workspace-relative file.

use super::helpers::arg_str;
use crate::tool::{resolve, Tool, ToolCtx};
use anyhow::{Context, Result};
use serde_json::{json, Value};

/// Create or overwrite a workspace-relative file.
pub struct Write;

impl Tool for Write {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Create or overwrite a workspace-relative file with the given content."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative or absolute path under a configured workspace root. A bare relative path targets workspace [0]." },
                "content": { "type": "string", "description": "Full file content to write" }
            },
            "required": ["path", "content"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let rel = arg_str(args, "path")?;
        let content = arg_str(args, "content")?;
        let path = resolve(&ctx.workspaces, rel)?;
        // Probe existence BEFORE the write so the file-change log can distinguish
        // "added" (new file) from "modified" (overwrite) — write is create-or-overwrite.
        let existed = path.exists();
        // Baseline pre-image BEFORE the overwrite ("virtual git", first-touch-wins) —
        // a missing file records the empty-baseline create marker.
        super::capture_baseline(ctx, &path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent directories for '{rel}'"))?;
        }
        std::fs::write(&path, content.as_bytes())
            .with_context(|| format!("writing file '{rel}'"))?;
        super::super::dircache::reindex(ctx.workspaces.clone(), ctx.dir_cache.clone());
        super::record_change(ctx, &path, if existed { "modified" } else { "added" });
        let mut result = format!("Wrote {} bytes to {}.", content.len(), rel);
        // L3: auto-neighborhood footer (best-effort, daemon may not be running).
        super::append_neighborhood_footer(&mut result, &rel);
        Ok(result)
    }
}

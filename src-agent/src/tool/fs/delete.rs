//! `delete` tool — delete a workspace-relative file.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use crate::tool::{resolve, Tool, ToolCtx};
use super::helpers::{arg_str, not_found_help};

/// Delete a workspace-relative file.
pub struct Delete;

impl Tool for Delete {
    fn name(&self) -> &'static str { "delete" }
    fn description(&self) -> &'static str {
        "Delete a workspace-relative file."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative file path. When multiple workspace roots are active, file listings prefix paths with [N] (e.g. \"[1]src/main.rs\") — copy that prefix exactly. A bare path with no prefix targets workspace [0]." }
            },
            "required": ["path"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let rel = arg_str(args, "path")?;
        let path = resolve(&ctx.workspaces, rel)?;
        if path.is_dir() {
            bail!("'{rel}' is a directory, not a file");
        }
        if !path.exists() {
            bail!("{}", not_found_help(ctx, &path, rel));
        }
        std::fs::remove_file(&path).with_context(|| format!("deleting file '{rel}'"))?;
        super::super::dircache::reindex(ctx.workspaces.clone(), ctx.dir_cache.clone());
        super::record_change(ctx, &path, "deleted");
        Ok(format!("Deleted {rel}."))
    }
}

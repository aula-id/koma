//! `edit` tool — replace an exact string in a file in place.

use super::helpers::{arg_str, not_found_help};
use crate::tool::{resolve, Tool, ToolCtx};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

/// Replace an exact string in a file in place.
pub struct Edit;

impl Tool for Edit {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &'static str {
        "Replace an exact string in a file in place. Read the file first to get the exact text. Fails if the old string is missing or not unique."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative or absolute path under a configured workspace root. A bare relative path targets workspace [0]." },
                "old": { "type": "string", "description": "Exact substring to replace" },
                "new": { "type": "string", "description": "Replacement text" },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence (default false — requires unique match)"
                }
            },
            "required": ["path", "old", "new"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let rel = arg_str(args, "path")?;
        let old = arg_str(args, "old")?;
        let new = arg_str(args, "new")?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = resolve(&ctx.workspaces, rel)?;
        if path.is_dir() {
            bail!("'{rel}' is a directory, not a file");
        }
        if !path.exists() {
            bail!("{}", not_found_help(ctx, &path, rel));
        }

        let content =
            std::fs::read_to_string(&path).with_context(|| format!("reading file '{rel}'"))?;

        let count = content.matches(old).count();
        if count == 0 {
            return Ok(format!(
                "string not found in {rel}; read the file and copy the exact text"
            ));
        }
        if count > 1 && !replace_all {
            return Ok(format!(
                "the old string appears {count} times in {rel}; add surrounding context to make it unique, or set replace_all=true"
            ));
        }

        let replaced = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };

        // Baseline pre-image BEFORE the rewrite ("virtual git", first-touch-wins).
        super::capture_baseline(ctx, &path);
        std::fs::write(&path, replaced.as_bytes())
            .with_context(|| format!("writing file '{rel}'"))?;
        super::super::dircache::reindex(ctx.workspaces.clone(), ctx.dir_cache.clone());
        // edit is guarded above to only touch an existing file → always a modify.
        super::record_change(ctx, &path, "modified");

        let n = if replace_all { count } else { 1 };
        let mut result = format!("edited {rel} ({n} replacement(s))");
        // L3: auto-neighborhood footer (best-effort, daemon may not be running).
        super::append_neighborhood_footer(&mut result, rel);
        Ok(result)
    }
}

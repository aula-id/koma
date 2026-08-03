//! `read` tool — read a workspace-relative file with line numbers.

use super::helpers::{arg_str, not_found_help};
use crate::tool::{resolve_read, Tool, ToolCtx};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

/// Read a workspace-relative file, returning line-numbered content.
pub struct Read;

impl Tool for Read {
    fn name(&self) -> &'static str {
        "read"
    }
    fn description(&self) -> &'static str {
        "Read a workspace-relative file. Returns line-numbered content. Use offset/limit to paginate large files. \
         For a file's imports and dependents, use graph_query."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative or absolute path under a configured workspace root. A bare relative path targets workspace [0]." },
                "offset": {
                    "type": "integer",
                    "description": "0-based line to start from (default 0)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max lines to read (default 20000, capped at 20000)."
                }
            },
            "required": ["path"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let rel = arg_str(args, "path")?;
        let path = resolve_read(&ctx.workspaces, rel, ctx.session_dir.as_deref(), &ctx.active_skill_dirs)?;
        if path.is_dir() {
            bail!("'{rel}' is a directory, not a file");
        }
        if !path.exists() {
            bail!("{}", not_found_help(ctx, &path, rel));
        }
        let content =
            std::fs::read_to_string(&path).with_context(|| format!("reading file '{rel}'"))?;

        const MAX_LINES: usize = 20_000;
        const MAX_BYTES: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;

        // Parse optional offset/limit; clamp limit to the hard cap.
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| (v as usize).min(MAX_LINES))
            .unwrap_or(MAX_LINES);

        // Collect all lines so we know the total for the notice.
        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        let mut out = String::new();
        let mut bytes = 0usize;
        // `last_emitted_idx` tracks the 0-based index of the last line emitted.
        let mut last_emitted_idx: Option<usize> = None;
        let mut byte_truncated = false;

        for (idx, line) in all_lines.iter().enumerate().skip(offset) {
            if idx >= offset + limit {
                break;
            }
            // 1-indexed line numbers, right-aligned in a 6-wide field.
            let rendered = format!("{:>6}\t{}\n", idx + 1, line);
            if bytes + rendered.len() > MAX_BYTES && last_emitted_idx.is_some() {
                byte_truncated = true;
                break;
            }
            bytes += rendered.len();
            out.push_str(&rendered);
            last_emitted_idx = Some(idx);
        }

        // Determine whether we reached the end of the file.
        // `showed_through` is the 1-based line number of the last line shown.
        let showed_through = last_emitted_idx.map(|i| i + 1).unwrap_or(offset);
        // `next_offset` is what the caller should pass as offset to continue.
        let next_offset = showed_through;
        let reached_end = !byte_truncated && (offset + limit >= total_lines);

        if !reached_end || offset > 0 {
            // Only add the notice when content was cut or we started mid-file.
            let start_line = offset + 1; // 1-based
            out.push_str(&format!(
                "\n[truncated: showing lines {start_line}-{showed_through} of {total_lines}. Use read with offset={next_offset} to continue.]"
            ));
        }
        // L3: auto-neighborhood footer (best-effort, daemon may not be running).
        super::append_neighborhood_footer(&mut out, &path);
        Ok(out)
    }
}

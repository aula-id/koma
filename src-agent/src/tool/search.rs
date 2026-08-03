//! Search tools: grep (regex file search) and glob (file-path pattern match).
//!
//! Both are read-only and safe — they auto-run without approval in Normal mode.
//! Paths are sandboxed via [`super::resolve`]; file walks use the `ignore` crate
//! (gitignore-aware), matching [`super::dircache`].

use super::{resolve_read, Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};

/// Search file contents by regular expression.
pub struct Grep;
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents by regular expression. Returns matching lines as path:line: text. \
         For structural/import dependency queries, prefer graph_query instead."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "A Rust regex pattern to search for."
                },
                "path": {
                    "type": "string",
                    "description": "Workspace-relative or absolute path under a configured workspace root. A bare relative path targets workspace [0]."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob filter for filenames/paths (e.g. '*.rs')."
                }
            },
            "required": ["pattern"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'pattern'"))?;

        // Compile the regex; return a clean error on failure.
        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return Ok(format!("invalid regex: {e}")),
        };

        let search_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let base = resolve_read(&ctx.workspaces, search_path, ctx.session_dir.as_deref(), &ctx.active_skill_dirs)?;

        // Optional glob filter.
        let glob_matcher: Option<globset::GlobMatcher> =
            match args.get("glob").and_then(Value::as_str) {
                Some(g) => {
                    let glob = globset::Glob::new(g)
                        .map_err(|e| anyhow::anyhow!("invalid glob '{g}': {e}"))?;
                    Some(glob.compile_matcher())
                }
                None => None,
            };

        const MAX_MATCHES: usize = 200;
        const MAX_LINE_CHARS: usize = 300;

        let mut matches: Vec<String> = Vec::new();
        let mut truncated = false;

        let walk = ignore::WalkBuilder::new(&base).build();
        'outer: for entry in walk.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let abs_path = entry.path();

            // Apply glob filter against the workspace-relative path.
            if let Some(ref m) = glob_matcher {
                let rel = ctx
                    .workspaces
                    .iter()
                    .enumerate()
                    .find_map(|(i, ws)| abs_path.strip_prefix(ws).ok().map(|r| (i, r)))
                    .map(|(i, r)| {
                        let abs = ctx.workspaces[i].join(r);
                        super::model_display_path(&ctx.workspaces, &abs)
                    });
                match rel {
                    Some(ref r) if !m.is_match(r.as_str()) => continue,
                    None => continue,
                    _ => {}
                }
            }

            // Skip binary files: try reading as UTF-8.
            let content = match std::fs::read_to_string(abs_path) {
                Ok(s) => s,
                Err(_) => continue, // binary or unreadable
            };

            let rel_display = ctx
                .workspaces
                .iter()
                .enumerate()
                .find_map(|(i, ws)| abs_path.strip_prefix(ws).ok().map(|r| (i, r)))
                .map(|(i, r)| {
                    let abs = ctx.workspaces[i].join(r);
                    super::model_display_path(&ctx.workspaces, &abs)
                })
                .unwrap_or_else(|| abs_path.display().to_string());

            for (lineno, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    let display_line = if line.chars().count() > MAX_LINE_CHARS {
                        let truncated_line: String = line.chars().take(MAX_LINE_CHARS).collect();
                        format!("{}…", truncated_line)
                    } else {
                        line.to_string()
                    };
                    matches.push(format!("{}:{}: {}", rel_display, lineno + 1, display_line));
                    if matches.len() >= MAX_MATCHES {
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }

        if matches.is_empty() {
            return Ok("no matches".to_string());
        }
        let mut out = matches.join("\n");
        if truncated {
            out.push_str(
                "\n... (truncated at 200 matches; narrow your pattern or path to see more)",
            );
        }
        Ok(out)
    }
}

/// Find files by glob pattern.
pub struct Glob;
impl Tool for Glob {
    fn name(&self) -> &'static str {
        "glob"
    }
    fn description(&self) -> &'static str {
        "Find files by glob pattern (e.g. **/*.rs). Returns matching paths. \
         For file dependency relationships, prefer graph_query."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match (e.g. '**/*.rs', 'src/**/*.toml')."
                },
                "path": {
                    "type": "string",
                    "description": "Workspace-relative or absolute path under a configured workspace root. A bare relative path targets workspace [0]."
                }
            },
            "required": ["pattern"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'pattern'"))?;

        let base_rel = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let base_abs = resolve_read(&ctx.workspaces, base_rel, ctx.session_dir.as_deref(), &ctx.active_skill_dirs)?;

        let matcher = globset::Glob::new(pattern)
            .map_err(|e| anyhow::anyhow!("invalid glob '{pattern}': {e}"))?
            .compile_matcher();

        const MAX_RESULTS: usize = 200;

        // Check if base is inside a workspace root — if not (e.g. under an
        // active skill dir), we must walk from base_abs directly instead of
        // filtering the workspace dir cache.
        let base_in_workspace = ctx.workspaces.iter().any(|ws| {
            let ws = ws.canonicalize().unwrap_or_else(|_| ws.clone());
            base_abs.starts_with(&ws)
        });

        // Prefer the live dir cache when the base is inside a workspace —
        // it's already gitignore-aware and sorted.
        let cache_files: Vec<String> = if base_in_workspace {
            let cache = ctx
                .dir_cache
                .read()
                .map_err(|_| anyhow::anyhow!("dir cache unavailable"))?;
            cache.files.clone()
        } else {
            Vec::new()
        };

        let mut results: Vec<String> = if base_in_workspace && !cache_files.is_empty() {
            cache_files
                .into_iter()
                .filter(|f| matcher.is_match(f.as_str()))
                .collect()
        } else {
            // Cache empty or base outside workspace: walk from base_abs.
            let mut v: Vec<String> = Vec::new();
            for entry in ignore::WalkBuilder::new(&base_abs).build().flatten() {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    let abs = entry.path();
                    let rel = ctx
                        .workspaces
                        .iter()
                        .enumerate()
                        .find_map(|(i, ws)| abs.strip_prefix(ws).ok().map(|r| (i, r)))
                        .map(|(i, r)| {
                            let abs_path = ctx.workspaces[i].join(r);
                            super::model_display_path(&ctx.workspaces, &abs_path)
                        })
                        .unwrap_or_else(|| abs.display().to_string());
                    if matcher.is_match(rel.as_str()) {
                        v.push(rel);
                    }
                }
            }
            v.sort();
            v
        };

        let truncated = results.len() > MAX_RESULTS;
        results.truncate(MAX_RESULTS);

        if results.is_empty() {
            return Ok("no files match".to_string());
        }
        let mut out = results.join("\n");
        if truncated {
            out.push_str("\n... (truncated at 200 results; narrow the glob to see more)");
        }
        Ok(out)
    }
}

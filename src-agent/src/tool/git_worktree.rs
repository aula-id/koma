//! The `git_worktree` tool: manage git worktrees (list/create/remove/enter/exit).
//!
//! `list`, `create`, and `remove` are pure git operations: they exec git directly and
//! return plain output. They do NOT mutate session state and can complete inline.
//!
//! `enter` and `exit` change the session's working directory (like `cd`) and must be
//! intercepted by the runtime (`process_tools`) before the generic dispatch path.
//! `enter` also pushes the worktree path into `settings.workdir` so it becomes an
//! allowed workspace root for the model. `exit` returns to the primary workdir
//! (the first entry in `settings.workdir`).
//!
//! The runtime interception mirrors the `cd` arm: the tool's `run` method does the
//! pure validation work and returns a sentinel-tagged result on success, which
//! `process_tools` strips and then applies via `apply_workspace_change`.

use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use super::{Tool, ToolCtx};

/// Sentinel prefix emitted by `enter` on success. The runtime strips this prefix,
/// extracts the canonical path, adds it to `settings.workdir` (if not already
/// present), and calls `apply_workspace_change`. The model never sees this prefix.
pub const GIT_WT_ENTER_PREFIX: &str = "__git_wt_enter__::";

/// Sentinel emitted by `exit` on success. The runtime detects this string, resolves
/// the session's primary workdir, and calls `apply_workspace_change` to return there.
pub const GIT_WT_EXIT_PREFIX: &str = "__git_wt_exit__";

/// Manage git worktrees: list, create, remove, and enter/exit.
pub struct GitWorktree;

impl Tool for GitWorktree {
    fn name(&self) -> &'static str {
        "git_worktree"
    }

    fn description(&self) -> &'static str {
        "Manage git worktrees — list/create/remove, and enter/exit. \
         `enter` switches your working directory into the worktree AND adds it to \
         your allowed workspace roots so subsequent tool calls work inside it. \
         `exit` returns you to the primary workdir (the first configured workspace root). \
         `create` and `remove` run git directly and return plain output. \
         After creating a new worktree, use `enter` to switch into it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "remove", "enter", "exit"],
                    "description": "Action to perform: list (show all worktrees), create (add a new worktree), remove (remove a worktree), enter (switch cwd into a worktree and add it to allowed roots), exit (return to the primary workdir)."
                },
                "path": {
                    "type": "string",
                    "description": "Worktree path. Required for create, remove, and enter. For enter: may be absolute or relative to the current workspace; will be canonicalized. For create/remove: passed directly to git."
                },
                "branch": {
                    "type": "string",
                    "description": "Branch or commit to check out in the new worktree (optional, only used by create). If omitted, git picks a new branch name matching the last path component."
                },
                "force": {
                    "type": "boolean",
                    "description": "Pass --force to git worktree remove (optional, only used by remove). Default false. Without --force, git refuses to remove a worktree that has uncommitted changes."
                }
            },
            "required": ["action"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'action'"))?;

        match action {
            "list" => {
                Ok(run_git(&["worktree", "list"], &ctx.workspace, 120_000))
            }

            "create" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required string argument 'path' for create"))?;

                let mut git_args: Vec<&str> = vec!["worktree", "add", path];
                let branch_val;
                if let Some(b) = args.get("branch").and_then(Value::as_str) {
                    branch_val = b.to_string();
                    git_args.push(&branch_val);
                }
                let output = run_git(&git_args, &ctx.workspace, 120_000);

                // Append hint on apparent success (exit code 0 in the output).
                if output.contains("exit code: 0") {
                    Ok(format!(
                        "{output}\nhint: use git_worktree action=\"enter\" path=\"{path}\" to switch into this worktree."
                    ))
                } else {
                    Ok(output)
                }
            }

            "remove" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required string argument 'path' for remove"))?;

                let force = args
                    .get("force")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                let mut git_args: Vec<&str> = vec!["worktree", "remove"];
                if force {
                    git_args.push("--force");
                }
                git_args.push(path);

                Ok(run_git(&git_args, &ctx.workspace, 120_000))
            }

            "enter" => {
                let path_str = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required string argument 'path' for enter"))?;

                // Resolve relative paths against the current workspace.
                let as_path = Path::new(path_str);
                let resolved = if as_path.is_absolute() {
                    as_path.to_path_buf()
                } else {
                    ctx.workspace.join(path_str)
                };

                // Canonicalize to resolve symlinks and `..`.
                let canonical = match resolved.canonicalize() {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(format!(
                            "error: cannot resolve '{}': {e}",
                            resolved.display()
                        ))
                    }
                };

                // Must be an existing directory.
                if !canonical.is_dir() {
                    return Ok(format!(
                        "error: '{}' is not a directory",
                        canonical.display()
                    ));
                }

                // Validate it is a git worktree: a worktree has a `.git` FILE (not a
                // directory — that's the main repo's layout). We accept BOTH:
                // - a `.git` file (linked worktree added via `git worktree add`)
                // - a `.git` directory (main repo clone, which is a valid root too)
                // The key requirement is that a `.git` entry EXISTS.
                if !canonical.join(".git").exists() {
                    return Ok(format!(
                        "error: '{}' is not a git worktree (no .git entry found)",
                        canonical.display()
                    ));
                }

                // Success: return the sentinel prefix + canonical path for the runtime.
                Ok(format!("{GIT_WT_ENTER_PREFIX}{}", canonical.display()))
            }

            "exit" => {
                // No path needed: the runtime resolves the primary workdir itself.
                Ok(GIT_WT_EXIT_PREFIX.to_string())
            }

            _ => Ok(format!("error: unknown action '{action}'; must be one of list, create, remove, enter, exit")),
        }
    }
}

/// Exec `git` directly (no shell wrapper) in `cwd` with the given `args`.
/// Uses the thread + recv_timeout pattern from shell.rs to enforce `timeout_ms`.
/// Returns the combined stdout+stderr, ANSI-stripped, capped, with a trailing
/// `exit code: N` line via [`super::shell::format_captured_output`].
///
/// NOTE: worktree ops are LOCAL — we do NOT inject `GIT_SSH_COMMAND`. That env
/// var is only needed for SSH remote operations and is handled by `git_operator`.
fn run_git(args: &[&str], cwd: &Path, timeout_ms: u64) -> String {
    let child = match Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("error: failed to spawn git: {e}"),
    };

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let timeout = std::time::Duration::from_millis(timeout_ms);
    let output = match rx.recv_timeout(timeout) {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return format!("error: git failed: {e}"),
        Err(_) => return format!("git timed out after {timeout_ms}ms"),
    };

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    // Strip ANSI color codes.
    let combined = crate::dto::chat::strip_ansi(&combined);

    let exit_code = output
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());

    super::shell::format_captured_output(combined, &exit_code)
}

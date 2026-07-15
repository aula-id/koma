//! The `git_worktree` tool: manage git worktrees (list/create/remove/enter/exit).
//!
//! Worktrees live in a SHADOW dir OUTSIDE the repo
//! (`~/.koma/sessions/<pwd_hash>/worktrees/<name>`) so they never clutter the
//! project tree. `create` and `remove` are name-based: the tool computes the shadow
//! path from the session's `worktrees_dir` and runs git from the repo root (never a
//! doomed cwd), then hands the runtime a sentinel so the workspace roots + live cwd
//! stay in sync.
//!
//! `list` is a pure git operation: it execs git directly and returns plain output.
//!
//! `create`, `remove`, `enter`, and `exit` all change the session's working
//! directory and/or allowed roots (like `cd`) and must be intercepted by the runtime
//! (`process_tools`) before the generic dispatch path:
//! - `create` adds the worktree AND auto-enters it (via the enter sentinel), which
//!   pushes the path into `settings.workdir` and switches cwd into it.
//! - `remove` deletes the worktree AND auto-exits (via the remove sentinel), which
//!   de-registers the path and snaps cwd back to the repo root if it was inside.
//! - `enter` pushes the worktree path into `settings.workdir` so it becomes an
//!   allowed workspace root, and switches cwd into it.
//! - `exit` returns to the primary workdir (the first entry in `settings.workdir`).
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

/// Sentinel prefix emitted by `remove` on success (git already ran from the repo
/// root, so the worktree is gone). The runtime strips this prefix, de-registers the
/// path from `settings.workdir`, and — if the session's live cwd was inside the now
/// deleted worktree — snaps it back to the primary workdir. The model never sees it.
pub const GIT_WT_REMOVE_PREFIX: &str = "__git_wt_remove__::";

/// Sentinel prefix emitted by `create` on success (worktree spawned AND entered).
/// Distinct from `GIT_WT_ENTER_PREFIX` so the runtime can emit a confirmation that
/// clearly states BOTH the creation and the entry — weaker models won't misread the
/// result as a failure. The runtime strips this prefix, registers the path in
/// `settings.workdir`, persists, and calls `apply_workspace_change`.
pub const GIT_WT_CREATE_PREFIX: &str = "__git_wt_create__::";

/// Manage git worktrees: list, create, remove, and enter/exit.
pub struct GitWorktree;

impl Tool for GitWorktree {
    fn name(&self) -> &'static str {
        "git_worktree"
    }

    fn description(&self) -> &'static str {
        "Manage git worktrees — list/create/remove, and enter/exit. Worktrees live \
         OUTSIDE your repo in a shadow dir (`~/.koma/sessions/<id>/worktrees/<name>`) \
         so they never clutter the project. \
         `create` takes a `name` (NOT a path): it spawns the worktree under the shadow \
         dir and switches you into it (adds it to your allowed workspace roots too). \
         `remove` takes a `name`: it deletes that worktree and returns you to the repo \
         root. \
         `enter` switches your working directory into an existing worktree AND adds it \
         to your allowed workspace roots so subsequent tool calls work inside it. \
         `exit` returns you to the primary workdir (the first configured workspace root)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "remove", "enter", "exit"],
                    "description": "Action to perform: list (show all worktrees), create (spawn a new worktree under the shadow dir and switch into it), remove (delete a worktree and return to the repo root), enter (switch cwd into an existing worktree and add it to allowed roots), exit (return to the primary workdir)."
                },
                "name": {
                    "type": "string",
                    "description": "The worktree's short name (NOT a path). Required for create and remove. The tool computes the shadow path `~/.koma/sessions/<id>/worktrees/<name>` from it, so a bare name like \"feature-x\" is all you pass."
                },
                "path": {
                    "type": "string",
                    "description": "Worktree path. Required for enter only (use `name` for create/remove). For enter: may be absolute or relative to the current workspace; will be canonicalized."
                },
                "branch": {
                    "type": "string",
                    "description": "Branch or commit to check out in the new worktree (optional, only used by create). If omitted, git picks a new branch name matching the worktree name."
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
                // Run from the session's effective working directory (ctx.workspace,
                // honors `cd`); `git worktree list` is correct from anywhere in the repo.
                let repo_root = ctx.workspace.clone();
                Ok(run_git(&["worktree", "list"], &repo_root, 120_000))
            }

            "create" => {
                // Name-based: spawn the worktree under the shadow dir so it lives
                // OUTSIDE the repo. The tool computes the path from `name`.
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required string argument 'name' for create"))?;

                let base = ctx.worktrees_dir.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("no active session — cannot resolve worktree dir")
                })?;
                let shadow = base.join(name);

                // Ensure the `worktrees/` parent exists before git adds into it.
                if let Err(e) = std::fs::create_dir_all(base) {
                    return Ok(format!(
                        "error: cannot create worktrees dir '{}': {e}",
                        base.display()
                    ));
                }

                // Run git from the session's effective working directory (ctx.workspace,
                // honors `cd`) — consistent with bash / git_operator.
                let repo_root = ctx.workspace.clone();

                let shadow_str = shadow.to_string_lossy().into_owned();
                let mut git_args: Vec<&str> = vec!["worktree", "add", &shadow_str];
                let branch_val;
                if let Some(b) = args.get("branch").and_then(Value::as_str) {
                    branch_val = b.to_string();
                    git_args.push(&branch_val);
                }
                let output = run_git(&git_args, &repo_root, 120_000);

                // On success, auto-enter: hand the runtime the CREATE sentinel so it
                // adds the shadow path to settings.workdir AND switches cwd into it,
                // then emits a clear "created + entered" confirmation to the model.
                if output.contains("exit code: 0") {
                    Ok(format!("{GIT_WT_CREATE_PREFIX}{}", shadow.display()))
                } else {
                    Ok(output)
                }
            }

            "remove" => {
                // Name-based: resolve the same shadow path `create` used.
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required string argument 'name' for remove"))?;

                let base = ctx.worktrees_dir.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("no active session — cannot resolve worktree dir")
                })?;
                let shadow = base.join(name);

                let force = args
                    .get("force")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                // Run git from the session's effective working directory (ctx.workspace,
                // honors `cd`) — consistent with bash / git_operator.
                let repo_root = ctx.workspace.clone();

                let shadow_str = shadow.to_string_lossy().into_owned();
                let mut git_args: Vec<&str> = vec!["worktree", "remove"];
                if force {
                    git_args.push("--force");
                }
                git_args.push(&shadow_str);

                let output = run_git(&git_args, &repo_root, 120_000);

                // On success, auto-exit: hand the runtime the REMOVE sentinel so it
                // de-registers the path and snaps cwd back to the repo root if needed.
                if output.contains("exit code: 0") {
                    Ok(format!("{GIT_WT_REMOVE_PREFIX}{}", shadow.display()))
                } else {
                    // e.g. "contains modified or untracked files, use --force" — the
                    // model can retry with force:true.
                    Ok(output)
                }
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
    // Guard: a dead cwd (e.g. a removed worktree the session's live cwd sat in)
    // would make git fail obscurely — bail with a clear message instead.
    if !cwd.is_dir() {
        return format!("error: git working directory '{}' does not exist", cwd.display());
    }
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");
    // No console flash on Windows — see `tool::shell::no_console_window`'s docs
    // for the `FreeConsole()` causal chain this guards against.
    super::shell::no_console_window(&mut cmd);
    let child = match cmd.spawn() {
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

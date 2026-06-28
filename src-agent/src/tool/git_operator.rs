//! The `git_operator` tool: run git commands in the session workspace.
//!
//! Executes `git` DIRECTLY (no shell wrapper — args are passed as separate argv
//! entries so there is no injection risk). Injects the session's selected SSH key
//! via `GIT_SSH_COMMAND` so SSH remotes use exactly that key without any
//! interactive prompt. Destructive operations are gated behind `confirm_destructive`
//! to prevent accidental data loss. Output is ANSI-stripped and capped to the last
//! [`crate::config::MAX_TOOL_OUTPUT_CHARS`] chars via the shared helper in
//! [`super::shell`].

use anyhow::Result;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use super::{Tool, ToolCtx};

/// Run git commands in the session workspace.
pub struct GitOperator;

impl Tool for GitOperator {
    fn name(&self) -> &'static str {
        "git_operator"
    }

    fn description(&self) -> &'static str {
        "Run a git command in the session workspace. Supports the full git surface: \
         status, add, commit, push, pull, fetch, merge, rebase, branch, checkout, \
         switch, log, diff, stash, tag, remote, reset, cherry-pick, revert, blame, \
         show, restore, clone, init, and more. Uses the session's selected SSH key \
         (set via git_cred) for SSH remotes — the key is injected into GIT_SSH_COMMAND \
         so the correct identity is used without any interactive prompt. \
         Destructive operations (force-push, hard reset, clean -f, branch -D, etc.) \
         require confirm_destructive=true. This tool is NON-INTERACTIVE: git is exec'd \
         directly (never via a shell) and will never prompt for credentials or \
         passphrases — it fails fast instead. \
         IMPORTANT: the FIRST element of 'args' MUST be the git subcommand (e.g. \
         \"push\", \"commit\", \"status\"). Global options that precede the subcommand \
         (e.g. -C, --git-dir) are NOT supported and will be rejected — they are \
         unnecessary because git already runs in the session workspace. Put the \
         subcommand first, e.g. [\"push\", \"origin\", \"main\"]."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The git arguments as an array, e.g. [\"commit\", \"-m\", \"msg\"] or [\"push\", \"origin\", \"main\"]. Do NOT include \"git\" as the first element."
                },
                "confirm_destructive": {
                    "type": "boolean",
                    "description": "Set to true to allow destructive operations (force-push, hard reset, clean -f, branch -D, stash drop/clear, tag -d, filter-branch, gc --prune, etc.). Default false."
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "Timeout in milliseconds (default 120000)."
                }
            },
            "required": ["args"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        // --- Parse args -------------------------------------------------------
        let raw_args = args
            .get("args")
            .ok_or_else(|| anyhow::anyhow!("missing required array argument 'args'"))?;

        let arr = raw_args
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("'args' must be an array of strings"))?;

        if arr.is_empty() {
            return Ok("error: 'args' must not be empty (provide at least the git subcommand)".into());
        }

        let mut git_args: Vec<String> = Vec::with_capacity(arr.len());
        for (i, v) in arr.iter().enumerate() {
            match v.as_str() {
                Some(s) => git_args.push(s.to_string()),
                None => return Ok(format!("error: element at index {i} in 'args' is not a string")),
            }
        }

        // --- Subcommand-first guard (BLOCKER 1) -----------------------------------
        // Global git options (e.g. -C, --git-dir) placed before the subcommand
        // would cause the destructive guardrail to see the flag as the subcmd and
        // skip all checks.  We reject them outright: the session workspace is
        // already the cwd so no global options are needed.
        if git_args[0].starts_with('-') {
            return Ok(format!(
                "error: the first element of 'args' must be the git subcommand, \
                 not a flag/option (got '{}'). Put the subcommand first, \
                 e.g. [\"push\",\"origin\",\"main\"].",
                git_args[0]
            ));
        }

        let confirm_destructive = args
            .get("confirm_destructive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let timeout_ms: u64 = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(120_000);

        // --- Destructive guardrail --------------------------------------------
        if !confirm_destructive {
            if let Some(reason) = check_destructive(&git_args) {
                return Ok(format!(
                    "error: destructive operation detected ({reason}). \
                     Re-run with confirm_destructive=true if this is intentional."
                ));
            }
        }

        // --- SSH key injection ------------------------------------------------
        // Build GIT_SSH_COMMAND if a key is selected; reuse the same home-dir
        // resolution that git_cred uses (dirs::home_dir).
        let ssh_command: Option<String> = if let Some(ref key) = ctx.ssh_key {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot resolve home directory for SSH key path"))?;
            let key_path = home.join(".ssh").join(key);
            Some(format!(
                "ssh -i '{}' -o IdentitiesOnly=yes -o BatchMode=yes",
                key_path.display()
            ))
        } else {
            None
        };

        // --- Spawn git directly (not via sh -c) --------------------------------
        let mut cmd = Command::new("git");
        cmd.args(&git_args)
            .current_dir(&ctx.workspace)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Never allow git to block on an interactive credential prompt.
            .env("GIT_TERMINAL_PROMPT", "0");

        if let Some(ref ssh_cmd) = ssh_command {
            cmd.env("GIT_SSH_COMMAND", ssh_cmd);
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Ok(format!("error: failed to spawn git: {e}")),
        };

        // Wait with timeout using the same thread+channel pattern as shell.rs.
        let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });

        let timeout = std::time::Duration::from_millis(timeout_ms);
        let output = match rx.recv_timeout(timeout) {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Ok(format!("error: git failed: {e}")),
            Err(_) => return Ok(format!("git timed out after {timeout_ms}ms")),
        };

        // --- Format output ----------------------------------------------------
        let mut combined = String::new();
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
        combined.push_str(&String::from_utf8_lossy(&output.stderr));

        // Strip ANSI color codes (git colorizes output by default).
        let combined = crate::dto::chat::strip_ansi(&combined);

        let exit_code_n = output.status.code().unwrap_or(-1);
        let exit_code_str = exit_code_n.to_string();

        // Cap via the shared helper (last MAX_TOOL_OUTPUT_CHARS chars).
        // format_captured_output appends "exit code: N" — do not duplicate it here.
        Ok(super::shell::format_captured_output(combined, &exit_code_str))
    }
}

/// Check whether the git argument list contains a destructive operation that
/// requires explicit confirmation. Returns `Some(description)` when blocked,
/// `None` when safe to proceed.
///
/// Destructive patterns covered:
/// - `push` with `--force` / `-f` / `--force-with-lease` / `--delete` / a
///   colon-prefixed refspec deletion (`:<ref>`)
/// - `reset` with `--hard`
/// - `clean` with any combination of `-f`, `-d`, `-x` (including bundled forms
///   like `-fd`, `-xfd`, etc.)
/// - `branch` with `-D` or with both `-d` and `--force` / `-f`
/// - `checkout`, `switch`, `restore` with `-f` / `--force` / `--discard-changes`
/// - `stash` `drop` or `clear`
/// - `tag` with `-d` / `--delete`
/// - `update-ref` with `-d`
/// - `reflog` `expire` or `delete`
/// - `filter-branch` (always destructive)
/// - `gc` with `--prune`
fn check_destructive(args: &[String]) -> Option<&'static str> {
    // The subcommand is always the first element; flags follow.
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("");

    // Collect all flags (everything after the subcommand) for fast membership
    // testing. We also need to inspect positional args for refspec patterns.
    let rest = &args[1..];

    /// Returns true if any element in `haystack` equals `needle` (exact match).
    fn has_flag(haystack: &[String], needle: &str) -> bool {
        haystack.iter().any(|a| a == needle)
    }

    /// Returns true if any element starts with `prefix` (covers `--flag=value`
    /// forms like `--force-with-lease=<refname>`).
    fn has_prefix(haystack: &[String], prefix: &str) -> bool {
        haystack.iter().any(|a| a.starts_with(prefix))
    }

    /// Returns true if any SHORT-FLAG bundle contains all chars in `chars`.
    /// E.g. `has_bundle_char(args, 'f')` matches `-f`, `-fd`, `-xfd`, etc.
    /// Only considers arguments that start with a single `-` (not `--`).
    fn has_bundle_char(haystack: &[String], ch: char) -> bool {
        haystack.iter().any(|a| {
            a.starts_with('-') && !a.starts_with("--") && a.contains(ch)
        })
    }

    match subcmd {
        "push" => {
            if has_flag(rest, "--force")
                || has_flag(rest, "-f")
                || has_bundle_char(rest, 'f')
                || has_prefix(rest, "--force-with-lease")
                || has_flag(rest, "--delete")
                || has_flag(rest, "-d")
            {
                return Some("push --force / --force-with-lease / --delete");
            }
            // Colon-prefixed refspec deletion: ":refs/heads/foo" or ":main"
            if rest.iter().any(|a| a.starts_with(':')) {
                return Some("push with colon-prefixed refspec deletion");
            }
        }

        "reset" => {
            if has_flag(rest, "--hard") {
                return Some("reset --hard");
            }
        }

        "clean" => {
            // Any combination of -f, -d, -x (bundled or separate) is destructive.
            if has_bundle_char(rest, 'f')
                || has_flag(rest, "--force")
            {
                return Some("clean -f (filesystem wipe)");
            }
        }

        "branch" => {
            // -D (uppercase) is the explicit force-delete shorthand; also catch
            // bundles like -dD.
            if has_flag(rest, "-D") || has_bundle_char(rest, 'D') {
                return Some("branch -D (force delete)");
            }
            // -d + --force or -d + -f is equivalent.
            if (has_flag(rest, "-d") || has_flag(rest, "--delete"))
                && (has_flag(rest, "--force") || has_flag(rest, "-f") || has_bundle_char(rest, 'f'))
            {
                return Some("branch -d --force");
            }
        }

        "checkout" | "switch" | "restore" => {
            if has_flag(rest, "-f")
                || has_flag(rest, "--force")
                || has_flag(rest, "--discard-changes")
                || has_bundle_char(rest, 'f')
            {
                return Some("checkout/switch/restore --force (discards local changes)");
            }
        }

        "stash" => {
            let subcmd2 = rest.first().map(|s| s.as_str()).unwrap_or("");
            if subcmd2 == "drop" || subcmd2 == "clear" {
                return Some("stash drop/clear");
            }
        }

        "tag" => {
            if has_flag(rest, "-d") || has_flag(rest, "--delete") {
                return Some("tag -d (delete tag)");
            }
        }

        "update-ref" => {
            if has_flag(rest, "-d") {
                return Some("update-ref -d (delete ref)");
            }
        }

        "reflog" => {
            let subcmd2 = rest.first().map(|s| s.as_str()).unwrap_or("");
            if subcmd2 == "expire" || subcmd2 == "delete" {
                return Some("reflog expire/delete");
            }
        }

        "filter-branch" => {
            return Some("filter-branch (rewrites history)");
        }

        "gc" => {
            if has_prefix(rest, "--prune") {
                return Some("gc --prune");
            }
        }

        _ => {}
    }

    None
}

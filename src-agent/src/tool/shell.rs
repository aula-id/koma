//! Shell tool: run arbitrary commands in the workspace.
//!
//! `bash` is RISKY — it requires user approval in Normal mode. Its safety
//! relies entirely on the approval gate, not path-sandboxing (unlike the
//! filesystem tools). Output is captured (stdout + stderr) and capped at the
//! last `MAX_TOOL_OUTPUT_CHARS` characters so verbose build output doesn't
//! flood the context.

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock};
use super::{Tool, ToolCtx};

/// Run `command` via `sh -c` in `cwd`, capturing stdout+stderr, and return the
/// combined output: ANSI-stripped, capped to the LAST [`crate::config::MAX_TOOL_OUTPUT_CHARS`]
/// chars, with a trailing `exit code: N` line. Bounded by `timeout_ms` (the child
/// keeps running on a drain thread past the timeout, but the caller is freed with a
/// timeout message so the UI/turn never stalls).
///
/// This is THE shared shell-execution primitive: the model-callable `bash` tool
/// ([`Bash::run`]) and the `!` user-shell shortcut (`app::runtime::actions::chat`)
/// both funnel through here so the output cap, ANSI stripping, and timeout bound can
/// never diverge between them. Capturing-only (no TTY); never panics.
pub fn run_shell_capture(command: &str, cwd: &Path, timeout_ms: u64) -> String {
    // Spawn the child, capturing stdout + stderr.
    let child = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("error: failed to spawn command: {e}"),
    };

    // Wait with timeout using a helper thread + channel.
    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    // `child` is moved into the thread; we get the result back via the channel.
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let timeout = std::time::Duration::from_millis(timeout_ms);
    let output = match rx.recv_timeout(timeout) {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return format!("error: command failed: {e}"),
        Err(_) => {
            // The child thread owns the child now — we can't kill it here, but we
            // still return a timeout message so the caller doesn't stall. The thread
            // drains on its own when the child finishes.
            return format!("command timed out after {timeout_ms}ms");
        }
    };

    // Combine stdout + stderr into one string.
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    // Strip ANSI color codes so git/cargo colorized output doesn't bleed into
    // tool results, history, the transcript, and the rolling summary.
    let combined = crate::dto::chat::strip_ansi(&combined);

    let exit_code = output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
    format_captured_output(combined, &exit_code)
}

/// Format captured command output: ANSI must already be stripped. Applies the
/// shared output cap (last [`crate::config::MAX_TOOL_OUTPUT_CHARS`] chars), adds a
/// truncation notice when trimmed, ensures a trailing newline, and appends
/// `exit code: <code>`. Shared by [`run_shell_capture`] and `git_operator`.
pub(crate) fn format_captured_output(text: String, exit_code: &str) -> String {
    const MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;
    let truncated;
    let tail: String = if text.chars().count() > MAX_CHARS {
        truncated = true;
        text.chars().rev().take(MAX_CHARS).collect::<String>()
            .chars().rev().collect()
    } else {
        truncated = false;
        text
    };

    let mut out = String::new();
    if truncated {
        out.push_str(&format!("... (output truncated to last {MAX_CHARS} chars; redirect to a file and read it if you need the full output)\n"));
    }
    out.push_str(&tail);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("exit code: {exit_code}"));
    out
}

/// Run a shell command in the workspace directory.
pub struct Bash;
impl Tool for Bash {
    fn name(&self) -> &'static str { "bash" }
    fn description(&self) -> &'static str {
        "Run a shell command in the workspace. Use for cargo, build commands, and general shell tasks. \
         For git operations, use the git_operator tool instead — it handles SSH key injection and \
         destructive-operation guards automatically. Output is captured (stdout+stderr)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to run (passed to sh -c)."
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "Timeout in milliseconds (default 120000)."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run the command in the background and return a job id immediately; poll with bash_output, stop with bash_kill. Use for long-running commands. Defaults to false."
                }
            },
            "required": ["command"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let command = args.get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'command'"))?;

        // Redirect git commands to git_operator, which handles SSH key injection
        // and destructive-operation guards. Bash can't do either safely.
        // Match `git` as a command word anywhere in the command (not just at the
        // start), so cheats like `cd /tmp && git status` or `echo $(git log)`
        // are caught too.
        static GIT_RE: OnceLock<Regex> = OnceLock::new();
        let git_re = GIT_RE.get_or_init(|| Regex::new(r"\bgit\b").unwrap());
        if git_re.is_match(command.trim()) {
            return Ok(
                "error: use the git_operator tool for git commands, not bash. \
                 git_operator runs git directly (no shell-injection risk), injects the \
                 session SSH key automatically, and gates destructive operations. \
                 Example: git_operator({\"args\": [\"log\", \"--oneline\", \"-5\"]})".to_string()
            );
        }

        let timeout_ms: u64 = args.get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(120_000);

        // Delegate to the shared primitive so the tool and the `!` user-shell
        // shortcut share the exact same execution, output cap, ANSI strip, and
        // timeout bound. `run` is fallible by trait, but the primitive folds every
        // failure into its returned string, so this is always `Ok`.
        Ok(run_shell_capture(command, &ctx.workspace, timeout_ms))
    }
}

/// Poll a background bash job's status + captured output.
///
/// Like [`Task`](super::task::Task), this tool is advertised to the model but
/// NEVER dispatched through [`Tool::run`]: the runtime
/// (`app::runtime::stream::process_tools`) intercepts a `bash_output` call BEFORE
/// the generic dispatch path, looks up the job in the session's `bash_jobs`
/// registry, and answers synchronously. The `run` impl exists only to satisfy the
/// [`Tool`] trait and must never be reached.
pub struct BashOutput;
impl Tool for BashOutput {
    fn name(&self) -> &'static str { "bash_output" }
    fn description(&self) -> &'static str {
        "Return the current status and captured output of a background bash job \
         (one started by bash with run_in_background=true). Poll this to watch a \
         long-running command's progress."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job id returned when the background command was started (e.g. \"bash-1\")."
                }
            },
            "required": ["job_id"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: bash_output must be handled by the runtime".into())
    }
}

/// Terminate a running background bash job.
///
/// Like [`Task`](super::task::Task), this tool is advertised to the model but
/// NEVER dispatched through [`Tool::run`]: the runtime intercepts a `bash_kill`
/// call BEFORE the generic dispatch path, kills the job (SIGTERM to its child),
/// and answers synchronously. The `run` impl exists only to satisfy the [`Tool`]
/// trait and must never be reached.
pub struct BashKill;
impl Tool for BashKill {
    fn name(&self) -> &'static str { "bash_kill" }
    fn description(&self) -> &'static str {
        "Terminate a running background bash job (one started by bash with \
         run_in_background=true)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job id of the background command to stop (e.g. \"bash-1\")."
                }
            },
            "required": ["job_id"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: bash_kill must be handled by the runtime".into())
    }
}

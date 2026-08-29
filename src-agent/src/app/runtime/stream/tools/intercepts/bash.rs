//! Bash interceptor blocks (`bash` FG + `run_in_background`, `bash_output`,
//! `bash_kill`) — split out of `intercepts.rs` for file size (see the parent
//! module doc for the `InterceptFlow` control-flow contract).
//!
//! Every model `bash` call is handled here as a [`crate::app::bgbash::BashJob`]:
//! - `run_in_background: true` → job with `tool_call_id: None`, immediate result
//! - foreground (default) → job with `tool_call_id: Some(call.id)`, park on
//!   `pending_tool_tasks` until Done or Ctrl+B promote
//!
//! Bash never falls through to `dispatch_deferred` / `capture_raw`.

use std::sync::OnceLock;

use regex::Regex;

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;

use super::InterceptFlow;
use crate::app::runtime::stream::tools::approval::{
    bash_status_line, filter_bash_output, parse_bash_id,
};

/// Same git-word pattern as [`crate::tool::shell::Bash::run`] — keep in sync.
fn git_command_re() -> &'static Regex {
    static GIT_RE: OnceLock<Regex> = OnceLock::new();
    GIT_RE.get_or_init(|| crate::re_util::static_re(r"(?:^|[\s;&|(])git\b"))
}

pub(in crate::app::runtime::stream::tools) fn intercept_bash_background(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let background = args
        .get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timeout_ms: u64 = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(120_000);

    // Shared validation for FG and BG — push an error tool result and continue.
    let err = if command.trim().is_empty() {
        Some("error: bash requires a non-empty 'command'".to_string())
    } else if git_command_re().is_match(command.trim()) {
        Some(
            "error: use the git_operator tool for git commands, not bash. \
             git_operator runs git directly (no shell-injection risk), injects the \
             session SSH key automatically, and gates destructive operations. \
             Example: git_operator({\"args\": [\"log\", \"--oneline\", \"-5\"]})"
                .to_string(),
        )
    } else {
        None
    };
    if let Some(result) = err {
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), result));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Fail-closed: same writable-root gate as inline bash via build_tool_ctx.
    let ctx = crate::app::runtime::stream::spawn::build_tool_ctx(state, sess_idx);
    if ctx.workspaces.is_empty() || ctx.workspace.as_os_str().is_empty() {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: no writable workspace root — SDLC execute/integrate binding \
             missing or invalid; cannot run bash against primary"
                .to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Lazily create THIS session's completion channel once, then reuse it
    // (mirrors the deferred tool-task channel).
    if state.rest.sessions[sess_idx].bash_done_tx.is_none() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.rest.sessions[sess_idx].bash_done_tx = Some(tx);
        state.rest.sessions[sess_idx].bash_done_rx = Some(rx);
    }
    let id = state.rest.sessions[sess_idx].next_bash_id();
    let cwd = ctx.workspace.clone();
    let done_tx = state.rest.sessions[sess_idx].bash_done_tx.clone();

    if background {
        let job = crate::app::bgbash::spawn_bash_job(
            id,
            command,
            cwd,
            done_tx,
            None, // true BG — no park
            None, // no FG timeout
        );
        state.rest.sessions[sess_idx].bash_jobs.push(job);
        crate::app::runtime::bg_persist::persist_bash_jobs(&state.rest.sessions[sess_idx]);
        let result = format!(
            "started background job bash-{id} (running). Poll with \
             bash_output{{\"job_id\":\"bash-{id}\"}}, stop with \
             bash_kill{{\"job_id\":\"bash-{id}\"}}."
        );
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), result));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Foreground: park the turn on this job until Done or Ctrl+B promote.
    let job = crate::app::bgbash::spawn_bash_job(
        id,
        command,
        cwd,
        done_tx,
        Some(call.id.clone()),
        Some(timeout_ms),
    );
    state.rest.sessions[sess_idx].bash_jobs.push(job);
    crate::app::runtime::bg_persist::persist_bash_jobs(&state.rest.sessions[sess_idx]);

    state.rest.sessions[sess_idx]
        .pending_tool_tasks
        .push(call.id.clone());
    state.rest.sessions[sess_idx].awaiting_tool_tasks = true;
    state.rest.sessions[sess_idx].status = format!("running bash-{id}");
    state.rest.sessions[sess_idx].tool_idx += 1;
    // Park: return from process_tools so the resume gate waits on pending_tool_tasks.
    // Mirrors dispatch_deferred's early return for heavy tools.
    InterceptFlow::Return
}

pub(in crate::app::runtime::stream::tools) fn intercept_bash_output(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let tail_lines = args
        .get("tail_lines")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let result = match parse_bash_id(job_id).and_then(|n| {
        state.rest.sessions[sess_idx]
            .bash_jobs
            .iter()
            .find(|j| j.id == n)
    }) {
        Some(job) => {
            let status = job.snapshot_status();
            let line = bash_status_line(&status);
            let out = job.output_snapshot();
            // Only reshape a FINISHED job's output when the model gave
            // no explicit pattern/tail_lines shaping of its own — an
            // explicit ask is deliberate and must be respected as-is.
            // Running/Killed/Error, and any poll with pattern or
            // tail_lines set, take the exact original path below.
            let finished_code = match status {
                crate::app::bgbash::BashJobStatus::Done(code) => Some(code),
                _ => None,
            };
            let saving = state.rest.sessions[sess_idx]
                .session
                .as_ref()
                .map(|s| s.settings.bash_saving)
                .unwrap_or(true);
            let qualifies =
                finished_code.is_some() && tail_lines.is_none() && pattern.is_none() && saving;

            if out.is_empty() {
                format!("{line}\n(no output yet)")
            } else if qualifies {
                // Mirror `tool::shell::finalize_output`'s "saving" path
                // (filter + tee) for a finished background job, same as
                // synchronous bash/git_operator.
                let code = match finished_code {
                    Some(c) => c,
                    None => {
                        crate::model::store::append_global_error_log(
                            "bash",
                            "BUG: finished_code was None after qualifies",
                        );
                        return InterceptFlow::Continue;
                    }
                };
                let (text, should_tee) =
                    crate::app::bgbash::render_finished_output(&job.command, &out, code, saving);
                let mut body = format!("{line}\n{text}");
                if should_tee {
                    let log_dir = state.rest.sessions[sess_idx]
                        .session
                        .as_ref()
                        .map(|s| s.path.join("opt"));
                    if let Some(dir) = log_dir {
                        if let Some(path) = job.ensure_tee_log(&dir, &out) {
                            body.push_str(&format!("\nfull-output: {}", path.display()));
                        }
                    }
                }
                body
            } else {
                match filter_bash_output(&out, tail_lines, pattern) {
                    Ok(filtered) if filtered.is_empty() => format!("{line}\n(no matching lines)"),
                    Ok(filtered) => format!("{line}\n{filtered}"),
                    Err(e) => format!("{line}\n[{e}]"),
                }
            }
        }
        None => format!("error: no such job: {job_id}"),
    };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(in crate::app::runtime::stream::tools) fn intercept_bash_kill(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let result = match parse_bash_id(job_id).and_then(|n| {
        state.rest.sessions[sess_idx]
            .bash_jobs
            .iter()
            .find(|j| j.id == n)
    }) {
        Some(job) => {
            crate::app::bgbash::kill_bash_job(job);
            format!("job bash-{} killed", job.id)
        }
        None => format!("error: no such job: {job_id}"),
    };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

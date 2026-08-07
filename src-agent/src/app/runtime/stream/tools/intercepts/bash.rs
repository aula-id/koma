//! Background-bash interceptor blocks (`bash` with `run_in_background`,
//! `bash_output`, `bash_kill`) — split out of `intercepts.rs` for file size
//! (pure code motion, no behaviour change; see the parent module doc for the
//! `InterceptFlow` control-flow contract every `intercept_*` fn here follows).

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;

use super::InterceptFlow;
use crate::app::runtime::stream::tools::approval::{
    bash_status_line, filter_bash_output, parse_bash_id,
};

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
    if background {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let result = if command.trim().is_empty() {
            "error: bash requires a non-empty 'command'".to_string()
        } else {
            // Fail-closed: same writable-root gate as inline bash via build_tool_ctx.
            let ctx = crate::app::runtime::stream::spawn::build_tool_ctx(state, sess_idx);
            if ctx.workspaces.is_empty() || ctx.workspace.as_os_str().is_empty() {
                "error: no writable workspace root — SDLC execute/integrate binding \
                 missing or invalid; cannot run bash against primary"
                    .to_string()
            } else {
                // Lazily create THIS session's completion channel once, then
                // reuse it (mirrors the deferred tool-task channel). The worker
                // fires the finished job id over `bash_done_tx`; the event-loop
                // deferred drain reads `bash_done_rx` to pop a toast.
                if state.rest.sessions[sess_idx].bash_done_tx.is_none() {
                    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                    state.rest.sessions[sess_idx].bash_done_tx = Some(tx);
                    state.rest.sessions[sess_idx].bash_done_rx = Some(rx);
                }
                let id = state.rest.sessions[sess_idx].next_bash_id();
                // Prefer the sandboxed tool cwd (bound worktree), not raw effective_cwd.
                let cwd = ctx.workspace.clone();
                let done_tx = state.rest.sessions[sess_idx].bash_done_tx.clone();
                let job = crate::app::bgbash::spawn_bash_job(id, command, cwd, done_tx);
                state.rest.sessions[sess_idx].bash_jobs.push(job);
                // Persist the new job record so it survives close/reopen (#25).
                crate::app::runtime::bg_persist::persist_bash_jobs(&state.rest.sessions[sess_idx]);
                format!(
                    "started background job bash-{id} (running). Poll with \
                     bash_output{{\"job_id\":\"bash-{id}\"}}, stop with \
                     bash_kill{{\"job_id\":\"bash-{id}\"}}."
                )
            }
        };
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), result));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    // Not a background bash — fall through to the normal path below.
    InterceptFlow::Fallthrough
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

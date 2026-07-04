//! The `task` tool: delegate work to a background sub-agent.
//!
//! This tool is advertised to the MAIN model so it can hand a self-contained
//! task to a named agent. It is NEVER dispatched through [`Tool::run`]: the
//! runtime (`app::runtime::stream::process_tools`) intercepts a `task` call
//! BEFORE the generic classify/dispatch path, spawns the sub-agent in the
//! background, and returns immediately so the main turn continues. The `run`
//! impl exists only to satisfy the [`Tool`] trait and must never be reached.

use anyhow::Result;
use serde_json::{json, Value};
use super::{Tool, ToolCtx};

/// Delegate a task to a named sub-agent that runs in the background.
pub struct Task;
impl Tool for Task {
    fn name(&self) -> &'static str { "task" }

    fn description(&self) -> &'static str {
        "Delegate a self-contained task to a named specialist sub-agent that runs \
         autonomously to completion and returns its FULL report as this tool's result \
         for you to read and react to. You MAY call this tool MULTIPLE times in a \
         single turn to run several sub-agents IN PARALLEL (up to 5 at once) — each \
         returns its own report. Use it to offload exploration, research, or mechanical \
         edits. The `agent` argument must be one of the sub-agents listed in your \
         system prompt. With run_in_background=true the call returns immediately with \
         a sub-agent id and does NOT block; its full report is delivered to you \
         automatically when it finishes (no polling needed) — poll/stop on demand with \
         task_output/task_kill."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Name of the sub-agent to delegate to (must be one listed under # Sub-agents in your system prompt)."
                },
                "prompt": {
                    "type": "string",
                    "description": "The full task instruction for the sub-agent."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run the sub-agent in the background and return its id immediately; poll with task_output, stop with task_kill. Use for long autonomous tasks you want to run while continuing. Defaults to false."
                }
            },
            "required": ["agent", "prompt"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: task tool must be handled by the runtime".into())
    }
}

/// Poll a background (detached) sub-agent's status + report/transcript.
///
/// Like [`Task`] and [`BashOutput`](super::shell::BashOutput), this tool is
/// advertised to the model but NEVER dispatched through [`Tool::run`]: the runtime
/// (`app::runtime::stream::process_tools`) intercepts a `task_output` call BEFORE
/// the generic dispatch path, looks up the sub-agent in the session's `subagents`
/// registry, and answers synchronously. The `run` impl exists only to satisfy the
/// [`Tool`] trait and must never be reached.
pub struct TaskOutput;
impl Tool for TaskOutput {
    fn name(&self) -> &'static str { "task_output" }
    fn description(&self) -> &'static str {
        "Check the current status/progress of a background sub-agent (one started \
         by task with run_in_background=true). NOTE: when a background sub-agent \
         finishes, its FULL report is delivered to you AUTOMATICALLY — you do NOT \
         need to poll. Use task_output only for an occasional mid-run progress peek \
         if you genuinely need it; never call it in a loop."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": ["integer", "string"],
                    "description": "The sub-agent id returned when the background task was started (e.g. 3)."
                }
            },
            "required": ["id"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: task_output must be handled by the runtime".into())
    }
}

/// Terminate a running background (detached) sub-agent.
///
/// Like [`Task`] and [`BashKill`](super::shell::BashKill), this tool is advertised
/// to the model but NEVER dispatched through [`Tool::run`]: the runtime intercepts
/// a `task_kill` call BEFORE the generic dispatch path, aborts the sub-agent's
/// tokio task, and answers synchronously. The `run` impl exists only to satisfy
/// the [`Tool`] trait and must never be reached.
pub struct TaskKill;
impl Tool for TaskKill {
    fn name(&self) -> &'static str { "task_kill" }
    fn description(&self) -> &'static str {
        "Terminate a running background sub-agent (one started by task with \
         run_in_background=true)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": ["integer", "string"],
                    "description": "The sub-agent id of the background task to stop (e.g. 3)."
                }
            },
            "required": ["id"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: task_kill must be handled by the runtime".into())
    }
}

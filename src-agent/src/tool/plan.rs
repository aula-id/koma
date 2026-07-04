//! The plan-mode tools: `plan_enter` (request read-only Plan mode) and
//! `plan_ready` (present a finished plan for the user's approval).
//!
//! Both mirror the `cd` / `bash_output` pattern: [`Tool::run`] is a stub — the
//! real work happens in the runtime's interception in
//! `app::runtime::stream::tools::approval::process_tools`, BEFORE the generic
//! dispatch path, because a bare `ToolCtx` can't reach `AppState`.
//!
//! - `plan_enter` returns the [`PLAN_ENTER_SENTINEL`]; the interception flips the
//!   mode to `Plan` via `set_agent_mode`.
//! - `plan_ready` carries the whole plan text in its args (too big to round-trip
//!   through a sentinel), so the interception parses the args DIRECTLY (via
//!   [`parse_plan_ready_args`]), writes the plan to `<session>/plan.md`, and
//!   PARKS the round for the user's y/a/n decision. `run` is a never-called stub.

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};

/// Sentinel returned by a successful [`PlanEnter::run`]. The runtime's
/// `plan_enter` interception recognises this exact string and applies the mode
/// switch; the model never sees it (the interception replaces it with a
/// human-readable confirmation). There is no failure case for `plan_enter` — it
/// takes no arguments — so this is the only possible result.
pub const PLAN_ENTER_SENTINEL: &str = "__PLAN_ENTER__";

/// Request to enter read-only Plan mode. See module docs.
pub struct PlanEnter;

impl Tool for PlanEnter {
    fn name(&self) -> &'static str {
        "plan_enter"
    }

    fn description(&self) -> &'static str {
        "Enter plan mode: tools become read-only while you explore and design. Use this \
         when the user asks you to plan, learn, research, explore, or design something \
         before building (e.g. 'plan this', 'learn the codebase', 'research how X works', \
         'design the architecture'). Do not enter plan mode for direct implementation \
         requests, and never for your own convenience - the trigger is the user's intent \
         to plan or understand first."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok(PLAN_ENTER_SENTINEL.to_string())
    }
}

/// Present a finished plan for the user's approval. See module docs — this tool
/// is FULLY INTERCEPTED (`process_tools` parses its args and never calls `run`).
pub struct PlanReady;

impl Tool for PlanReady {
    fn name(&self) -> &'static str {
        "plan_ready"
    }

    fn description(&self) -> &'static str {
        "Present your finished plan for user approval. summary is shown to the user; plan \
         (full detail: files, exact edits, order, risks) is saved to <session>/plan.md. \
         Only call this from plan mode when your plan is complete. Only call this after ALL \
         work is finished - including background bash jobs and sub-agents; collect their \
         results first. The user may approve, approve with history compaction, or ask to \
         keep discussing."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Short user-facing summary of the plan: what will change and the important points"
                },
                "plan": {
                    "type": "string",
                    "description": "The FULL detailed plan: files to touch, exact changes, how and why. This is saved to the session plan.md for execution."
                }
            },
            "required": ["summary", "plan"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: plan_ready must be handled by the runtime".into())
    }
}

/// Pure validation for a `plan_ready` call's arguments, extracted so the
/// interception in `process_tools` can parse them directly (the tool's `run` is
/// a never-called stub). Returns the trimmed `(summary, plan)` on success, or an
/// `error:`-prefixed message string (surfaced to the model verbatim) when either
/// required field is missing or blank.
pub(crate) fn parse_plan_ready_args(args: &Value) -> Result<(String, String), String> {
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: plan_ready requires a non-empty 'summary'")?;
    let plan = args
        .get("plan")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: plan_ready requires a non-empty 'plan'")?;
    Ok((summary.to_string(), plan.to_string()))
}

/// Tool-result text answered back to the model when the user APPROVES a plan
/// (plain, no compaction). Names the on-disk plan so the model can re-read it.
pub(crate) fn plan_approved_text(plan_path: &str) -> String {
    format!(
        "plan approved by user - exit planning and execute it now. Full detail is in \
         {plan_path}; read it if you need to refresh any part."
    )
}

/// Tool-result text answered back to the model when the user APPROVES a plan AND
/// asks to compact history to it. The compaction + plan seed happen in the
/// runtime; this only tells the model what is about to occur.
pub(crate) fn plan_approved_compact_text() -> &'static str {
    "plan approved by user (with history compaction) - context will be compacted to the \
     approved plan; execute it now."
}

/// Tool-result text answered back to the model when the user DENIES a plan and
/// wants to keep discussing. Mode stays Plan.
pub(crate) fn plan_denied_text() -> &'static str {
    "plan not approved - the user wants to keep discussing. Stay in plan mode, take their \
     feedback, revise the plan, and call plan_ready again when ready."
}

#[cfg(test)]
#[path = "plan_test.rs"]
mod tests;

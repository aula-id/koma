//! The SDLC tool suite: `mission_ready`, `mission_verify`, `mission_integrate`.
//!
//! `mission_ready` mirrors `plan_ready`: the tool's `run` is a stub — the real
//! work happens in the runtime interception in `process_tools`, BEFORE the
//! generic dispatch path.
//!
//! `mission_verify` and `mission_integrate` follow the same intercepted stub
//! pattern: the tool defines the schema so the model can call it, but the
//! runtime intercepts the call and does the real work.

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};

/// Sentinel returned by a successful `mission_ready` call (never actually
/// returned — the interception handles everything). Kept for pattern parity.
#[allow(dead_code)]
pub const MISSION_READY_SENTINEL: &str = "__MISSION_READY__";

/// Present a finished SDLC mission for the user's approval.
pub struct MissionReady;

impl Tool for MissionReady {
    fn name(&self) -> &'static str {
        "mission_ready"
    }

    fn description(&self) -> &'static str {
        "Present your finished SDLC mission contract for user approval. Must include goal, \
         acceptance criteria, non_goals, lane, verify_plan, human_gates, risks, rationale, \
         and graph_tasks (array of task titles). Only call this from SDLC mode when your \
         exploration and contract-building is complete."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "highlights": {
                    "type": "string",
                    "description": "The key things the user must know to approve: the important changes, decisions, and risks."
                },
                "goal": {
                    "type": "string",
                    "description": "What this mission achieves."
                },
                "non_goals": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "What this mission explicitly does NOT do."
                },
                "acceptance": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Concrete criteria that must be met for the mission to be considered done."
                },
                "lane": {
                    "type": "string",
                    "description": "Verification lane: express (minimal), standard, or full (thorough)."
                },
                "verify_plan": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Steps to verify correctness after execution."
                },
                "human_gates": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Checkpoints requiring human review."
                },
                "risks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Known risks and mitigations."
                },
                "rationale": {
                    "type": "string",
                    "description": "Why this approach over alternatives."
                },
                "graph_tasks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task titles for the execution checklist."
                }
            },
            "required": ["highlights", "goal", "acceptance", "graph_tasks"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: mission_ready must be handled by the runtime".into())
    }
}

/// Mark verify_bit on a graph node after running verify steps.
pub struct MissionVerify;

impl Tool for MissionVerify {
    fn name(&self) -> &'static str {
        "mission_verify"
    }

    fn description(&self) -> &'static str {
        "Mark verify_bit on a graph node after running verify evidence. \
         Pass evidence describing what was verified (e.g. test output). \
         pass=false reopens the node to active (verify failed)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "The graph node id (e.g. t1, t2)."
                },
                "evidence": {
                    "type": "string",
                    "description": "Evidence of verification (e.g. test output, lint result)."
                },
                "pass": {
                    "type": "boolean",
                    "description": "Whether verification passed (default true). Pass false to reopen."
                }
            },
            "required": ["evidence"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok("error: mission_verify must be handled by the runtime".into())
    }
}

/// Enter integrate phase; merge to main if clean+safe.
pub struct MissionIntegrate;

impl Tool for MissionIntegrate {
    fn name(&self) -> &'static str {
        "mission_integrate"
    }

    fn description(&self) -> &'static str {
        "Enter the integrate phase: merge the mission branch into the primary workdir. \
         If the working tree is dirty, the branch is left ready for manual merge or PR. \
         Never force-pushes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Summary of what was accomplished in this mission."
                },
                "force_branch_only": {
                    "type": "boolean",
                    "description": "If true, skip merge and leave the branch ready for manual integration."
                }
            },
            "required": ["summary"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok("error: mission_integrate must be handled by the runtime".into())
    }
}

/// Parse mission_ready args from the model's tool call. Returns `(highlights,
/// Mission-like fields)` on success, or an error string.
pub(crate) fn parse_mission_ready_args(args: &Value) -> Result<MissionArgs, String> {
    let highlights = args
        .get("highlights")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: mission_ready requires a non-empty 'highlights'")?;
    let goal = args
        .get("goal")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: mission_ready requires a non-empty 'goal'")?;
    let acceptance = string_array(args, "acceptance")
        .filter(|v| !v.is_empty())
        .ok_or("error: mission_ready requires non-empty 'acceptance'")?;
    let graph_tasks = string_array(args, "graph_tasks")
        .filter(|v| !v.is_empty())
        .ok_or("error: mission_ready requires non-empty 'graph_tasks'")?;
    let non_goals = string_array(args, "non_goals").unwrap_or_default();
    let verify_plan = string_array(args, "verify_plan").unwrap_or_default();
    let human_gates = string_array(args, "human_gates").unwrap_or_default();
    let risks = string_array(args, "risks").unwrap_or_default();
    let lane = args
        .get("lane")
        .and_then(Value::as_str)
        .unwrap_or("standard")
        .to_string();
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Ok(MissionArgs {
        highlights: highlights.to_string(),
        goal: goal.to_string(),
        non_goals,
        acceptance,
        lane,
        verify_plan,
        human_gates,
        risks,
        rationale,
        graph_tasks,
    })
}

/// Parse mission_verify args. Returns `(node_id, evidence, pass)` on success.
pub(crate) fn parse_mission_verify_args(
    args: &Value,
) -> Result<(Option<String>, String, bool), String> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let evidence = args
        .get("evidence")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: mission_verify requires a non-empty 'evidence'")?;
    let pass = args.get("pass").and_then(Value::as_bool).unwrap_or(true);
    Ok((node_id, evidence.to_string(), pass))
}

/// Parse mission_integrate args. Returns `(summary, force_branch_only)`.
pub(crate) fn parse_mission_integrate_args(args: &Value) -> Result<(String, bool), String> {
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: mission_integrate requires a non-empty 'summary'")?;
    let force_branch_only = args
        .get("force_branch_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((summary.to_string(), force_branch_only))
}

fn string_array(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
}

/// Parsed mission_ready arguments.
#[derive(Debug)]
pub(crate) struct MissionArgs {
    pub highlights: String,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub acceptance: Vec<String>,
    pub lane: String,
    pub verify_plan: Vec<String>,
    pub human_gates: Vec<String>,
    pub risks: Vec<String>,
    pub rationale: String,
    pub graph_tasks: Vec<String>,
}

/// Tool-result text when user approves a mission.
pub(crate) fn mission_approved_text(body: &str) -> String {
    format!(
        "mission approved by user — execute it now. Full contract below; follow it exactly.\n\n\
         --- APPROVED MISSION ---\n{}\n--- END MISSION ---",
        body.trim()
    )
}

/// Tool-result text when user approves a mission AND wants to compact.
pub(crate) fn mission_approved_compact_text() -> &'static str {
    "mission approved by user (with history compaction) — context will be compacted to the \
     approved mission; execute it now."
}

/// Tool-result text when user denies a mission.
pub(crate) fn mission_denied_text() -> &'static str {
    "mission not approved — the user wants to keep discussing. Stay in SDLC mode, take their \
     feedback, revise the contract, and call mission_ready again when ready."
}

/// Tool-result text for a successful verify.
pub(crate) fn mission_verify_result(node_id: &str, pass: bool) -> String {
    if pass {
        format!("mission_verify: node {node_id} marked verified")
    } else {
        format!("mission_verify: node {node_id} verify failed — reopened to active")
    }
}

/// Tool-result text for a successful integrate.
pub(crate) fn mission_integrate_result(message: &str) -> String {
    format!("mission_integrate: {message}")
}

#[cfg(test)]
#[path = "sdlc_test.rs"]
mod tests;

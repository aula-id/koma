//! The SDLC tool suite: `mission_ready`, `mission_verify`, `mission_prepare`,
//! `mission_integrate`, plus amendment / human-gate helpers parsed by the
//! runtime intercepts.
//!
//! `mission_ready` mirrors `plan_ready`: the tool's `run` is a stub — the real
//! work happens in the runtime interception in `process_tools`, BEFORE the
//! generic dispatch path.

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};
use crate::model::sdlc::graph::ChecklistNode;

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
         and graph_tasks (array of task titles or {title, parent?, id?} objects forming an \
         epic→story→task tree). Only call this from SDLC mode when your exploration and \
         contract-building is complete. Calling again on an approved mission starts the \
         amendment/reapproval path. Optionally pass target_branch to specify which branch \
         the mission merges into on integrate (defaults to current branch at approval time)."
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
                    "items": {
                        "oneOf": [
                            { "type": "string" },
                            {
                                "type": "object",
                                "properties": {
                                    "title": { "type": "string" },
                                    "parent": { "type": "string" },
                                    "id": { "type": "string" },
                                    "owned_paths": {
                                        "type": "array",
                                        "items": { "type": "string" },
                                        "description": "Glob patterns for file paths this node owns. Write/edit/delete to paths matching a DIFFERENT active node's patterns is rejected."
                                    }
                                },
                                "required": ["title"]
                            }
                        ]
                    },
                    "description": "Task titles or {title, parent?, id?} for the execution graph."
                },
                "amendment_note": {
                    "type": "string",
                    "description": "If amending an approved mission, short note of what changed."
                },
                "branch": {
                    "type": "string",
                    "description": "Optional mission branch name. When omitted, a branch is classified from the goal (fix/|feat/|chore/|…). Worktree is still created only on approve."
                },
                "target_branch": {
                    "type": "string",
                    "description": "Optional target branch for integration (where the mission branch merges into). When omitted, defaults to the current branch at approval time. SDLC never auto-merges into main/master — those require manual PR/merge."
                }
            },
            "required": ["highlights", "goal", "acceptance", "graph_tasks"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
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
        "Mark verify_bit on a LEAF graph node after running verify evidence. \
         Pass evidence describing what was verified (e.g. test output). \
         pass=false reopens the leaf and its ancestors. Parents roll up automatically. \
         To request a named human_gate, set human_gate — this PARKS for the user's \
         explicit y/n; the model cannot mark the gate approved itself."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "The leaf graph node id."
                },
                "evidence": {
                    "type": "string",
                    "description": "Evidence of verification (e.g. test output, lint result)."
                },
                "pass": {
                    "type": "boolean",
                    "description": "Whether verification passed (default true). Pass false to reopen."
                },
                "human_gate": {
                    "type": "string",
                    "description": "Request USER approval of a named human_gate from the mission contract. Parks for explicit y/n; the model cannot self-approve. Approval is persisted only after the user accepts."
                }
            },
            "required": ["evidence"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok("error: mission_verify must be handled by the runtime".into())
    }
}

/// Transition from prepare to execute phase after source branch and worktrees
/// are set up.
pub struct MissionPrepare;

impl Tool for MissionPrepare {
    fn name(&self) -> &'static str {
        "mission_prepare"
    }

    fn description(&self) -> &'static str {
        "SDLC-only prepare→execute phase transition. Available only in SDLC prepare phase \
         after mission approval. Confirms source branch and worktree setup is complete. \
         Unrelated to ordinary Plan mode."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Optional note about the prepare phase completion."
                }
            },
            "required": []
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok("error: mission_prepare must be handled by the runtime".into())
    }
}

/// Enter integrate phase; merge into frozen target branch if clean+safe.
pub struct MissionIntegrate;

impl Tool for MissionIntegrate {
    fn name(&self) -> &'static str {
        "mission_integrate"
    }

    fn description(&self) -> &'static str {
        "Enter the integrate phase: merge the mission branch into the frozen target \
         (target_worktree_path + target_branch captured at approval). \
         Requires frozen graph complete, all required leaf evidence verified, valid binding, \
         frozen target present, and approved human gates. Branch-only cannot bypass those gates. \
         If the target working tree is dirty, the branch is left ready for manual merge or PR. \
         Never force-pushes. Never infers destination from live cwd."
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
                    "description": "If true, skip merge and leave the branch ready (still requires evidence gates)."
                }
            },
            "required": ["summary"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok("error: mission_integrate must be handled by the runtime".into())
    }
}

/// Parse mission_ready args from the model's tool call.
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
    let graph_tasks = parse_graph_tasks(args)?;
    if graph_tasks.is_empty() {
        return Err("error: mission_ready requires non-empty 'graph_tasks'".into());
    }
    let non_goals = string_array(args, "non_goals").unwrap_or_default();
    let verify_plan = string_array(args, "verify_plan").unwrap_or_default();
    let human_gates = string_array(args, "human_gates").unwrap_or_default();
    let risks = string_array(args, "risks").unwrap_or_default();
    let lane = args
        .get("lane")
        .and_then(Value::as_str)
        .unwrap_or("standard")
        .trim()
        .to_ascii_lowercase();
    if !matches!(lane.as_str(), "express" | "standard" | "full") {
        return Err("error: lane must be express|standard|full".into());
    }
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let amendment_note = args
        .get("amendment_note")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let branch = match args.get("branch").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Some(
            crate::model::sdlc::branch_name::sanitize_branch_name(s)
                .map_err(|e| format!("error: invalid branch: {e}"))?,
        ),
        _ => None,
    };
    let target_branch = match args.get("target_branch").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Some(
            crate::model::sdlc::branch_name::sanitize_branch_name(s)
                .map_err(|e| format!("error: invalid target_branch: {e}"))?,
        ),
        _ => None,
    };

    // Lane / anti-megatask validation.
    crate::model::sdlc::decompose::validate_lane_graph(&lane, &graph_tasks, acceptance.len())?;

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
        amendment_note,
        branch,
        target_branch,
    })
}

fn parse_graph_tasks(args: &Value) -> Result<Vec<ChecklistNode>, String> {
    let arr = args
        .get("graph_tasks")
        .and_then(Value::as_array)
        .ok_or("error: mission_ready requires non-empty 'graph_tasks'")?;
    let mut out = Vec::new();
    for (i, v) in arr.iter().enumerate() {
        if let Some(s) = v.as_str() {
            let t = s.trim();
            if t.is_empty() {
                continue;
            }
            out.push(ChecklistNode {
                title: t.to_string(),
                status: "pending".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            });
        } else if let Some(obj) = v.as_object() {
            let title = obj
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("error: graph_tasks[{i}] object needs non-empty title"))?
                .to_string();
            let parent_title = obj
                .get("parent")
                .or_else(|| obj.get("parent_title"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let id = obj
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let owned_paths: Vec<String> = obj
                .get("owned_paths")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.push(ChecklistNode {
                title,
                status: "pending".into(),
                parent_title,
                id,
                owned_paths,
            });
        } else {
            return Err(format!(
                "error: graph_tasks[{i}] must be a string or {{title, parent?}} object"
            ));
        }
    }
    Ok(out)
}

/// Parse mission_verify args. Returns `(node_id, evidence, pass, human_gate)`.
pub(crate) fn parse_mission_verify_args(
    args: &Value,
) -> Result<(Option<String>, String, bool, Option<String>), String> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let human_gate = args
        .get("human_gate")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let evidence = args
        .get("evidence")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("error: mission_verify requires a non-empty 'evidence'")?;
    let pass = args.get("pass").and_then(Value::as_bool).unwrap_or(true);
    Ok((node_id, evidence.to_string(), pass, human_gate))
}

/// Parse mission_prepare args. Returns the optional `note`.
pub(crate) fn parse_mission_prepare_args(args: &Value) -> Result<Option<String>, String> {
    let note = args
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(note)
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
    pub graph_tasks: Vec<ChecklistNode>,
    pub amendment_note: Option<String>,
    /// Optional user-requested mission branch (sanitized). When None at ready,
    /// classifier fills intent before approve bind.
    pub branch: Option<String>,
    /// Optional user-requested target integration branch. When None at ready,
    /// `establish_mission_binding` captures current_git_branch at approval time.
    pub target_branch: Option<String>,
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

/// Tool-result text when approve failed closed (binding).
pub(crate) fn mission_binding_failed_text(detail: &str) -> String {
    format!(
        "mission NOT approved — worktree/branch binding failed ({detail}). \
         Stay in assess, fix git/worktree, call mission_ready again."
    )
}

/// Tool-result text for a successful verify.
pub(crate) fn mission_verify_result(node_id: &str, pass: bool) -> String {
    if pass {
        format!("mission_verify: leaf {node_id} marked verified (parents rolled up if complete)")
    } else {
        format!("mission_verify: leaf {node_id} verify failed — reopened leaf + ancestors")
    }
}

/// Tool-result text for a successful prepare transition.
pub(crate) fn mission_prepare_result(note: &str) -> String {
    if note.is_empty() {
        "mission_prepare: prepare phase complete, transitioning to execute".into()
    } else {
        format!("mission_prepare: prepare phase complete, transitioning to execute ({note})")
    }
}

/// Tool-result text for a successful integrate.
pub(crate) fn mission_integrate_result(message: &str) -> String {
    format!("mission_integrate: {message}")
}

#[cfg(test)]
#[path = "sdlc_test.rs"]
mod tests;

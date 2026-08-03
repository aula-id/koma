//! The `skill` tool: load/unload/list Agent Skills into the session context.

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};

/// Sentinel prefix on a successful `load` result. The runtime's skill interception
/// recognises this, stores the body in `active_skills`, and surfaces confirmation
/// to the model.
pub const SKILL_LOAD_PREFIX: &str = "__skill_load__::";
/// Sentinel prefix on a successful `unload` result.
pub const SKILL_UNLOAD_PREFIX: &str = "__skill_unload__::";

/// Load, unload, or list Agent Skills.
pub struct Skill;

impl Tool for Skill {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn description(&self) -> &'static str {
        "Load or unload an Agent Skill into the session context. Skills are \
         catalogues of name+description in the system prompt; full bodies are \
         only injected after load. action=\"load\" loads the body for this and \
         later turns; action=\"unload\" removes it; action=\"list\" shows \
         available skills and which are active. Dir-form skills (bar/SKILL.md) \
         list companion files in the load result — use `read` with absolute \
         paths under skill_dir to access them."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "load, unload, or list",
                    "enum": ["load", "unload", "list"]
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (for load/unload)"
                }
            },
            "required": ["action"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("list");

        match action {
            "load" => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required 'name' for load"))?
                    .trim()
                    .to_lowercase();

                let skill = ctx
                    .skill_registry
                    .as_ref()
                    .and_then(|r| r.get(&name))
                    .ok_or_else(|| anyhow::anyhow!("unknown skill: {name}"))?;

                // Read body fresh from disk.
                let raw = std::fs::read_to_string(&skill.file_path)
                    .map_err(|e| anyhow::anyhow!("failed to read skill '{}': {e}", skill.name))?;
                let (_, body) = crate::model::agent_def::split_frontmatter(&raw)?;
                let body = body.trim().to_string();

                // Sentinel: intercept stores body in active_skills.
                Ok(format!("{SKILL_LOAD_PREFIX}{name}\n{body}"))
            }
            "unload" => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required 'name' for unload"))?
                    .trim()
                    .to_lowercase();
                Ok(format!("{SKILL_UNLOAD_PREFIX}{name}"))
            }
            "list" => {
                let active = ctx.active_skill_names.as_deref().unwrap_or(&[]);
                match ctx.skill_registry.as_ref() {
                    Some(reg) if !reg.is_empty() => {
                        let mut lines: Vec<String> = Vec::new();
                        lines.push("Available skills:".to_string());
                        for s in reg.list() {
                            let marker = if active.contains(&s.name) {
                                " [ACTIVE]"
                            } else {
                                ""
                            };
                            lines.push(format!("- {}{}: {}", s.name, marker, s.description));
                        }
                        Ok(lines.join("\n"))
                    }
                    _ => Ok("No skills found.".to_string()),
                }
            }
            other => Err(anyhow::anyhow!("unknown action: {other}")),
        }
    }
}

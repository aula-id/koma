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

/// Result when `load` is asked for a skill whose body is already in context.
/// Not a load sentinel, so the runtime must not replace the stored body.
pub fn already_active_message(name: &str) -> String {
    format!(
        "skill '{name}' is already active — its body is already in context. Do not load it again."
    )
}

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
         later turns. If the skill is already active, load does not read the \
         file or replace the body. action=\"unload\" removes it; action=\"list\" \
         shows available skills and marks active ones with [ACTIVE]. Dir-form \
         skills (bar/SKILL.md) list companion files in the load result — use \
         `read` with absolute paths under skill_dir to access them."
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
        let action = args.get("action").and_then(Value::as_str).unwrap_or("list");

        match action {
            "load" => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required 'name' for load"))?
                    .trim()
                    .to_lowercase();

                // Already in the session tail as "# Skill: <name>". A second load
                // must not re-read disk or replace that body. Reload from disk is
                // a separate panel action.
                if ctx
                    .active_skill_names
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .any(|active| active == &name)
                {
                    return Ok(already_active_message(&name));
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;

    fn ctx(active: Option<Vec<String>>) -> ToolCtx {
        ToolCtx {
            plan_read_only: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            workspace: std::path::PathBuf::from("."),
            workspaces: vec![std::path::PathBuf::from(".")],
            dir_cache: std::sync::Arc::new(
                std::sync::RwLock::new(crate::tool::DirCache::default()),
            ),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            scratch_dir: None,
            internet_mode: crate::model::settings::InternetMode::default(),
            ssh_key: None,
            skill_registry: None,
            active_skill_names: active,
            active_skill_dirs: Vec::new(),
            mcp_manager: None,
            sec_manager: None,
            bash_saving: true,
            bash_log_dir: None,
            session_dir: None,
            allow_scratch: true,
            sdlc_assess: false,
            sdlc_active_node_id: None,
            search_engine: None,
            call_track: crate::tool::CallTrack::new(),
        }
    }

    #[test]
    fn load_of_an_active_skill_does_not_reread_or_reinject() {
        let ctx = ctx(Some(vec!["te33".to_string()]));
        let result = Skill
            .run(&ctx, &json!({"action": "load", "name": "TE33"}))
            .unwrap();
        assert_eq!(result, already_active_message("te33"));
        assert!(!result.starts_with(SKILL_LOAD_PREFIX));
    }
}

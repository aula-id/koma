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
/// Sentinel prefix on a successful `create` result. The runtime strips it and
/// rebuilds the skill catalogue before the model sees the message.
pub const SKILL_CREATE_PREFIX: &str = "__skill_create__::";
/// Sentinel prefix on a successful `modify` result. The runtime strips it,
/// rebuilds the catalogue, and refreshes the body if that skill is loaded.
pub const SKILL_MODIFY_PREFIX: &str = "__skill_modify__::";

/// Result when `load` is asked for a skill whose body is already in context.
/// Not a load sentinel, so the runtime must not replace the stored body.
pub fn already_active_message(name: &str) -> String {
    format!(
        "skill '{name}' is already active — its body is already in context. Do not load it again."
    )
}

/// Load, unload, list, or create Agent Skills.
pub struct Skill;

impl Tool for Skill {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn description(&self) -> &'static str {
        "Load, unload, list, create, or modify an Agent Skill. action=\"load\" \
         injects the body for this and later turns. If the skill is already \
         active, load does not read the file or replace the body. \
         action=\"unload\" removes it. action=\"list\" shows skills and marks \
         active ones with [ACTIVE]. action=\"create\" makes a new skill. \
         action=\"modify\" changes an existing skill. Do not use write or edit \
         for skill files; those folders are outside the workspace. create needs \
         name, description, and instruction. modify needs name plus description \
         or instruction (omit one to keep it). scope (or location) is project \
         (default) or global, for create only. Only the main agent chat calls \
         create and modify. Dir-form skills list companion files in the load \
         result — use `read` with absolute paths under skill_dir."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "load, unload, list, create, or modify",
                    "enum": ["load", "unload", "list", "create", "modify"]
                },
                "name": {
                    "type": "string",
                    "description": "Skill name. Required for load, unload, create, and modify. Lowercase words and hyphens, like review-notes."
                },
                "description": {
                    "type": "string",
                    "description": "create: required. modify: new one-line list text. Omit on modify to keep the current line."
                },
                "instruction": {
                    "type": "string",
                    "description": "create: required. modify: new steps. Omit on modify to keep the current steps."
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "create only. Where to save: project (default) or global. location is accepted as the same field."
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
            "create" => create_skill(ctx, args),
            "modify" => modify_skill(ctx, args),
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

fn create_skill(ctx: &ToolCtx, args: &Value) -> Result<String> {
    let name = required_text(args, "name")?;
    let description = required_text(args, "description")?;
    let instruction = required_text(args, "instruction")?;
    let scope = args
        .get("scope")
        .or_else(|| args.get("location"))
        .and_then(Value::as_str)
        .unwrap_or("project")
        .trim()
        .to_lowercase();
    let target = match scope.as_str() {
        "" | "project" => crate::model::skill::OwnedSkillTarget::Project,
        "global" => crate::model::skill::OwnedSkillTarget::Global,
        _ => anyhow::bail!("scope must be \"project\" or \"global\""),
    };
    let workdir = match target {
        crate::model::skill::OwnedSkillTarget::Project => Some(project_root(ctx)?),
        crate::model::skill::OwnedSkillTarget::Global => None,
    };
    let path = crate::model::skill::create_owned_skill(
        workdir,
        target,
        name,
        &crate::model::skill::SkillEdit {
            description: description.to_string(),
            triggers: String::new(),
            allowed_tools: Vec::new(),
            instruction: instruction.to_string(),
        },
    )?;
    let shown = path.to_string_lossy().replace('\\', "/");
    let scope_label = if matches!(target, crate::model::skill::OwnedSkillTarget::Global) {
        "global"
    } else {
        "project"
    };
    Ok(format!(
        "{SKILL_CREATE_PREFIX}Created {scope_label} skill '{name}'.\nPath: {shown}\nIt is not loaded yet. Next, call skill({{\"action\":\"load\",\"name\":\"{name}\"}})."
    ))
}

fn modify_skill(ctx: &ToolCtx, args: &Value) -> Result<String> {
    let name = required_text(args, "name")?.to_lowercase();
    let description = optional_text(args, "description");
    let instruction = optional_text(args, "instruction");
    if description.is_none() && instruction.is_none() {
        anyhow::bail!("modify needs description or instruction");
    }
    let registry = ctx
        .skill_registry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("unknown skill: {name}"))?;
    let skill = registry
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown skill: {name}"))?;
    let description = description.unwrap_or(skill.description.as_str());
    let instruction = instruction.unwrap_or(skill.body.as_str());
    let path = crate::model::skill::update_owned_skill(
        registry,
        &skill.skill_id,
        &skill.generation,
        &skill.name,
        &crate::model::skill::SkillEdit {
            description: description.to_string(),
            triggers: skill.triggers.clone(),
            allowed_tools: skill.allowed_tools.clone(),
            instruction: instruction.to_string(),
        },
    )?;
    let shown = path.to_string_lossy().replace('\\', "/");
    Ok(format!(
        "{SKILL_MODIFY_PREFIX}{name}\nUpdated skill '{name}'.\nPath: {shown}\nIf it is already loaded, the new text replaces the old one. Otherwise call skill({{\"action\":\"load\",\"name\":\"{name}\"}})."
    ))
}

fn optional_text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn required_text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing {key}"))
}

fn project_root(ctx: &ToolCtx) -> Result<&std::path::Path> {
    ctx.workspaces
        .iter()
        .find(|path| !path.as_os_str().is_empty())
        .map(std::path::PathBuf::as_path)
        .or_else(|| {
            if ctx.workspace.as_os_str().is_empty() {
                None
            } else {
                Some(ctx.workspace.as_path())
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!("No project is open. Use scope \"global\", or open a project.")
        })
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

    #[test]
    fn skill_create_defaults_to_the_project_folder() {
        let root = std::env::temp_dir().join(format!("koma-skill-maker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut ctx = ctx(None);
        ctx.workspace = root.clone();
        ctx.workspaces = vec![root.clone()];
        let result = Skill
            .run(
                &ctx,
                &json!({
                    "action": "create",
                    "name": "wdym",
                    "description": "Explain a short confusing phrase.",
                    "instruction": "Restate the user's last message in plain words."
                }),
            )
            .unwrap();
        assert!(result.starts_with(SKILL_CREATE_PREFIX));
        let result = result.trim_start_matches(SKILL_CREATE_PREFIX);
        let entry = root.join(".agents/skills/wdym/SKILL.md");
        assert!(entry.is_file(), "{result}");
        assert!(result.contains("Created project skill 'wdym'."));
        assert!(result.contains("skill({\"action\":\"load\",\"name\":\"wdym\"})"));
        let body = std::fs::read_to_string(&entry).unwrap();
        assert!(body.contains("description: Explain a short confusing phrase."));
        assert!(body.contains("Restate the user's last message"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skill_create_rejects_a_bad_scope_without_writing() {
        let ctx = ctx(None);
        let error = Skill
            .run(
                &ctx,
                &json!({
                    "action": "create",
                    "name": "wdym",
                    "description": "Explain.",
                    "instruction": "Explain.",
                    "scope": "home"
                }),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("project"));
        assert!(error.contains("global"));
    }

    #[test]
    fn skill_modify_changes_the_instruction_and_keeps_the_description() {
        let root = std::env::temp_dir().join(format!("koma-skill-modify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut ctx = ctx(None);
        ctx.workspace = root.clone();
        ctx.workspaces = vec![root.clone()];
        Skill
            .run(
                &ctx,
                &json!({
                    "action": "create",
                    "name": "wdym",
                    "description": "Explain a short confusing phrase.",
                    "instruction": "Restate the last message."
                }),
            )
            .unwrap();
        ctx.skill_registry = Some(crate::model::skill::SkillRegistry::load(Some(&root), &[]));
        let result = Skill
            .run(
                &ctx,
                &json!({
                    "action": "modify",
                    "name": "wdym",
                    "instruction": "Restate the last message, then give one example."
                }),
            )
            .unwrap();
        assert!(result.starts_with(SKILL_MODIFY_PREFIX));
        let body = std::fs::read_to_string(root.join(".agents/skills/wdym/SKILL.md")).unwrap();
        assert!(body.contains("description: Explain a short confusing phrase."));
        assert!(body.contains("then give one example"));
        assert!(!body.contains("Restate the last message.\n"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

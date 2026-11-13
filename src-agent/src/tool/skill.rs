//! The skill tools: find, load, unload, create and modify Agent Skills. The catalogue is never
//! listed to the model; it asks `skill_find` for what it needs.

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

/// How many hits one `skill_find` returns by default, and the most it will return.
const FIND_DEFAULT: usize = 6;
const FIND_MAX: usize = 10;

fn name_arg(args: &Value, what: &str) -> Result<String> {
    args.get("name")
        .and_then(Value::as_str)
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required 'name' for {what}"))
}

/// What the model gets back from `skill_find`. The catalogue is never listed: a request goes in, a few
/// matches come out.
fn find_skills(ctx: &ToolCtx, args: &Value) -> Result<String> {
    use crate::model::skill::search::decisive;
    let query = optional_text(args, "query").ok_or_else(|| anyhow::anyhow!("missing required 'query' for skill_find: say what you need the skill for"))?;
    let Some(reg) = ctx.skill_registry.as_ref().filter(|r| !r.is_empty()) else {
        return Ok("No skills are installed.".to_string());
    };
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = (args.get("limit").and_then(Value::as_u64).unwrap_or(FIND_DEFAULT as u64) as usize).clamp(1, FIND_MAX);
    let scope = ctx.skill_scope.clone().unwrap_or_default();
    // A wider pool when a model re-rank may reorder it; plain paging otherwise.
    let pool = if offset == 0 && ctx.skill_rank.is_some() { 20 } else { limit };
    let (mut hits, total) = reg.search(query, &scope, offset, pool);
    if offset == 0 && !decisive(&hits) {
        if let Some(rank) = ctx.skill_rank.as_ref() {
            let cands: Vec<(String, String)> = hits
                .iter()
                .filter_map(|h| reg.get(&h.name).map(|d| (d.name.clone(), d.description.clone())))
                .collect();
            if let Some(order) = rank(query, &cands) {
                let mut reordered: Vec<_> = order.iter().filter_map(|n| hits.iter().find(|h| &h.name == n).cloned()).collect();
                for h in &hits {
                    if !reordered.iter().any(|r| r.name == h.name) {
                        reordered.push(h.clone());
                    }
                }
                hits = reordered;
            }
        }
    }
    hits.truncate(limit);
    if hits.is_empty() {
        return Ok(format!("No skill matches \"{query}\". Try other words for what the skill would be about."));
    }
    let active = ctx.active_skill_names.as_deref().unwrap_or(&[]);
    let mut out = format!("{total} skill(s) match \"{query}\"; showing {}-{}:\n", offset + 1, offset + hits.len());
    for h in &hits {
        if let Some(d) = reg.get(&h.name) {
            let marker = if active.contains(&d.name) { " [ACTIVE]" } else { "" };
            out.push_str(&format!("- {}{}: {}\n", d.name, marker, d.description));
        }
    }
    if offset + hits.len() < total {
        out.push_str(&format!("More: call skill_find again with offset {}.\n", offset + hits.len()));
    }
    out.push_str("Load one with skill_load.");
    Ok(out)
}

fn load_skill(ctx: &ToolCtx, args: &Value) -> Result<String> {
    let name = name_arg(args, "skill_load")?;
    // Already in the session tail as "# Skill: <name>". A second load must not re-read disk or replace
    // that body. Reload from disk is a separate panel action.
    if ctx.active_skill_names.as_deref().unwrap_or(&[]).iter().any(|active| active == &name) {
        return Ok(already_active_message(&name));
    }
    if let Some(scope) = ctx.skill_scope.as_ref() {
        if !scope.allows(&name) {
            anyhow::bail!("skill '{name}' is not available to you; use skill_find to see what is");
        }
    }
    let skill = ctx
        .skill_registry
        .as_ref()
        .and_then(|r| r.get(&name))
        .ok_or_else(|| anyhow::anyhow!("unknown skill: {name}; use skill_find to look for it"))?;
    // Read body fresh from disk.
    let raw = std::fs::read_to_string(&skill.file_path).map_err(|e| anyhow::anyhow!("failed to read skill '{}': {e}", skill.name))?;
    let (_, body) = crate::model::agent_def::split_frontmatter(&raw)?;
    // Sentinel: intercept stores body in active_skills.
    Ok(format!("{SKILL_LOAD_PREFIX}{name}\n{}", body.trim()))
}

macro_rules! skill_tool {
    ($ty:ident, $name:literal, $desc:expr, $params:expr, $run:expr) => {
        pub struct $ty;
        impl Tool for $ty {
            fn name(&self) -> &'static str {
                $name
            }
            fn description(&self) -> &'static str {
                $desc
            }
            fn parameters(&self) -> Value {
                $params
            }
            fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
                $run(ctx, args)
            }
        }
    };
}

skill_tool!(
    SkillFind,
    "skill_find",
    "Find an Agent Skill by what you need it for. Skills are not listed; describe the task in a few words \
     (\"review how JWT tokens are checked\") and get the best matches, each with a one-line description. \
     Use it before specialised work, then skill_load the one that fits. Page with offset for more.",
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "What you need the skill for, in a few words."},
            "limit": {"type": "integer", "description": "Matches to return, 1 to 10. Default 6."},
            "offset": {"type": "integer", "description": "Skip this many matches, to page."}
        },
        "required": ["query"]
    }),
    find_skills
);

skill_tool!(
    SkillLoad,
    "skill_load",
    "Load one Agent Skill by name (from skill_find) so its steps are in context for this and later turns. \
     If it is already active nothing is re-read. Dir-form skills list companion files in the result; \
     use `read` with absolute paths under skill_dir.",
    json!({
        "type": "object",
        "properties": {"name": {"type": "string", "description": "Skill name, like review-notes."}},
        "required": ["name"]
    }),
    load_skill
);

skill_tool!(
    SkillUnload,
    "skill_unload",
    "Unload a skill that is done, so its body leaves the context.",
    json!({
        "type": "object",
        "properties": {"name": {"type": "string", "description": "Skill name to unload."}},
        "required": ["name"]
    }),
    |_ctx: &ToolCtx, args: &Value| -> Result<String> {
        let name = name_arg(args, "skill_unload")?;
        Ok(format!("{SKILL_UNLOAD_PREFIX}{name}"))
    }
);

skill_tool!(
    SkillCreate,
    "skill_create",
    "Create a new Agent Skill. Needs name, description (one line used to find it) and instruction (the \
     steps). scope is project (default) or global. Do not use write or edit for skill files; those folders \
     are outside the workspace. Only the main agent chat calls it. It is not loaded until skill_load.",
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Lowercase words and hyphens, like review-notes."},
            "description": {"type": "string", "description": "One line saying when to use it."},
            "instruction": {"type": "string", "description": "The steps."},
            "scope": {"type": "string", "enum": ["project", "global"], "description": "Where to save. Default project."}
        },
        "required": ["name", "description", "instruction"]
    }),
    create_skill
);

skill_tool!(
    SkillModify,
    "skill_modify",
    "Change an existing Agent Skill. Needs name plus description or instruction (omit one to keep it). \
     Do not use write or edit for skill files. Only the main agent chat calls it.",
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Skill name."},
            "description": {"type": "string", "description": "New one-line text. Omit to keep."},
            "instruction": {"type": "string", "description": "New steps. Omit to keep."}
        },
        "required": ["name"]
    }),
    modify_skill
);

/// The names of the skill tools the model is shown.
#[cfg(test)]
pub const SKILL_TOOLS: [&str; 5] = ["skill_find", "skill_load", "skill_unload", "skill_create", "skill_modify"];

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
        "{SKILL_CREATE_PREFIX}Created {scope_label} skill '{name}'.\nPath: {shown}\nIt is not loaded yet. Next, call skill_load with name \"{name}\"."
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
        "{SKILL_MODIFY_PREFIX}{name}\nUpdated skill '{name}'.\nPath: {shown}\nIf it is already loaded, the new text replaces the old one. Otherwise call skill_load with name \"{name}\"."
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
            skill_scope: None,
            skill_rank: None,
            mcp_manager: None,
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
        let result = SkillLoad
            .run(&ctx, &json!({"name": "TE33"}))
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
        let result = SkillCreate
            .run(
                &ctx,
                &json!({
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
        assert!(result.contains("skill_load with name \"wdym\""));
        let body = std::fs::read_to_string(&entry).unwrap();
        assert!(body.contains("description: Explain a short confusing phrase."));
        assert!(body.contains("Restate the user's last message"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skill_create_rejects_a_bad_scope_without_writing() {
        let ctx = ctx(None);
        let error = SkillCreate
            .run(
                &ctx,
                &json!({
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
        SkillCreate
            .run(
                &ctx,
                &json!({
                    "name": "wdym",
                    "description": "Explain a short confusing phrase.",
                    "instruction": "Restate the last message."
                }),
            )
            .unwrap();
        ctx.skill_registry = Some(crate::model::skill::SkillRegistry::load(Some(&root), &[]));
        let result = SkillModify
            .run(
                &ctx,
                &json!({
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

    #[test]
    fn the_model_is_shown_the_five_skill_tools_and_no_skill_tool() {
        let names = crate::tool::main_tool_names();
        for n in SKILL_TOOLS {
            assert!(names.iter().any(|x| x == n), "{n} is not advertised");
        }
        assert!(!names.iter().any(|x| x == "skill"), "the old skill tool is still advertised");
        assert!(crate::tool::DEFERRED_TOOLS.contains(&"skill_find"));
        assert!(crate::tool::tool_allowed_in_plan("skill_load"));
        for n in SKILL_TOOLS {
            let tool = crate::tool::all_tools().into_iter().find(|t| t.name() == n).unwrap();
            assert!(!tool.parameters().to_string().contains("\"list\""), "{n} offers a list");
        }
    }

    fn registry_of(n: usize) -> crate::model::skill::SkillRegistry {
        let root = std::env::temp_dir().join(format!("koma-skillfind-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(".agents/skills");
        for i in 0..n {
            let d = dir.join(format!("filler-{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("SKILL.md"), format!("---\ndescription: Generic helper {i} for odd chores.\n---\nbody {i}")).unwrap();
        }
        let d = dir.join("jwt-review");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), "---\ndescription: Review how JSON web tokens are signed and checked.\n---\nsteps").unwrap();
        crate::model::skill::SkillRegistry::load_for_test(&root)
    }

    #[test]
    fn skill_find_answers_a_request_and_never_lists() {
        let mut c = ctx(None);
        c.skill_registry = Some(registry_of(300));
        let out = SkillFind.run(&c, &json!({"query": "how are JWT tokens checked"})).unwrap();
        assert!(out.contains("jwt-review"), "{out}");
        assert!(out.lines().count() <= 12, "too long: {out}");
        assert!(SkillFind.run(&c, &json!({})).is_err(), "a find without a query must be refused");
        let none = SkillFind.run(&c, &json!({"query": "zzqx"})).unwrap();
        assert!(none.starts_with("No skill matches"));
    }

    #[test]
    fn skill_find_marks_the_active_ones_and_follows_a_rerank() {
        let mut c = ctx(Some(vec!["jwt-review".into()]));
        c.skill_registry = Some(registry_of(5));
        assert!(SkillFind.run(&c, &json!({"query": "jwt token"})).unwrap().contains("jwt-review [ACTIVE]"));
        let mut c = ctx(None);
        c.skill_registry = Some(registry_of(5));
        c.skill_rank = Some(std::sync::Arc::new(|_q, cands| Some(cands.iter().rev().map(|c| c.0.clone()).collect())));
        // "helper" matches every filler equally, so the answer is not decisive and the re-rank applies.
        let out = SkillFind.run(&c, &json!({"query": "helper chores", "limit": 2})).unwrap();
        assert!(out.contains("filler-4"), "{out}");
    }

    #[test]
    fn a_skill_outside_the_scope_is_neither_found_nor_loaded() {
        let mut c = ctx(None);
        c.skill_registry = Some(registry_of(3));
        c.skill_scope = Some(crate::model::skill::search::Scope::new(&["filler-*".to_string()]));
        assert!(SkillFind.run(&c, &json!({"query": "jwt token"})).unwrap().starts_with("No skill matches"));
        assert!(SkillLoad.run(&c, &json!({"name": "jwt-review"})).is_err());
        assert!(SkillLoad.run(&c, &json!({"name": "filler-1"})).unwrap().starts_with(SKILL_LOAD_PREFIX));
    }
}

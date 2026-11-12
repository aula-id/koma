//! Parsing and validation of registry files, and the rules for layering a
//! session overlay on top of the embedded base.
//!
//! Layering rule: an overlay can change text and narrow permissions but cannot
//! widen them. A role's shelf permissions can only shrink, its tools can only
//! bind to handlers the embedded role already uses, and the overlay cannot
//! create roles. A file that breaks a rule is skipped with a warning and the
//! embedded version of that file stays in force.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::model::{
    is_host_handler, Avatar, LaneModel, Registry, Role, Skill, ToolDef, KNOWN_HANDLERS,
    SHELF_TOOLS, SINGLE_ROLES,
};
use super::source::Tree;
use super::template;

const MAX_DESCRIPTION: usize = 2000;
const MAX_IDENT: usize = 48;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleToml {
    id: String,
    display: String,
    shelf_allow: Vec<String>,
    #[serde(default)]
    chat: LaneModel,
    #[serde(default)]
    work: LaneModel,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsToml {
    #[serde(default)]
    tool: Vec<ToolToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolToml {
    name: String,
    handler: String,
    description: String,
    schema: toml::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AvatarsToml {
    #[serde(default)]
    avatar: Vec<AvatarToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AvatarToml {
    id: String,
    role: String,
    display: Option<String>,
    alias: Option<String>,
    #[serde(default)]
    chat: LaneModel,
    #[serde(default)]
    work: LaneModel,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillFront {
    name: String,
    description: String,
}

#[derive(Default)]
struct RoleParts {
    role: Option<RoleToml>,
    tools: Option<Vec<ToolDef>>,
    prompt: Option<String>,
}

/// The valid contents of one file tree, before layering.
#[derive(Default)]
pub struct Parsed {
    roles: BTreeMap<String, RoleParts>,
    avatars: Option<Vec<Avatar>>,
    prompts: BTreeMap<String, String>,
    skills: BTreeMap<String, Skill>,
}

fn valid_ident(s: &str, allow_dash: bool) -> bool {
    !s.is_empty()
        && s.len() <= MAX_IDENT
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || (allow_dash && b == b'-')
        })
        && s.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Parse every recognised file in `tree`. Files that fail validation are
/// dropped and recorded in `warnings`.
pub fn parse(tree: &Tree, warnings: &mut Vec<String>) -> Parsed {
    let mut out = Parsed::default();
    for (path, text) in tree {
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["roles", id, "role.toml"] if valid_ident(id, true) => match parse_role(id, text) {
                Ok(role) => out.roles.entry((*id).to_string()).or_default().role = Some(role),
                Err(e) => warnings.push(format!("{path}: {e}")),
            },
            ["roles", id, "tools.toml"] if valid_ident(id, true) => match parse_tools(text) {
                Ok(tools) => out.roles.entry((*id).to_string()).or_default().tools = Some(tools),
                Err(e) => warnings.push(format!("{path}: {e}")),
            },
            ["roles", id, "prompt.md"] if valid_ident(id, true) => {
                if text.trim().is_empty() {
                    warnings.push(format!("{path}: empty prompt"));
                } else {
                    out.roles.entry((*id).to_string()).or_default().prompt =
                        Some(text.trim().to_string());
                }
            }
            ["avatars.toml"] => match parse_avatars(text) {
                Ok(avatars) => out.avatars = Some(avatars),
                Err(e) => warnings.push(format!("{path}: {e}")),
            },
            ["prompts", file] if file.ends_with(".md") => {
                let name = file.trim_end_matches(".md");
                if !valid_ident(name, false) {
                    warnings.push(format!("{path}: bad prompt name"));
                } else if let Err(e) = template::validate(text) {
                    warnings.push(format!("{path}: {e}"));
                } else {
                    out.prompts
                        .insert(name.to_string(), text.trim().to_string());
                }
            }
            ["skills", name, "SKILL.md"] if valid_ident(name, true) => {
                match parse_skill(name, text) {
                    Ok(skill) => {
                        out.skills.insert((*name).to_string(), skill);
                    }
                    Err(e) => warnings.push(format!("{path}: {e}")),
                }
            }
            // Other files inside a skill directory are companions; nothing to load.
            ["skills", _, ..] => {}
            _ => warnings.push(format!("{path}: not a registry file; ignored")),
        }
    }
    out
}

fn parse_avatars(text: &str) -> Result<Vec<Avatar>, String> {
    let file: AvatarsToml = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut ids = BTreeSet::new();
    let mut out = Vec::with_capacity(file.avatar.len());
    for a in file.avatar {
        if !valid_ident(&a.id, true) {
            return Err(format!("bad avatar id '{}'", a.id));
        }
        if !valid_ident(&a.role, true) {
            return Err(format!("avatar '{}': bad role '{}'", a.id, a.role));
        }
        if !ids.insert(a.id.clone()) {
            return Err(format!("avatar '{}' defined twice", a.id));
        }
        if SINGLE_ROLES.contains(&a.role.as_str()) && a.id != a.role {
            return Err(format!(
                "the {} role has exactly one avatar, named '{}'",
                a.role, a.role
            ));
        }
        if SINGLE_ROLES.contains(&a.id.as_str()) && a.role != a.id {
            return Err(format!(
                "avatar id '{}' is reserved for the {} role",
                a.id, a.id
            ));
        }
        out.push(Avatar {
            id: a.id,
            role: a.role,
            display: a
                .display
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
            alias: a.alias.map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty() && s.chars().count() <= 3 && s.chars().all(|c| c.is_ascii_alphanumeric())),
            chat: a.chat,
            work: a.work,
        });
    }
    Ok(out)
}

fn parse_role(dir_id: &str, text: &str) -> Result<RoleToml, String> {
    let role: RoleToml = toml::from_str(text).map_err(|e| e.to_string())?;
    if role.id != dir_id {
        return Err(format!(
            "id '{}' does not match directory '{dir_id}'",
            role.id
        ));
    }
    if role.display.trim().is_empty() {
        return Err("display is empty".to_string());
    }
    let mut seen = BTreeSet::new();
    for tool in &role.shelf_allow {
        if !SHELF_TOOLS.contains(&tool.as_str()) {
            return Err(format!("unknown shelf tool '{tool}'"));
        }
        if !seen.insert(tool.as_str()) {
            return Err(format!("shelf tool '{tool}' listed twice"));
        }
    }
    Ok(role)
}

fn parse_tools(text: &str) -> Result<Vec<ToolDef>, String> {
    let file: ToolsToml = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut names = BTreeSet::new();
    let mut out = Vec::with_capacity(file.tool.len());
    for tool in file.tool {
        if !valid_ident(&tool.name, false) {
            return Err(format!("bad tool name '{}'", tool.name));
        }
        if !names.insert(tool.name.clone()) {
            return Err(format!("tool '{}' defined twice", tool.name));
        }
        if tool.name.starts_with("sec_") || tool.handler.starts_with("sec_") {
            return Err(format!(
                "tool '{}': sec_ tools are not part of the room",
                tool.name
            ));
        }
        if !KNOWN_HANDLERS.contains(&tool.handler.as_str()) {
            return Err(format!(
                "tool '{}': unknown handler '{}'",
                tool.name, tool.handler
            ));
        }
        if is_host_handler(&tool.handler) && tool.name != tool.handler {
            return Err(format!(
                "tool '{}': a host tool's name is its handler '{}'",
                tool.name, tool.handler
            ));
        }
        let description = tool.description.trim();
        if description.is_empty() || description.len() > MAX_DESCRIPTION {
            return Err(format!(
                "tool '{}': description must be 1..{MAX_DESCRIPTION} bytes",
                tool.name
            ));
        }
        let schema = serde_json::to_value(&tool.schema).map_err(|e| e.to_string())?;
        if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
            return Err(format!(
                "tool '{}': schema must be an object schema",
                tool.name
            ));
        }
        out.push(ToolDef {
            name: tool.name,
            handler: tool.handler,
            description: description.to_string(),
            schema,
        });
    }
    Ok(out)
}

/// Split `---\n<yaml>\n---\n<body>`. Text without a leading fence is all body.
fn split_frontmatter(text: &str) -> Result<(Option<&str>, &str), String> {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok((None, text));
    };
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            return Ok((Some(&rest[..offset]), &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err("frontmatter not closed".to_string())
}

fn parse_skill(dir_name: &str, text: &str) -> Result<Skill, String> {
    let (front, body) = split_frontmatter(text)?;
    let front = front.ok_or_else(|| "missing frontmatter (name, description)".to_string())?;
    let front: SkillFront = serde_yaml_ng::from_str(front).map_err(|e| e.to_string())?;
    if front.name != dir_name {
        return Err(format!(
            "name '{}' does not match directory '{dir_name}'",
            front.name
        ));
    }
    if front.description.trim().is_empty() {
        return Err("description is empty".to_string());
    }
    let body = body.trim();
    if body.is_empty() {
        return Err("empty skill body".to_string());
    }
    Ok(Skill {
        name: front.name,
        description: front.description.trim().to_string(),
        body: body.to_string(),
    })
}

/// Build the base registry from the embedded files. Strict: any warning or
/// missing part is an error, because the embedded copy is the fallback that
/// must always be valid.
pub fn build_base(tree: &Tree) -> Result<Registry, String> {
    let mut warnings = Vec::new();
    let parsed = parse(tree, &mut warnings);
    if !warnings.is_empty() {
        return Err(format!(
            "embedded room files are invalid: {}",
            warnings.join("; ")
        ));
    }
    let mut registry = Registry {
        version: 1,
        prompts: parsed.prompts,
        skills: parsed.skills,
        ..Registry::default()
    };
    for (id, parts) in parsed.roles {
        let (Some(role), Some(tools), Some(prompt)) = (parts.role, parts.tools, parts.prompt)
        else {
            return Err(format!(
                "embedded role '{id}' needs role.toml, tools.toml and prompt.md"
            ));
        };
        registry.roles.insert(
            id.clone(),
            Role {
                id,
                display: role.display,
                shelf_allow: role.shelf_allow,
                chat: role.chat,
                work: role.work,
                tools,
                prompt,
            },
        );
    }
    // Without an avatars file every role gets one avatar named after it.
    let avatars = parsed.avatars.unwrap_or_else(|| {
        registry
            .roles
            .keys()
            .map(|id| Avatar {
                id: id.clone(),
                role: id.clone(),
                display: None,
                alias: None,
                chat: LaneModel::default(),
                work: LaneModel::default(),
            })
            .collect()
    });
    for a in avatars {
        if !registry.roles.contains_key(&a.role) {
            return Err(format!(
                "embedded avatar '{}' uses unknown role '{}'",
                a.id, a.role
            ));
        }
        registry.avatars.insert(a.id.clone(), a);
    }
    for single in SINGLE_ROLES {
        if registry.roles.contains_key(*single) && !registry.avatars.contains_key(*single) {
            return Err(format!("embedded avatars must include '{single}'"));
        }
    }
    Ok(registry)
}

/// Layer an overlay on `registry`, enforcing the narrowing rule.
pub fn apply_overlay(registry: &mut Registry, overlay: Parsed, warnings: &mut Vec<String>) {
    for (id, parts) in overlay.roles {
        let Some(base) = registry.roles.get_mut(&id) else {
            warnings.push(format!("roles/{id}: the overlay cannot add roles; ignored"));
            continue;
        };
        if let Some(role) = parts.role {
            let allowed: BTreeSet<&str> = base.shelf_allow.iter().map(String::as_str).collect();
            match role
                .shelf_allow
                .iter()
                .find(|t| !allowed.contains(t.as_str()))
            {
                Some(extra) => warnings.push(format!(
                    "roles/{id}/role.toml: shelf tool '{extra}' would widen the role; file ignored"
                )),
                None => {
                    base.display = role.display;
                    base.shelf_allow = role.shelf_allow;
                    base.chat = role.chat;
                    base.work = role.work;
                }
            }
        }
        if let Some(tools) = parts.tools {
            let allowed: BTreeSet<&str> = base.tools.iter().map(|t| t.handler.as_str()).collect();
            match tools.iter().find(|t| !allowed.contains(t.handler.as_str())) {
                Some(extra) => warnings.push(format!(
                    "roles/{id}/tools.toml: handler '{}' is not one of this role's handlers; file ignored",
                    extra.handler
                )),
                None => base.tools = tools,
            }
        }
        if let Some(prompt) = parts.prompt {
            base.prompt = prompt;
        }
    }
    // Avatars merge by id: the overlay can add avatars or restyle existing
    // ones, but an existing avatar keeps its role and every avatar must play a
    // role that exists. Permissions come from the role, so this widens nothing.
    for a in overlay.avatars.into_iter().flatten() {
        if !registry.roles.contains_key(&a.role) {
            warnings.push(format!(
                "avatars.toml: avatar '{}' uses unknown role '{}'; ignored",
                a.id, a.role
            ));
            continue;
        }
        match registry.avatars.get(&a.id) {
            Some(existing) if existing.role != a.role => warnings.push(format!(
                "avatars.toml: avatar '{}' cannot change its role; ignored",
                a.id
            )),
            _ => {
                registry.avatars.insert(a.id.clone(), a);
            }
        }
    }
    registry.prompts.extend(overlay.prompts);
    registry.skills.extend(overlay.skills);
}

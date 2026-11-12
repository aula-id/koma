//! Registry data model: roles, tool definitions, prompt templates, skills.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::template::TemplateError;

/// Shelf tools a role may be granted. A role file can only list names from here.
pub const SHELF_TOOLS: &[&str] = &[
    "put",
    "get",
    "list",
    "put_scope",
    "put_claim",
    "put_evidence",
    "put_verdict",
    "put_finding",
    "put_report",
    "put_asset",
    "put_trajectory",
    "put_result",
];

/// Host tools a role may be granted for its lanes, chat and work alike. The
/// handler name is the builtin name. The clerk is not granted these.
/// `sec_*` is never a host handler.
pub const HOST_HANDLERS: &[&str] = &[
    "read",
    "write",
    "edit",
    "delete",
    "dir_list",
    "grep",
    "glob",
    "bash",
    "remember",
    "recall",
    "forget",
    "message_find",
    "message_load",
    "skill",
    "web_search",
    "web_fetch",
    "browser_tabs",
    "browser_inspect",
    "browser_interact",
    "browser_evaluate",
];

/// Handlers compiled into the binary. A tool definition may only bind to one of
/// these, so a hot-reloaded file can never introduce new behaviour.
pub const KNOWN_HANDLERS: &[&str] = &[
    "group_delegate",
    "group_status",
    "group_nudge",
    "group_finish",
    "group_task_complete",
    "message_user",
    "clerk_flag",
    "read",
    "write",
    "edit",
    "delete",
    "dir_list",
    "grep",
    "glob",
    "bash",
    "remember",
    "recall",
    "forget",
    "message_find",
    "message_load",
    "skill",
    "web_search",
    "web_fetch",
    "browser_tabs",
    "browser_inspect",
    "browser_interact",
    "browser_evaluate",
];

/// Room handlers (coordination, shelf is separate, clerk). Everything else in
/// [`KNOWN_HANDLERS`] is a host builtin.
pub fn is_host_handler(handler: &str) -> bool {
    HOST_HANDLERS.contains(&handler)
}

/// Model selection for one lane. Unset fields inherit the session default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneModel {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// One tool the model sees, bound to a compiled handler.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub handler: String,
    pub description: String,
    /// JSON schema of the arguments.
    pub schema: serde_json::Value,
}

/// A room avatar's definition. Every avatar has a chat lane and a work lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Role {
    pub id: String,
    pub display: String,
    pub shelf_allow: Vec<String>,
    pub chat: LaneModel,
    pub work: LaneModel,
    pub tools: Vec<ToolDef>,
    pub prompt: String,
}

/// A member of the room. It points at a [`Role`], which supplies the prompt,
/// tools and shelf permissions, so adding avatars adds capacity, not privilege.
/// Models set here override the role's, per lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Avatar {
    pub id: String,
    pub role: String,
    /// Shown instead of the role's display name when set.
    pub display: Option<String>,
    pub chat: LaneModel,
    pub work: LaneModel,
}

/// Which of an avatar's two lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LaneKind {
    Chat,
    Work,
}

/// Roles that exist exactly once, as the avatar with the same id.
pub const SINGLE_ROLES: &[&str] = &["chief", "clerk"];

/// A skill document available to the room's agents.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Why a template could not be rendered.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    UnknownPrompt(String),
    MissingVar { prompt: String, var: String },
    Syntax { prompt: String, detail: String },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::UnknownPrompt(name) => write!(f, "no prompt template named '{name}'"),
            RenderError::MissingVar { prompt, var } => {
                write!(f, "prompt '{prompt}' needs variable '{var}'")
            }
            RenderError::Syntax { prompt, detail } => {
                write!(f, "prompt '{prompt}' is malformed: {detail}")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// An immutable snapshot of everything the room loads from files.
///
/// A turn takes one snapshot at its start and records [`Registry::version`], so
/// a reload never changes a turn that is already running.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    /// Bumped on every reload that changed the content.
    pub version: u64,
    pub roles: BTreeMap<String, Role>,
    pub avatars: BTreeMap<String, Avatar>,
    pub prompts: BTreeMap<String, String>,
    pub skills: BTreeMap<String, Skill>,
}

impl Registry {
    pub fn role(&self, id: &str) -> Option<&Role> {
        self.roles.get(id)
    }

    pub fn avatar(&self, id: &str) -> Option<&Avatar> {
        self.avatars.get(id)
    }

    /// The role an avatar plays.
    pub fn role_of(&self, avatar: &str) -> Option<&Role> {
        self.roles.get(&self.avatars.get(avatar)?.role)
    }

    /// Every avatar, in id order.
    pub fn all_avatars(&self) -> impl Iterator<Item = &Avatar> {
        self.avatars.values()
    }

    /// Avatars that play `role`, in id order.
    pub fn avatars_of(&self, role: &str) -> Vec<&Avatar> {
        self.avatars.values().filter(|a| a.role == role).collect()
    }

    /// Name to show for an avatar.
    pub fn display_of<'a>(&'a self, avatar: &'a str) -> &'a str {
        match self.avatars.get(avatar) {
            Some(a) => a
                .display
                .as_deref()
                .or_else(|| self.roles.get(&a.role).map(|r| r.display.as_str()))
                .unwrap_or(avatar),
            None => avatar,
        }
    }

    /// The model for one of an avatar's lanes: the avatar's own settings win,
    /// then the role's, field by field. Unset fields inherit the session default.
    pub fn lane_model(&self, avatar: &str, lane: LaneKind) -> LaneModel {
        let Some(a) = self.avatars.get(avatar) else {
            return LaneModel::default();
        };
        let (own, base) = match (lane, self.roles.get(&a.role)) {
            (LaneKind::Chat, role) => (&a.chat, role.map(|r| &r.chat)),
            (LaneKind::Work, role) => (&a.work, role.map(|r| &r.work)),
        };
        let pick = |own: &Option<String>, base: Option<&Option<String>>| {
            own.clone().or_else(|| base.and_then(Clone::clone))
        };
        LaneModel {
            provider: pick(&own.provider, base.map(|b| &b.provider)),
            model: pick(&own.model, base.map(|b| &b.model)),
            effort: pick(&own.effort, base.map(|b| &b.effort)),
        }
    }

    /// Render the named prompt template with `vars`. Extra variables are
    /// ignored; a placeholder without a value is an error.
    pub fn render(&self, prompt: &str, vars: &[(&str, &str)]) -> Result<String, RenderError> {
        let body = self
            .prompts
            .get(prompt)
            .ok_or_else(|| RenderError::UnknownPrompt(prompt.to_string()))?;
        super::template::render(body, vars).map_err(|e| match e {
            TemplateError::MissingVar(var) => RenderError::MissingVar {
                prompt: prompt.to_string(),
                var,
            },
            TemplateError::Syntax(detail) => RenderError::Syntax {
                prompt: prompt.to_string(),
                detail,
            },
        })
    }
}

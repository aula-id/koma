//! Enums and constants for the `/agents` dashboard sub-mode state machine and
//! field navigation.

/// Which scope a freshly-created agent is written to.
///
/// Mirrors [`crate::model::agent_def::AgentScope`] but owns no borrow, so it can
/// live inside the long-lived [`super::AgentsState`]. Converted to the borrowed scope
/// at save time (it needs the session dir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentScope {
    /// `<session_dir>/agents/`.
    Session,
    /// `~/.koma/agents/`.
    Global,
}

impl AgentScope {
    /// Short label used in the create-scope picker and source tags.
    pub fn label(self) -> &'static str {
        match self {
            AgentScope::Session => "session",
            AgentScope::Global => "global",
        }
    }
}

/// The active sub-mode of the `/agents` dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSubMode {
    /// Navigating the agent list / reading the selected agent (read-only).
    Browse,
    /// Editing an existing file-backed agent's fields + body.
    Edit,
    /// Creating a new agent (scope + name first, then the same field editor).
    Create,
    /// Confirming deletion of the selected file-backed agent (`y`/`n`).
    DeleteConfirm,
}

/// One editable field in the Edit/Create detail editor, in display/nav order.
///
/// `Name` is only navigable in Create (an existing agent's name is its
/// filename and is not renamed in place — renaming = delete + create).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEditField {
    /// Create-only: the filename slug (sanitised by the data layer on save).
    Name,
    /// Required user-facing description (frontmatter `description`). Full-size
    /// editable (opens the nano editor like the Body).
    Description,
    /// Optional free-text describing WHEN to delegate to this agent (frontmatter
    /// `conditions`). Full-size editable (opens the nano editor like the Body).
    Conditions,
    /// Registered model the agent runs on; `(inherit main)` when unset. Opens a
    /// single-select picker over the models registered in `/settings`.
    Model,
    /// Comma/space separated tool allow-list (frontmatter `tools`).
    Tools,
    /// The markdown body = the agent system prompt.
    Body,
}

impl AgentEditField {
    /// Left-column label for the detail editor.
    pub fn label(self) -> &'static str {
        match self {
            AgentEditField::Name => "name",
            AgentEditField::Description => "description",
            AgentEditField::Conditions => "conditions",
            AgentEditField::Model => "model",
            AgentEditField::Tools => "tools",
            AgentEditField::Body => "prompt",
        }
    }
}

/// Field order while EDITING an existing agent (no name row).
pub(super) const EDIT_FIELDS: &[AgentEditField] = &[
    AgentEditField::Description,
    AgentEditField::Conditions,
    AgentEditField::Model,
    AgentEditField::Tools,
    AgentEditField::Body,
];

/// Field order while CREATING a new agent (name row first).
pub(super) const CREATE_FIELDS: &[AgentEditField] = &[
    AgentEditField::Name,
    AgentEditField::Description,
    AgentEditField::Conditions,
    AgentEditField::Model,
    AgentEditField::Tools,
    AgentEditField::Body,
];

/// Seed body for a freshly-created agent (placeholder system prompt).
pub(super) const TEMPLATE_BODY: &str =
    "You are a focused subagent. Do the task you are given, then report your\nfindings concisely.";

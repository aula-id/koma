//! [`AgentSource`] enum and [`AgentDef`] struct with its `impl` block.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The safe read-only default tool set used when an agent declares no `tools`.
pub(super) const DEFAULT_TOOLS: [&str; 4] = ["read", "grep", "glob", "dir_list"];

/// The recursion-guard tool that is NEVER auto-included in an agent's allow-list.
pub(super) const TASK_TOOL: &str = "task";

// ---------------------------------------------------------------------------
// AgentDef
// ---------------------------------------------------------------------------

/// Source origin of the agent definition (determines load tier / precedence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentSource {
    /// Built-in agent compiled into the binary.
    Builtin,
    /// Global agent from `~/.simple-coder/agents/`.
    Global,
    /// Session-specific agent from `<session_dir>/agents/`.
    #[default]
    Session,
}

/// One agent definition loaded from frontmatter + markdown.
///
/// Frontmatter fields are deserialized directly; `name`, `prompt`, `source`,
/// and `file_path` are runtime-only state (`#[serde(skip)]`) populated by the
/// loader.
///
/// `PartialEq` lets a `Vec<AgentDef>` ride inside the daemon's `/agents` snapshot
/// projection (whose enclosing `ModeSnapshot` is compared by the snapshot differ to
/// detect a mode-payload change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDef {
    /// Agent name — the filename stem, lowercased (`PINEAPPLE.md` → `pineapple`).
    /// Validated to alnum + dash, no path traversal. The agent identifier.
    #[serde(skip)]
    pub name: String,

    /// User-facing description (when to use this agent; shown in menus). Required.
    /// Human-facing label only — NOT injected into the system-prompt roster.
    pub description: String,

    /// Free-text describing WHEN the main agent should delegate to this sub-agent.
    /// This is the only field injected into the system-prompt sub-agent roster
    /// (falling back to `description` when empty). Optional.
    #[serde(default)]
    pub conditions: String,

    /// UUID of a registered [`crate::model::app_config::ModelEntry`] this agent
    /// runs on. When `Some`, the resolver looks up the entry in `session_models`
    /// first, then the global `config.models`, and dispatches via that entry's
    /// provider connection. `None` = inherit the Main role (legacy fallback).
    /// Takes precedence over the legacy `model` / `provider_uuid` fields below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_uuid: Option<String>,

    /// OpenRouter model slug (e.g. `"openai/gpt-oss-20b"`), or `None` to inherit
    /// the session model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// OpenRouter provider routing slug (e.g. `"groq"`). `None` means default
    /// routing.
    ///
    /// LEGACY back-compat: older agent files carried a free-text `provider`
    /// routing slug. It is still READ + WRITTEN so those files round-trip, but
    /// the editor's Provider field now drives [`Self::provider_uuid`] (a chosen
    /// API provider *connection*) instead of this routing slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// The chosen API provider connection (a [`crate::model::app_config::ProviderConn`]
    /// uuid) this agent dispatches against. `None` = inherit the session's
    /// provider. Distinct from the legacy `provider` routing slug above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_uuid: Option<String>,

    /// Allow-list of tool names this agent may invoke. The `"task"` tool is never
    /// auto-included (recursion guard). An empty/absent list means the safe
    /// read-only default `[read, grep, glob, dir_list]`.
    #[serde(default)]
    pub tools: Vec<String>,

    /// Maximum agentic iterations for this agent (reasoning-loop cap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,

    /// Reasoning/thinking effort (`""`, `"off"`, `"low"`, `"high"`, `"max"`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,

    /// Sampling temperature (typically `0.0`–`2.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Theme accent (color name or hex code).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// When `true`, hide this agent from menus but keep it in the registry.
    #[serde(default)]
    pub hidden: bool,

    /// When `true`, remove this agent from the registry (disables a built-in or
    /// file agent). Processed at load time.
    #[serde(default)]
    pub disable: bool,

    /// The markdown body following the frontmatter — the agent system prompt.
    /// Not from frontmatter; extracted after the split.
    #[serde(skip)]
    pub prompt: String,

    /// Where this agent was loaded from. Populated by the loader.
    #[serde(skip)]
    pub source: AgentSource,

    /// Absolute path to the `.md` file, or `None` for built-ins. Used for
    /// save/delete.
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

impl Default for AgentDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            conditions: String::new(),
            model_uuid: None,
            model: None,
            provider: None,
            provider_uuid: None,
            tools: Vec::new(),
            steps: None,
            effort: None,
            temperature: None,
            color: None,
            hidden: false,
            disable: false,
            prompt: String::new(),
            source: AgentSource::Session,
            file_path: None,
        }
    }
}

impl AgentDef {
    /// Construct a built-in agent (no file path, `source = Builtin`).
    pub fn builtin(name: &str, description: &str, prompt: &str, tools: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            prompt: prompt.to_string(),
            tools,
            source: AgentSource::Builtin,
            ..Default::default()
        }
    }

    /// The effective tool allow-list for this agent.
    ///
    /// Falls back to the safe read-only default when `tools` is empty, and always
    /// strips the `"task"` tool (recursion guard) regardless of what was declared.
    pub fn effective_tools(&self) -> Vec<String> {
        let base: Vec<String> = if self.tools.is_empty() {
            DEFAULT_TOOLS.iter().map(|t| t.to_string()).collect()
        } else {
            self.tools.clone()
        };
        base.into_iter().filter(|t| t != TASK_TOOL).collect()
    }

    /// Render this agent into a frontmatter + markdown string for writing to disk.
    ///
    /// Only emits fields that are explicitly set; the read-only default tool set
    /// is treated as "unset" and omitted so the round-trip stays minimal.
    pub fn to_markdown(&self) -> String {
        let mut fm = serde_yaml_ng::Mapping::new();
        let key = |s: &str| serde_yaml_ng::Value::String(s.to_string());
        let sval = |s: &str| serde_yaml_ng::Value::String(s.to_string());

        // Required.
        fm.insert(key("description"), sval(&self.description));

        // Optional: only emit when set (when to delegate to this agent).
        if !self.conditions.trim().is_empty() {
            fm.insert(key("conditions"), sval(&self.conditions));
        }

        if let Some(model_uuid) = &self.model_uuid {
            fm.insert(key("model_uuid"), sval(model_uuid));
        }
        if let Some(model) = &self.model {
            fm.insert(key("model"), sval(model));
        }
        if let Some(provider) = &self.provider {
            fm.insert(key("provider"), sval(provider));
        }
        if let Some(provider_uuid) = &self.provider_uuid {
            fm.insert(key("provider_uuid"), sval(provider_uuid));
        }
        if !self.tools.is_empty() {
            let seq = self.tools.iter().map(|t| sval(t)).collect();
            fm.insert(key("tools"), serde_yaml_ng::Value::Sequence(seq));
        }
        if let Some(steps) = self.steps {
            fm.insert(key("steps"), serde_yaml_ng::Value::Number(steps.into()));
        }
        if let Some(effort) = &self.effort {
            fm.insert(key("effort"), sval(effort));
        }
        if let Some(temperature) = self.temperature {
            // f32 -> f64 keeps the YAML number well-formed.
            fm.insert(
                key("temperature"),
                serde_yaml_ng::Value::Number((temperature as f64).into()),
            );
        }
        if let Some(color) = &self.color {
            fm.insert(key("color"), sval(color));
        }
        if self.hidden {
            fm.insert(key("hidden"), serde_yaml_ng::Value::Bool(true));
        }
        if self.disable {
            fm.insert(key("disable"), serde_yaml_ng::Value::Bool(true));
        }

        let fm_str = serde_yaml_ng::to_string(&serde_yaml_ng::Value::Mapping(fm))
            .unwrap_or_default();
        let fm_str = fm_str.trim_end();
        format!("---\n{fm_str}\n---\n{}", self.prompt)
    }
}

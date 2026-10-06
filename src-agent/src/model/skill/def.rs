//! Skill data types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Source origin of a skill (determines load tier / precedence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SkillSource {
    /// Global skill from `~/.koma/skills/`.
    Global,
    /// Claude compat skill from `<workdir>/.claude/skills/`.
    Claude,
    /// Koma project skill from `<workdir>/.agent/skills/`.
    ProjectAgent,
    /// Koma project skill from `<workdir>/.agents/skills/`.
    #[default]
    ProjectAgents,
    /// User-configured read-only discovery root. The value is the original
    /// configuration index so Settings can report the winning source.
    ExtraRoot(usize),
}

/// Stable ownership scope exposed to GUI clients. Precedence internals remain
/// server-side so clients cannot turn a display label into write authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillScopeWire {
    Global,
    Project,
    External,
}

impl SkillSource {
    pub fn scope(self) -> SkillScopeWire {
        match self {
            Self::Global => SkillScopeWire::Global,
            Self::ProjectAgent | Self::ProjectAgents => SkillScopeWire::Project,
            Self::Claude | Self::ExtraRoot(_) => SkillScopeWire::External,
        }
    }

    pub fn editable(self) -> bool {
        matches!(
            self,
            Self::Global | Self::ProjectAgent | Self::ProjectAgents
        )
    }

    pub fn external_root_index(self) -> Option<usize> {
        match self {
            Self::ExtraRoot(index) => Some(index),
            _ => None,
        }
    }
}

/// One skill definition loaded from a markdown file.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillDef {
    pub name: String,
    pub description: String,
    /// Free-text `triggers` compatibility metadata.
    pub triggers: String,
    /// Optional `allowed-tools` declaration. Koma displays but does not enforce it.
    pub allowed_tools: Vec<String>,
    /// Full markdown body after frontmatter — loaded into a session only on activation.
    pub body: String,
    pub source: SkillSource,
    /// Absolute/canonical entry path where available.
    pub file_path: PathBuf,
    pub skill_dir: Option<PathBuf>,
    /// Recursive, relative companion file names; contents remain lazy.
    pub companion_files: Vec<String>,
    /// Parsed ordinary YAML mapping retained for semantic unknown-key preservation.
    pub frontmatter: Option<serde_yaml_ng::Mapping>,
    /// Exact bytes between the opening and closing frontmatter fences.
    pub raw_frontmatter: Option<String>,
    /// False when structured Save must fail closed instead of risking YAML loss.
    pub frontmatter_roundtrip_safe: bool,
    /// Server-generated opaque identity for the discovered source location.
    pub skill_id: String,
    /// Server-generated generation token for the current entry bytes.
    pub generation: String,
}

/// Lightweight body-free catalogue row for the Skills sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogueEntry {
    pub skill_id: String,
    pub generation: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: String,
    pub source_tier: String,
    pub scope: SkillScopeWire,
    /// Display-only source path. Backend actions never trust this as authority.
    pub source_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_root_index: Option<usize>,
}

/// Full on-demand editor payload. The body never rides regular snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub skill_id: String,
    pub generation: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: String,
    #[serde(default)]
    pub declared_tools: Vec<String>,
    pub instruction: String,
    #[serde(default)]
    pub companion_files: Vec<String>,
    pub scope: SkillScopeWire,
    pub source_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_root_index: Option<usize>,
    pub editable: bool,
    pub structured_save_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillIdentityInput {
    pub skill_id: String,
    pub generation: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDuplicateInput {
    pub skill_id: String,
    pub generation: String,
    pub name: String,
    pub destination_name: String,
}

/// Deterministic result for one bulk skill operation item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillItemOutcome {
    pub name: String,
    /// `success`, `skipped`, or `failed`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SkillCatalogueEntry {
    pub fn from_def(def: &SkillDef) -> Self {
        Self {
            skill_id: def.skill_id.clone(),
            generation: def.generation.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            triggers: def.triggers.clone(),
            source_tier: match def.source {
                SkillSource::Global => "global",
                SkillSource::Claude => "claude",
                SkillSource::ProjectAgent => "project-agent",
                SkillSource::ProjectAgents => "project-agents",
                SkillSource::ExtraRoot(_) => "extra-root",
            }
            .to_string(),
            scope: def.source.scope(),
            source_path: def.file_path.clone(),
            external_root_index: def.source.external_root_index(),
        }
    }
}

impl SkillDetail {
    pub fn from_def(def: &SkillDef) -> Self {
        Self {
            skill_id: def.skill_id.clone(),
            generation: def.generation.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            triggers: def.triggers.clone(),
            declared_tools: def.allowed_tools.clone(),
            instruction: def.body.clone(),
            companion_files: def.companion_files.clone(),
            scope: def.source.scope(),
            source_path: def.file_path.clone(),
            external_root_index: def.source.external_root_index(),
            editable: def.source.editable(),
            structured_save_supported: def.source.editable() && def.frontmatter_roundtrip_safe,
        }
    }
}

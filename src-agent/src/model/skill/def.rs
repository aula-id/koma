//! Skill data types.

use std::path::PathBuf;

/// Source origin of a skill (determines load tier / precedence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillSource {
    /// Global skill from `~/.koma/skills/`.
    Global,
    /// Claude compat skill from `<workdir>/.claude/skills/`.
    Claude,
    /// Koma project skill from `<workdir>/.agent/skills/`.
    ProjectAgent,
    /// Koma project skill from `<workdir>/.agents/skills/` (highest priority).
    #[default]
    ProjectAgents,
}

/// One skill definition loaded from a markdown file.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillDef {
    /// Skill name — the filename stem or directory name, lowercased and validated.
    pub name: String,
    /// User-facing description of when to use this skill. Required.
    pub description: String,
    /// Full markdown body after frontmatter — the instructions loaded on demand.
    pub body: String,
    /// Where this skill was loaded from.
    pub source: SkillSource,
    /// Absolute path to the entry file (`foo.md` or `bar/SKILL.md`).
    pub file_path: PathBuf,
    /// Parent directory of a dir-form skill (`Some` for `bar/SKILL.md`, `None`
    /// for flat `foo.md`). Lets the loaded body reference sibling files.
    pub skill_dir: Option<PathBuf>,
}

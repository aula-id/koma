//! Skill discovery: scan known directories for skill files and build a registry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::def::{SkillDef, SkillSource};
use super::parse::load_skill_file;

/// Skill entry file names (case-insensitive match — both accepted).
const SKILL_ENTRY_FILES: &[&str] = &["SKILL.md", "skill.md"];

/// Skip these filenames when scanning for flat skill files.
const SKIP_NAMES: &[&str] = &["readme.md", "readme"];

/// The in-memory skill registry: lowercased name → [`SkillDef`].
///
/// Skills from higher-precedence tiers override earlier ones by name.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, SkillDef>,
}

impl SkillRegistry {
    /// Load the full skill registry by scanning all discovery roots.
    ///
    /// `workdir` is the primary working directory (for project-level roots).
    /// Scan order (later overrides earlier by name):
    /// 1. `~/.koma/skills/` (global)
    /// 2. `<workdir>/.claude/skills/` (Claude compat)
    /// 3. `<workdir>/.agent/skills/` (koma project)
    /// 4. `<workdir>/.agents/skills/` (koma project, highest)
    pub fn load(workdir: Option<&Path>) -> Self {
        let mut skills: BTreeMap<String, SkillDef> = BTreeMap::new();

        // Tier 1: global.
        if let Ok(dir) = global_skills_dir() {
            scan_skills_root(&dir, SkillSource::Global, &mut skills);
        }

        // Tier 2: Claude compat.
        if let Some(wd) = workdir {
            let dir = wd.join(".claude").join("skills");
            scan_skills_root(&dir, SkillSource::Claude, &mut skills);
        }

        // Tier 3: koma project (.agent).
        if let Some(wd) = workdir {
            let dir = wd.join(".agent").join("skills");
            scan_skills_root(&dir, SkillSource::ProjectAgent, &mut skills);
        }

        // Tier 4: koma project (.agents) — highest priority.
        if let Some(wd) = workdir {
            let dir = wd.join(".agents").join("skills");
            scan_skills_root(&dir, SkillSource::ProjectAgents, &mut skills);
        }

        Self { skills }
    }

    /// Get a skill by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&SkillDef> {
        self.skills.get(&name.to_lowercase())
    }

    /// List all skills sorted by name.
    pub fn list(&self) -> Vec<&SkillDef> {
        self.skills.values().collect()
    }

    /// Number of skills in the registry.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Format a catalogue for system prompt injection.
    ///
    /// Returns one `- name: description` line per skill, sorted by name.
    /// Empty string when no skills are found.
    pub fn catalogue_text(&self) -> String {
        self.skills
            .values()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Returns `~/.koma/skills/` (the global skills directory).
fn global_skills_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::model::store::base_dir()?.join("skills"))
}

/// Scan a single skills root directory for skill entries.
///
/// Supports both flat (`foo.md`) and dir-form (`bar/SKILL.md`).
/// Missing directory is not an error. One malformed file is logged and skipped.
fn scan_skills_root(
    root: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            // Flat form: `<name>.md` directly under root.
            load_flat_skill(&path, source, skills);
        } else if path.is_dir() {
            // Dir form: `<name>/SKILL.md` or `<name>/skill.md`.
            load_dir_skill(&path, source, skills);
        }
    }
}

/// Try to load a flat skill file (`foo.md`) from a skills root.
fn load_flat_skill(
    path: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
) {
    let name = match path.file_stem() {
        Some(s) => s.to_string_lossy().to_lowercase(),
        None => return,
    };
    // Skip non-md files and common junk.
    if path.extension().is_none_or(|ext| ext != "md") {
        return;
    }
    if SKIP_NAMES.contains(&name.as_str()) {
        return;
    }
    match load_skill_file(path, source, None) {
        Ok(skill) => {
            skills.insert(skill.name.clone(), skill);
        }
        Err(e) => {
            crate::model::store::append_global_error_log(
                "skill registry",
                &format!("skipped {}: {e}", path.display()),
            );
        }
    }
}

/// Try to load a dir-form skill (`<name>/SKILL.md` or `<name>/skill.md`).
fn load_dir_skill(
    dir: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
) {
    // Look for SKILL.md or skill.md (case-insensitive).
    let entry_file = SKILL_ENTRY_FILES
        .iter()
        .find(|name| dir.join(name).exists())
        .map(|name| dir.join(name));

    let Some(entry_path) = entry_file else {
        return; // No entry file — not a skill dir.
    };

    let skill_dir = Some(dir.to_path_buf());
    match load_skill_file(&entry_path, source, skill_dir) {
        Ok(skill) => {
            skills.insert(skill.name.clone(), skill);
        }
        Err(e) => {
            crate::model::store::append_global_error_log(
                "skill registry",
                &format!("skipped {}: {e}", entry_path.display()),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn scan_skills_root_for_test(
    root: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
) {
    scan_skills_root(root, source, skills);
}

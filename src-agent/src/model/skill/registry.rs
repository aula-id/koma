//! Skill discovery: scan known directories for skill files and build a registry.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use super::def::{SkillCatalogueEntry, SkillDef, SkillSource};
use super::parse::load_skill_file;

const SKILL_ENTRY_FILES: &[&str] = &["SKILL.md", "skill.md"];
const SKIP_NAMES: &[&str] = &["readme.md", "readme"];
pub const MAX_SKILL_TEXT_READ_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidExtraSkillRoot {
    pub config_index: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, SkillDef>,
}

impl SkillRegistry {
    /// Load all built-in v0.4.0 tiers, then valid configured External roots.
    /// Later tiers override earlier skills by canonical name.
    pub fn load(workdir: Option<&Path>, extra_skill_roots: &[PathBuf]) -> Self {
        let mut skills = BTreeMap::new();
        if let Ok(dir) = global_skills_dir() {
            scan_skills_root(&dir, SkillSource::Global, &mut skills);
        }
        if let Some(workdir) = workdir {
            scan_skills_root(
                &workdir.join(".claude").join("skills"),
                SkillSource::Claude,
                &mut skills,
            );
            scan_skills_root(
                &workdir.join(".agent").join("skills"),
                SkillSource::ProjectAgent,
                &mut skills,
            );
            scan_skills_root(
                &workdir.join(".agents").join("skills"),
                SkillSource::ProjectAgents,
                &mut skills,
            );
        }
        for root in valid_extra_skill_roots(workdir, extra_skill_roots) {
            scan_skills_root(
                &root.path,
                SkillSource::ExtraRoot(root.config_index),
                &mut skills,
            );
        }
        Self { skills }
    }

    #[cfg(test)]
    pub(super) fn load_isolated(workdir: Option<&Path>, extra_skill_roots: &[PathBuf]) -> Self {
        let mut skills = BTreeMap::new();
        if let Some(workdir) = workdir {
            scan_skills_root(
                &workdir.join(".claude").join("skills"),
                SkillSource::Claude,
                &mut skills,
            );
            scan_skills_root(
                &workdir.join(".agent").join("skills"),
                SkillSource::ProjectAgent,
                &mut skills,
            );
            scan_skills_root(
                &workdir.join(".agents").join("skills"),
                SkillSource::ProjectAgents,
                &mut skills,
            );
        }
        for root in valid_extra_skill_roots(workdir, extra_skill_roots) {
            scan_skills_root(
                &root.path,
                SkillSource::ExtraRoot(root.config_index),
                &mut skills,
            );
        }
        Self { skills }
    }

    pub fn get(&self, name: &str) -> Option<&SkillDef> {
        self.skills.get(&name.to_lowercase())
    }

    pub fn get_by_identity(&self, skill_id: &str) -> Option<&SkillDef> {
        self.skills
            .values()
            .find(|skill| skill.skill_id == skill_id)
    }

    pub fn detail_by_identity(
        &self,
        skill_id: &str,
        generation: &str,
    ) -> anyhow::Result<crate::model::skill::SkillDetail> {
        let skill = self
            .get_by_identity(skill_id)
            .ok_or_else(|| anyhow::anyhow!("Skill source is no longer the current winner"))?;
        if skill.generation != generation {
            anyhow::bail!("Skill changed on disk; refresh and reopen it");
        }
        Ok(crate::model::skill::SkillDetail::from_def(skill))
    }

    pub fn read_companion_text(
        &self,
        skill_id: &str,
        generation: &str,
        path: &str,
    ) -> anyhow::Result<String> {
        let skill = self
            .get_by_identity(skill_id)
            .ok_or_else(|| anyhow::anyhow!("Skill source is no longer the current winner"))?;
        if skill.generation != generation {
            anyhow::bail!("Skill changed on disk; refresh and reopen it");
        }
        let relative =
            relative_companion(path).ok_or_else(|| anyhow::anyhow!("Invalid companion path"))?;
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if !skill.companion_files.iter().any(|file| file == &normalized) {
            anyhow::bail!("Companion file is not in the skill inventory");
        }
        let root = skill
            .skill_dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Flat skills do not have companion files"))?;
        let canonical_root = std::fs::canonicalize(root)?;
        let candidate = std::fs::canonicalize(root.join(relative))?;
        if !candidate.starts_with(&canonical_root) {
            anyhow::bail!("Companion path escapes the skill directory");
        }
        let metadata = std::fs::symlink_metadata(&candidate)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            anyhow::bail!("Companion path is not a regular file");
        }
        if metadata.len() > MAX_SKILL_TEXT_READ_BYTES {
            anyhow::bail!("Companion file exceeds the 2 MiB text-read limit");
        }
        Ok(std::fs::read_to_string(candidate)?)
    }

    pub fn list(&self) -> Vec<&SkillDef> {
        self.skills.values().collect()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn catalogue(&self) -> Vec<SkillCatalogueEntry> {
        self.skills
            .values()
            .map(SkillCatalogueEntry::from_def)
            .collect()
    }

    pub fn catalogue_text(&self) -> String {
        self.skills
            .values()
            .map(|skill| format!("- {}: {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub(crate) fn normalize_skill_root(root: &Path, workdir: Option<&Path>) -> PathBuf {
    let raw = root.to_string_lossy();
    let trimmed = raw.trim();
    let expanded = if trimmed == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed))
    } else if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(trimmed))
    } else {
        PathBuf::from(trimmed)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else if let Some(workdir) = workdir {
        workdir.join(expanded)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(expanded)
    };
    std::fs::canonicalize(&absolute).unwrap_or_else(|_| lexical_normalize(&absolute))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn relative_companion(path: &str) -> Option<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return None;
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(path)
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

/// Canonicalize/deduplicate configured roots and reject any root that could
/// alias, contain, or sit inside a Koma-owned Global/Project skill root.
pub fn valid_extra_skill_roots(
    workdir: Option<&Path>,
    configured: &[PathBuf],
) -> Vec<ValidExtraSkillRoot> {
    let mut owned = Vec::new();
    if let Ok(global) = global_skills_dir() {
        owned.push(normalize_skill_root(&global, workdir));
    }
    if let Some(workdir) = workdir {
        owned.push(normalize_skill_root(
            &workdir.join(".agent").join("skills"),
            Some(workdir),
        ));
        owned.push(normalize_skill_root(
            &workdir.join(".agents").join("skills"),
            Some(workdir),
        ));
    }

    let mut seen = HashSet::new();
    configured
        .iter()
        .enumerate()
        .filter_map(|(config_index, root)| {
            let path = normalize_skill_root(root, workdir);
            if path.as_os_str().is_empty()
                || !path.is_dir()
                || owned.iter().any(|owned| overlaps(&path, owned))
                || !seen.insert(path.clone())
            {
                None
            } else {
                Some(ValidExtraSkillRoot { config_index, path })
            }
        })
        .collect()
}

pub(crate) fn global_skills_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::model::store::base_dir()?.join("skills"))
}

fn scan_skills_root(root: &Path, source: SkillSource, skills: &mut BTreeMap<String, SkillDef>) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() {
            load_flat_skill(&path, source, skills);
        } else if metadata.is_dir() {
            load_dir_skill(&path, source, skills);
        }
    }
}

fn load_flat_skill(path: &Path, source: SkillSource, skills: &mut BTreeMap<String, SkillDef>) {
    let name = match path.file_stem() {
        Some(stem) => stem.to_string_lossy().to_lowercase(),
        None => return,
    };
    if path.extension().is_none_or(|extension| extension != "md")
        || SKIP_NAMES.contains(&name.as_str())
    {
        return;
    }
    match load_skill_file(path, source, None) {
        Ok(skill) => {
            skills.insert(skill.name.clone(), skill);
        }
        Err(error) => crate::model::store::append_global_error_log(
            "skill registry",
            &format!("skipped {}: {error}", path.display()),
        ),
    }
}

fn load_dir_skill(dir: &Path, source: SkillSource, skills: &mut BTreeMap<String, SkillDef>) {
    let entry_file = SKILL_ENTRY_FILES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| {
            std::fs::symlink_metadata(path)
                .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                .unwrap_or(false)
        });
    let Some(entry_path) = entry_file else {
        return;
    };

    match load_skill_file(&entry_path, source, Some(dir.to_path_buf())) {
        Ok(skill) => {
            skills.insert(skill.name.clone(), skill);
        }
        Err(error) => crate::model::store::append_global_error_log(
            "skill registry",
            &format!("skipped {}: {error}", entry_path.display()),
        ),
    }
}

#[cfg(test)]
pub(crate) fn scan_skills_root_for_test(
    root: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
) {
    scan_skills_root(root, source, skills);
}

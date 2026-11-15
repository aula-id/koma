//! Skill discovery: scan known directories for skill files and build a registry.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use super::def::{SkillCatalogueEntry, SkillDef, SkillSource};
use super::search::{self, Hit, Scope};
use super::parse::load_skill_file;

const SKILL_ENTRY_FILES: &[&str] = &["SKILL.md", "skill.md"];
const SKIP_NAMES: &[&str] = &["readme.md", "readme"];
/// Most pinned skills the prompt shows.
pub const PINNED_MAX: usize = 8;
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
        let global = global_skills_dir().ok();
        Self::load_tiers(global.as_deref(), workdir, extra_skill_roots, &mut SkipSink::Log)
    }

    #[cfg(test)]
    pub(super) fn load_isolated(workdir: Option<&Path>, extra_skill_roots: &[PathBuf]) -> Self {
        Self::load_tiers(None, workdir, extra_skill_roots, &mut SkipSink::Log)
    }

    /// Scan every tier in precedence order into one registry. `global` is the
    /// Global root, or `None` to leave that tier out.
    fn load_tiers(
        global: Option<&Path>,
        workdir: Option<&Path>,
        extra_skill_roots: &[PathBuf],
        sink: &mut SkipSink<'_>,
    ) -> Self {
        let mut skills = BTreeMap::new();
        if let Some(global) = global {
            scan_skills_root(global, SkillSource::Global, &mut skills, sink);
        }
        if let Some(workdir) = workdir {
            scan_skills_root(
                &workdir.join(".claude").join("skills"),
                SkillSource::Claude,
                &mut skills,
                sink,
            );
            scan_skills_root(
                &workdir.join(".agent").join("skills"),
                SkillSource::ProjectAgent,
                &mut skills,
                sink,
            );
            scan_skills_root(
                &workdir.join(".agents").join("skills"),
                SkillSource::ProjectAgents,
                &mut skills,
                sink,
            );
        }
        for root in valid_extra_skill_roots(workdir, extra_skill_roots) {
            scan_skills_root(
                &root.path,
                SkillSource::ExtraRoot(root.config_index),
                &mut skills,
                sink,
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

    /// The best matches for a request, most relevant first, and how many matched in all. Only skills
    /// inside `scope` are considered. An empty query matches nothing: skills are asked for, not listed.
    pub fn search(&self, query: &str, scope: &Scope, offset: usize, limit: usize) -> (Vec<Hit>, usize) {
        let words = search::words(query);
        let whole = query.trim().to_lowercase();
        let mut hits: Vec<Hit> = self
            .skills
            .values()
            .filter(|d| scope.allows(&d.name))
            .filter_map(|d| {
                let score = search::score(d, &words, &whole);
                (score > 0).then(|| Hit { name: d.name.clone(), score })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
        let total = hits.len();
        (hits.into_iter().skip(offset).take(limit).collect(), total)
    }

    /// Skills the user marked `pinned: true`, at most `max`, inside `scope`.
    pub fn pinned(&self, scope: &Scope, max: usize) -> Vec<&SkillDef> {
        self.skills.values().filter(|d| scope.allows(&d.name) && search::is_pinned(d)).take(max).collect()
    }

    /// A registry of just `<root>/.agents/skills`, for tests in other modules.
    #[cfg(test)]
    pub fn load_for_test(root: &Path) -> Self {
        Self::load_isolated(Some(root), &[])
    }

    #[cfg(test)]
    pub(super) fn from_defs(defs: Vec<SkillDef>) -> Self {
        Self { skills: defs.into_iter().map(|d| (d.name.clone(), d)).collect() }
    }

    /// The skills section of the system prompt: `None` when there are no skills at all, otherwise only
    /// the pinned ones (possibly an empty string). The rest are found with `skill_find`, never listed.
    pub fn prompt_text(&self) -> Option<String> {
        if self.skills.is_empty() {
            return None;
        }
        Some(
            self.pinned(&Scope::default(), PINNED_MAX)
                .iter()
                .map(|d| format!("- {}: {}", d.name, d.description))
                .collect::<Vec<_>>()
                .join("\n"),
        )
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

/// A skill entry that discovery found but could not load.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // read by the SDK skill listing, which is not wired up yet
pub(crate) struct SkippedSkill {
    /// The flat skill file or the `SKILL.md` of a dir-form skill.
    pub path: PathBuf,
    /// Why it was skipped.
    pub error: String,
}

/// Where scanning reports skills it could not load.
enum SkipSink<'a> {
    /// Append to the global error log (what `SkillRegistry::load` does).
    Log,
    /// Collect for the caller, without logging.
    Collect(&'a mut Vec<SkippedSkill>),
}

impl SkipSink<'_> {
    fn record(&mut self, path: &Path, error: &anyhow::Error) {
        match self {
            Self::Log => crate::model::store::append_global_error_log(
                "skill registry",
                &format!("skipped {}: {error}", path.display()),
            ),
            Self::Collect(skipped) => skipped.push(SkippedSkill {
                path: path.to_path_buf(),
                error: error.to_string(),
            }),
        }
    }
}

/// Load the same tiers as [`SkillRegistry::load`] and return the skills that
/// could not be loaded with their path and error, instead of logging them.
#[allow(dead_code)] // called by the SDK skill listing, which is not wired up yet
pub(crate) fn load_with_diagnostics(
    workdir: Option<&Path>,
    extra_skill_roots: &[PathBuf],
) -> (SkillRegistry, Vec<SkippedSkill>) {
    let global = global_skills_dir().ok();
    let mut skipped = Vec::new();
    let registry = SkillRegistry::load_tiers(
        global.as_deref(),
        workdir,
        extra_skill_roots,
        &mut SkipSink::Collect(&mut skipped),
    );
    (registry, skipped)
}

/// Load the skills of one discovery root on its own, ordered by name, so a
/// skill that a later tier shadows in the merged registry is still reachable.
/// Skills that fail to load are left out without logging; the merged load
/// reports them. An absent root gives an empty list.
#[allow(dead_code)] // called by the SDK skill listing, which is not wired up yet
pub(crate) fn load_root(root: &Path, source: SkillSource) -> Vec<SkillDef> {
    let mut skills = BTreeMap::new();
    let mut skipped = Vec::new();
    scan_skills_root(
        root,
        source,
        &mut skills,
        &mut SkipSink::Collect(&mut skipped),
    );
    skills.into_values().collect()
}

fn scan_skills_root(
    root: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
    sink: &mut SkipSink<'_>,
) {
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
            load_flat_skill(&path, source, skills, sink);
        } else if metadata.is_dir() {
            load_dir_skill(&path, source, skills, sink);
        }
    }
}

fn load_flat_skill(
    path: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
    sink: &mut SkipSink<'_>,
) {
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
        Err(error) => sink.record(path, &error),
    }
}

fn load_dir_skill(
    dir: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
    sink: &mut SkipSink<'_>,
) {
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
        Err(error) => sink.record(&entry_path, &error),
    }
}

#[cfg(test)]
pub(crate) fn scan_skills_root_for_test(
    root: &Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, SkillDef>,
) {
    scan_skills_root(root, source, skills, &mut SkipSink::Log);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod root_tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koma-skill-root-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(root: &Path, name: &str, description: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\ndescription: {description}\n---\nBody.\n"),
        )
        .unwrap();
    }

    #[test]
    fn load_root_lists_skill_shadowed_by_later_tier() {
        let workdir = temp_dir();
        let agent = workdir.join(".agent").join("skills");
        let agents = workdir.join(".agents").join("skills");
        write_skill(&agent, "demo", "from agent");
        write_skill(&agents, "demo", "from agents");

        let merged = SkillRegistry::load_isolated(Some(&workdir), &[]);
        let winner = merged.get("demo").unwrap();
        assert_eq!(winner.source, SkillSource::ProjectAgents);
        assert_eq!(winner.description, "from agents");

        let shadowed = load_root(&agent, SkillSource::ProjectAgent);
        assert_eq!(shadowed.len(), 1);
        assert_eq!(shadowed[0].name, "demo");
        assert_eq!(shadowed[0].source, SkillSource::ProjectAgent);
        assert_eq!(shadowed[0].description, "from agent");

        std::fs::remove_dir_all(&workdir).unwrap();
    }

    #[test]
    fn diagnostics_list_invalid_skill_and_valid_ones_load() {
        let workdir = temp_dir();
        let root = workdir.join(".agents").join("skills");
        write_skill(&root, "good", "works");
        let bad_dir = root.join("bad");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("SKILL.md"), "no frontmatter here\n").unwrap();

        let mut skipped = Vec::new();
        let registry = SkillRegistry::load_tiers(
            None,
            Some(&workdir),
            &[],
            &mut SkipSink::Collect(&mut skipped),
        );
        assert!(registry.get("good").is_some());
        assert!(registry.get("bad").is_none());
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].path, bad_dir.join("SKILL.md"));
        assert!(!skipped[0].error.is_empty());

        std::fs::remove_dir_all(&workdir).unwrap();
    }

    #[test]
    fn absent_root_is_empty() {
        let workdir = temp_dir();
        let missing = workdir.join("missing");
        assert!(load_root(&missing, SkillSource::Global).is_empty());

        let mut skipped = Vec::new();
        let registry = SkillRegistry::load_tiers(
            None,
            Some(&missing),
            &[],
            &mut SkipSink::Collect(&mut skipped),
        );
        assert!(registry.is_empty());
        assert!(skipped.is_empty());

        std::fs::remove_dir_all(&workdir).unwrap();
    }
}

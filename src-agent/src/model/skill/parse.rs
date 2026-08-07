//! Frontmatter parsing for skill files.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use super::def::{SkillDef, SkillSource};

/// Deserialize a YAML frontmatter chunk into the description field.
/// Extra keys are silently ignored (parse-tolerant).
fn parse_frontmatter_description(yaml_str: &str) -> String {
    for line in yaml_str.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("description:") {
            return v.trim().to_string();
        }
    }
    String::new()
}

/// Parse a skill `.md` string into a [`SkillDef`].
///
/// `name` is the canonical name (lowercased, validated stem/dir name).
/// `source`, `file_path`, and `skill_dir` are caller-supplied.
pub(crate) fn parse_skill(
    name: &str,
    content: &str,
    source: SkillSource,
    file_path: PathBuf,
    skill_dir: Option<PathBuf>,
) -> Result<SkillDef> {
    let (fm_str, body) = crate::model::agent_def::split_frontmatter(content)?;

    let description = match fm_str {
        Some(fm) => parse_frontmatter_description(fm),
        None => String::new(),
    };

    if description.trim().is_empty() {
        return Err(anyhow!("skill '{}' missing required 'description'", name));
    }

    Ok(SkillDef {
        name: name.to_string(),
        description,
        body: body.trim().to_string(),
        source,
        file_path,
        skill_dir,
    })
}

/// Validate a skill name: lowercase ASCII alphanumeric + dash, no traversal,
/// no leading/trailing dash, non-empty.
pub(crate) fn validate_skill_name(stem: &str) -> Result<String> {
    crate::model::agent_def::validate_agent_name(stem)
}

/// Load and parse a single skill `.md` file from disk.
///
/// `skill_dir` is set to `Some(parent_dir)` when this is a dir-form entry
/// (e.g. `bar/SKILL.md`), `None` for flat files (e.g. `foo.md`).
pub(crate) fn load_skill_file(
    path: &Path,
    source: SkillSource,
    skill_dir: Option<PathBuf>,
) -> Result<SkillDef> {
    let content = std::fs::read_to_string(path)?;
    // For dir-form entries (skill_dir is Some), the canonical name comes from
    // the DIRECTORY name (e.g. "bar" from bar/SKILL.md), not the file stem
    // (which would be "skill"). For flat files, use the file stem.
    let stem = if let Some(ref dir) = skill_dir {
        dir.file_name()
            .ok_or_else(|| anyhow!("no directory name"))?
            .to_string_lossy()
            .into_owned()
    } else {
        path.file_stem()
            .ok_or_else(|| anyhow!("no filename"))?
            .to_string_lossy()
            .into_owned()
    };

    let name = validate_skill_name(&stem)?;
    parse_skill(&name, &content, source, path.to_path_buf(), skill_dir)
}

//! Frontmatter parsing for skill files.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

use super::def::{SkillDef, SkillSource};

const MAX_COMPANION_DEPTH: usize = 16;
const MAX_COMPANION_FILES: usize = 1_000;
const MAX_SKILL_ENTRY_BYTES: u64 = 2 * 1024 * 1024;

fn scalar_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn mapping_value<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::String(key.to_string()))
}

fn parse_declared_tools(value: Option<&Value>) -> Vec<String> {
    let mut tools = match value {
        Some(Value::Sequence(values)) => values
            .iter()
            .filter_map(|value| match value {
                Value::String(tool) => Some(tool.trim().to_string()),
                _ => None,
            })
            .collect(),
        Some(Value::String(value)) => value
            .split([',', '\n'])
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    tools.sort();
    tools.dedup();
    tools
}

/// Conservative O1 guard: ordinary mappings are semantically writable, while
/// constructs serde_yaml_ng cannot guarantee to preserve are read-only in the
/// structured editor. The original raw frontmatter is always retained.
pub(crate) fn frontmatter_roundtrip_safe(raw: &str, mapping: Option<&Mapping>) -> bool {
    let Some(mapping) = mapping else {
        return raw.trim().is_empty();
    };
    if serde_yaml_ng::from_str::<Mapping>(raw).ok().as_ref() != Some(mapping) {
        return false;
    }
    for line in raw.lines() {
        let code = line.split('#').next().unwrap_or_default();
        let mut single = false;
        let mut double = false;
        let chars: Vec<char> = code.chars().collect();
        for (index, ch) in chars.iter().copied().enumerate() {
            match ch {
                '\'' if !double => single = !single,
                '"' if !single && (index == 0 || chars[index - 1] != '\\') => double = !double,
                '&' | '*' | '!' if !single && !double => {
                    let previous_is_token = index == 0
                        || chars[index - 1].is_whitespace()
                        || matches!(chars[index - 1], ':' | '[' | '{' | ',' | '-');
                    if previous_is_token {
                        return false;
                    }
                }
                _ => {}
            }
        }
    }
    true
}

pub(super) fn generation_for_bytes(content: &[u8]) -> String {
    let mut generation = Sha256::new();
    generation.update(b"koma-skill-generation-v1\0");
    generation.update(content);
    format!("{:x}", generation.finalize())
}

fn source_identity(source: SkillSource, path: &Path, content: &[u8]) -> (String, String) {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let source_tag = format!("{source:?}");
    let mut identity = Sha256::new();
    identity.update(b"koma-skill-id-v1\0");
    identity.update(source_tag.as_bytes());
    identity.update(b"\0");
    identity.update(canonical.to_string_lossy().as_bytes());
    (
        format!("{:x}", identity.finalize()),
        generation_for_bytes(content),
    )
}

fn collect_companion_files(skill_dir: Option<&Path>) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
        if depth > MAX_COMPANION_DEPTH || out.len() >= MAX_COMPANION_FILES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if out.len() >= MAX_COMPANION_FILES {
                return;
            }
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                visit(root, &path, depth + 1, out);
            } else if metadata.is_file() {
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                let display = relative.to_string_lossy().replace('\\', "/");
                if !display.eq_ignore_ascii_case("SKILL.md") {
                    out.push(display);
                }
            }
        }
    }
    let Some(root) = skill_dir else {
        return Vec::new();
    };
    let mut files = Vec::new();
    visit(root, root, 0, &mut files);
    files.sort();
    files
}

/// Parse a skill `.md` string into a [`SkillDef`].
pub(crate) fn parse_skill(
    name: &str,
    content: &str,
    source: SkillSource,
    file_path: PathBuf,
    skill_dir: Option<PathBuf>,
) -> Result<SkillDef> {
    let (fm_str, body) = crate::model::agent_def::split_frontmatter(content)?;
    let raw_frontmatter = fm_str.map(ToOwned::to_owned);
    let frontmatter = fm_str.and_then(|raw| serde_yaml_ng::from_str::<Mapping>(raw).ok());

    let description = frontmatter
        .as_ref()
        .map(|mapping| scalar_string(mapping_value(mapping, "description")))
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            // Keep discovery tolerant of advanced YAML the structured editor
            // refuses to rewrite; the source bytes remain the authority.
            fm_str.and_then(|raw| {
                raw.lines().find_map(|line| {
                    line.trim()
                        .strip_prefix("description:")
                        .map(|value| value.trim().trim_matches(['\'', '"']).to_string())
                })
            })
        })
        .unwrap_or_default();

    if description.trim().is_empty() {
        return Err(anyhow!("skill '{}' missing required 'description'", name));
    }

    let triggers = frontmatter
        .as_ref()
        .map(|mapping| scalar_string(mapping_value(mapping, "triggers")))
        .unwrap_or_default();
    let allowed_tools = frontmatter
        .as_ref()
        .map(|mapping| parse_declared_tools(mapping_value(mapping, "allowed-tools")))
        .unwrap_or_default();
    let frontmatter_roundtrip_safe = fm_str
        .map(|raw| frontmatter_roundtrip_safe(raw, frontmatter.as_ref()))
        .unwrap_or(true);
    let companion_files = collect_companion_files(skill_dir.as_deref());
    let (skill_id, generation) = source_identity(source, &file_path, content.as_bytes());

    Ok(SkillDef {
        name: name.to_string(),
        description,
        triggers,
        allowed_tools,
        body: body.trim().to_string(),
        source,
        file_path,
        skill_dir,
        companion_files,
        frontmatter,
        raw_frontmatter,
        frontmatter_roundtrip_safe,
        skill_id,
        generation,
    })
}

/// Validate a skill name: lowercase ASCII alphanumeric + dash, no traversal,
/// no leading/trailing dash, non-empty.
pub(crate) fn validate_skill_name(stem: &str) -> Result<String> {
    crate::model::agent_def::validate_agent_name(stem)
}

/// Load and parse a single skill `.md` file from disk.
pub(crate) fn load_skill_file(
    path: &Path,
    source: SkillSource,
    skill_dir: Option<PathBuf>,
) -> Result<SkillDef> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("SKILL.md is not a regular file"));
    }
    if metadata.len() > MAX_SKILL_ENTRY_BYTES {
        return Err(anyhow!("SKILL.md exceeds the 2 MiB text-read limit"));
    }
    let content = std::fs::read_to_string(path)?;
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

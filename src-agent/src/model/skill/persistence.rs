//! Ownership-checked skill persistence and bounded duplication.

use std::fs::OpenOptions;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

use super::def::SkillDef;
use super::parse::{generation_for_bytes, load_skill_file, parse_skill, validate_skill_name};
use super::registry::global_skills_dir;
use super::SkillRegistry;

pub const MAX_DUPLICATE_DEPTH: usize = 16;
pub const MAX_DUPLICATE_FILES: usize = 1_000;
pub const MAX_DUPLICATE_COMPANION_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_DUPLICATE_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SKILL_ENTRY_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedSkillTarget {
    Global,
    Project,
}

impl OwnedSkillTarget {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "global" => Ok(Self::Global),
            "project" => Ok(Self::Project),
            _ => bail!("Skill target must be 'global' or 'project'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEdit {
    pub description: String,
    pub triggers: String,
    pub allowed_tools: Vec<String>,
    pub instruction: String,
}

#[derive(Debug, Clone)]
struct InventoryFile {
    relative: PathBuf,
    bytes: Vec<u8>,
    entry: bool,
}

#[derive(Debug, Clone)]
struct SourceInventory {
    files: Vec<InventoryFile>,
    digest: String,
}

fn validate_edit(edit: &SkillEdit) -> Result<()> {
    if edit.description.trim().is_empty() {
        bail!("Skill description is required");
    }
    Ok(())
}

fn target_root(workdir: Option<&Path>, target: OwnedSkillTarget) -> Result<PathBuf> {
    match target {
        OwnedSkillTarget::Global => global_skills_dir(),
        OwnedSkillTarget::Project => workdir
            .map(|path| path.join(".agents").join("skills"))
            .ok_or_else(|| anyhow!("Project scope requires an active project")),
    }
}

fn canonical_owned_root(workdir: Option<&Path>, target: OwnedSkillTarget) -> Result<PathBuf> {
    let root = target_root(workdir, target)?;
    std::fs::create_dir_all(&root)
        .with_context(|| format!("create owned skill root {}", root.display()))?;
    if std::fs::symlink_metadata(&root)?.file_type().is_symlink() {
        bail!("Owned skill root cannot be a symlink");
    }
    std::fs::canonicalize(&root)
        .with_context(|| format!("resolve owned skill root {}", root.display()))
}

fn checked_name(name: &str) -> Result<String> {
    let name = validate_skill_name(name.trim())?;
    if matches!(name.as_str(), "skill" | "readme") {
        bail!("Reserved skill name '{name}'");
    }
    Ok(name)
}

fn set_string(mapping: &mut Mapping, key: &str, value: String) {
    mapping.insert(Value::String(key.to_string()), Value::String(value));
}

fn render_skill(mapping: &Mapping, instruction: &str) -> Result<Vec<u8>> {
    let yaml = serde_yaml_ng::to_string(mapping)?;
    let yaml = yaml.trim_start_matches("---\n").trim_end();
    Ok(format!("---\n{yaml}\n---\n{}\n", instruction.trim_end()).into_bytes())
}

fn canonical_skill_bytes(edit: &SkillEdit) -> Result<Vec<u8>> {
    validate_edit(edit)?;
    let mut mapping = Mapping::new();
    set_string(
        &mut mapping,
        "description",
        edit.description.trim().to_string(),
    );
    if !edit.triggers.trim().is_empty() {
        set_string(&mut mapping, "triggers", edit.triggers.trim().to_string());
    }
    if !edit.allowed_tools.is_empty() {
        mapping.insert(
            Value::String("allowed-tools".to_string()),
            Value::Sequence(
                edit.allowed_tools
                    .iter()
                    .map(|tool| Value::String(tool.trim().to_string()))
                    .filter(|tool| !matches!(tool, Value::String(value) if value.is_empty()))
                    .collect(),
            ),
        );
    }
    render_skill(&mapping, &edit.instruction)
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Skill path has no parent"))?;
    let temp = parent.join(format!(".SKILL.md.koma-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn reserve_destination(root: &Path, name: &str) -> Result<PathBuf> {
    let destination = root.join(name);
    std::fs::create_dir(&destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow!("A skill named '{name}' already exists in the destination")
        } else {
            anyhow!(error).context(format!("reserve {}", destination.display()))
        }
    })?;
    Ok(destination)
}

fn revalidate_reserved_destination(root: &Path, destination: &Path) -> Result<()> {
    let root_metadata = std::fs::symlink_metadata(root)?;
    let destination_metadata = std::fs::symlink_metadata(destination)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!("Destination skill root is no longer a regular directory");
    }
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        bail!("Reserved skill destination is no longer a regular directory");
    }
    let canonical_root = std::fs::canonicalize(root)?;
    let canonical_destination = std::fs::canonicalize(destination)?;
    if canonical_destination.parent() != Some(canonical_root.as_path()) {
        bail!("Reserved skill destination escaped its root");
    }
    Ok(())
}

fn revalidate_descendant_directory(destination: &Path, directory: &Path) -> Result<()> {
    let relative = directory.strip_prefix(destination)?;
    let mut current = destination.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("Invalid destination companion path");
        }
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("Destination companion directory is not a regular directory");
        }
    }
    Ok(())
}

fn remove_reserved(destination: &Path) {
    let _ = std::fs::remove_dir_all(destination);
}

pub fn create_owned_skill(
    workdir: Option<&Path>,
    target: OwnedSkillTarget,
    name: &str,
    edit: &SkillEdit,
) -> Result<PathBuf> {
    let name = checked_name(name)?;
    let bytes = canonical_skill_bytes(edit)?;
    if bytes.len() as u64 > MAX_SKILL_ENTRY_BYTES {
        bail!("SKILL.md exceeds the 2 MiB text limit");
    }
    let root = canonical_owned_root(workdir, target)?;
    let destination = reserve_destination(&root, &name)?;
    if let Err(error) = revalidate_reserved_destination(&root, &destination) {
        remove_reserved(&destination);
        return Err(error);
    }
    let entry = destination.join("SKILL.md");
    if let Err(error) = write_new_file(&entry, &bytes) {
        remove_reserved(&destination);
        return Err(error);
    }
    Ok(entry)
}

fn revalidate_owned_entry(skill: &SkillDef) -> Result<()> {
    let source_root = match skill.skill_dir.as_ref() {
        Some(directory) => directory
            .parent()
            .ok_or_else(|| anyhow!("Skill directory has no owned root"))?,
        None => skill
            .file_path
            .parent()
            .ok_or_else(|| anyhow!("Skill file has no owned root"))?,
    };
    let root_metadata = std::fs::symlink_metadata(source_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!("Owned skill root is no longer a regular directory");
    }
    if let Some(directory) = skill.skill_dir.as_ref() {
        let metadata = std::fs::symlink_metadata(directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("Owned skill directory is no longer a regular directory");
        }
    }
    let entry_metadata = std::fs::symlink_metadata(&skill.file_path)?;
    if entry_metadata.file_type().is_symlink() || !entry_metadata.is_file() {
        bail!("Owned SKILL.md is no longer a regular file");
    }
    let canonical_root = std::fs::canonicalize(source_root)?;
    let canonical_entry = std::fs::canonicalize(&skill.file_path)?;
    if !canonical_entry.starts_with(&canonical_root) {
        bail!("Owned skill path escapes its root");
    }
    Ok(())
}

pub fn update_owned_skill(
    registry: &SkillRegistry,
    skill_id: &str,
    generation: &str,
    edit: &SkillEdit,
) -> Result<PathBuf> {
    update_owned_skill_with_generation(registry, skill_id, generation, edit).map(|(path, _)| path)
}

/// Return the generation of the exact rendered bytes written, not a subsequent
/// disk re-read that could already contain someone else's concurrent edit.
pub(crate) fn update_owned_skill_with_generation(
    registry: &SkillRegistry,
    skill_id: &str,
    generation: &str,
    edit: &SkillEdit,
) -> Result<(PathBuf, String)> {
    validate_edit(edit)?;
    let skill = current_skill(registry, skill_id, generation)?;
    if !skill.source.editable() {
        bail!("External skills are read-only; duplicate the skill into Koma first");
    }
    revalidate_owned_entry(skill)?;
    if !skill.frontmatter_roundtrip_safe {
        bail!("This skill uses advanced YAML that Koma cannot safely round-trip; edit SKILL.md manually");
    }
    let mut mapping = skill.frontmatter.clone().unwrap_or_default();
    set_string(
        &mut mapping,
        "description",
        edit.description.trim().to_string(),
    );
    if edit.triggers.trim().is_empty() {
        mapping.remove(Value::String("triggers".to_string()));
    } else {
        set_string(&mut mapping, "triggers", edit.triggers.trim().to_string());
    }
    if edit.allowed_tools.is_empty() {
        mapping.remove(Value::String("allowed-tools".to_string()));
    } else {
        let mut tools: Vec<_> = edit
            .allowed_tools
            .iter()
            .map(|tool| tool.trim())
            .filter(|tool| !tool.is_empty())
            .map(|tool| Value::String(tool.to_string()))
            .collect();
        tools.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        tools.dedup();
        mapping.insert(
            Value::String("allowed-tools".to_string()),
            Value::Sequence(tools),
        );
    }
    let bytes = render_skill(&mapping, &edit.instruction)?;
    if bytes.len() as u64 > MAX_SKILL_ENTRY_BYTES {
        bail!("SKILL.md exceeds the 2 MiB text limit");
    }

    // Re-read immediately before publication to reject a stale winner/body.
    let current = load_skill_file(&skill.file_path, skill.source, skill.skill_dir.clone())?;
    if current.skill_id != skill_id || current.generation != generation {
        bail!("Skill changed on disk; refresh before saving");
    }
    revalidate_owned_entry(skill)?;
    let saved_generation = generation_for_bytes(&bytes);
    atomic_replace(&skill.file_path, &bytes)?;
    Ok((skill.file_path.clone(), saved_generation))
}

pub fn delete_owned_skill(
    registry: &SkillRegistry,
    skill_id: &str,
    generation: &str,
) -> Result<PathBuf> {
    let skill = current_skill(registry, skill_id, generation)?;
    if !skill.source.editable() {
        bail!("External skills cannot be deleted");
    }
    revalidate_owned_entry(skill)?;
    let path = skill
        .skill_dir
        .clone()
        .unwrap_or_else(|| skill.file_path.clone());
    reject_unsafe_tree(&path, 0, &mut 0usize)?;
    let current = load_skill_file(&skill.file_path, skill.source, skill.skill_dir.clone())?;
    if current.skill_id != skill_id || current.generation != generation {
        bail!("Skill changed on disk; refresh before deleting");
    }
    revalidate_owned_entry(skill)?;
    if path.is_dir() {
        std::fs::remove_dir_all(&path)?;
    } else {
        std::fs::remove_file(&path)?;
    }
    Ok(path)
}

pub fn install_owned_skill_zip(
    workdir: Option<&Path>,
    target: OwnedSkillTarget,
    name: &str,
    archive_bytes: &[u8],
) -> Result<PathBuf> {
    if archive_bytes.len() as u64 > MAX_DUPLICATE_TOTAL_BYTES {
        bail!("ZIP package exceeds the 64 MiB upload limit");
    }
    let name = checked_name(name)?;
    let mut archive =
        zip::ZipArchive::new(Cursor::new(archive_bytes)).context("open skill ZIP package")?;
    if archive.len() > MAX_DUPLICATE_FILES + MAX_DUPLICATE_DEPTH + 1 {
        bail!("ZIP package has too many entries");
    }

    struct ArchiveFile {
        path: PathBuf,
        bytes: Vec<u8>,
    }
    let mut files = Vec::new();
    let mut paths = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("ZIP contains an unsafe path"))?;
        if path.as_os_str().is_empty() {
            continue;
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o040000 && kind != 0o100000 {
                bail!(
                    "ZIP contains a symbolic link or special file: {}",
                    path.display()
                );
            }
        }
        paths.push((path.clone(), entry.is_dir()));
        if entry.is_dir() {
            continue;
        }
        if files.len() >= MAX_DUPLICATE_FILES {
            bail!("ZIP package exceeds the 1,000-file limit");
        }
        let is_entry = path.file_name().is_some_and(|part| part == "SKILL.md");
        let limit = if is_entry {
            MAX_SKILL_ENTRY_BYTES
        } else {
            MAX_DUPLICATE_COMPANION_BYTES
        };
        if entry.size() > limit {
            bail!("ZIP entry exceeds its byte limit: {}", path.display());
        }
        let mut bytes = Vec::new();
        entry.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            bail!("ZIP entry exceeds its byte limit: {}", path.display());
        }
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| anyhow!("ZIP uncompressed size overflow"))?;
        if total > MAX_DUPLICATE_TOTAL_BYTES {
            bail!("ZIP package exceeds the 64 MiB uncompressed limit");
        }
        files.push(ArchiveFile { path, bytes });
    }

    let root_entry = Path::new("SKILL.md");
    let has_root_entry = files.iter().any(|file| file.path == root_entry);
    let strip_prefix = if has_root_entry {
        None
    } else {
        let mut top = paths.iter().filter_map(|(path, _)| {
            path.components()
                .next()
                .and_then(|component| match component {
                    Component::Normal(part) => Some(part.to_os_string()),
                    _ => None,
                })
        });
        let first = top.next().ok_or_else(|| anyhow!("ZIP package is empty"))?;
        if top.any(|part| part != first) {
            bail!("ZIP must contain SKILL.md at its root or under one top-level directory");
        }
        let prefix = PathBuf::from(first);
        if !files
            .iter()
            .any(|file| file.path == prefix.join("SKILL.md"))
        {
            bail!("ZIP must contain SKILL.md at its root or under one top-level directory");
        }
        Some(prefix)
    };

    let mut publish = Vec::with_capacity(files.len());
    for file in files {
        let relative = match &strip_prefix {
            Some(prefix) => file.path.strip_prefix(prefix)?.to_path_buf(),
            None => file.path,
        };
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!("ZIP contains an invalid relative path");
        }
        let depth = relative
            .parent()
            .map_or(0, |parent| parent.components().count());
        if depth > MAX_DUPLICATE_DEPTH {
            bail!("ZIP package exceeds the maximum depth of 16");
        }
        publish.push(ArchiveFile {
            path: relative,
            bytes: file.bytes,
        });
    }
    publish.sort_by(|left, right| left.path.cmp(&right.path));
    if publish.windows(2).any(|pair| pair[0].path == pair[1].path) {
        bail!("ZIP contains duplicate output paths");
    }
    let entry = publish
        .iter()
        .find(|file| file.path == root_entry)
        .ok_or_else(|| anyhow!("ZIP package has no publishable SKILL.md"))?;
    let entry_text = std::str::from_utf8(&entry.bytes).context("SKILL.md must be UTF-8")?;

    let root = canonical_owned_root(workdir, target)?;
    let destination = reserve_destination(&root, &name)?;
    let result = (|| -> Result<PathBuf> {
        revalidate_reserved_destination(&root, &destination)?;
        parse_skill(
            &name,
            entry_text,
            match target {
                OwnedSkillTarget::Global => super::SkillSource::Global,
                OwnedSkillTarget::Project => super::SkillSource::ProjectAgents,
            },
            destination.join("SKILL.md"),
            Some(destination.clone()),
        )?;
        for file in publish.iter().filter(|file| file.path != root_entry) {
            revalidate_reserved_destination(&root, &destination)?;
            let output = destination.join(&file.path);
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
                revalidate_descendant_directory(&destination, parent)?;
            }
            write_new_file(&output, &file.bytes)?;
        }
        revalidate_reserved_destination(&root, &destination)?;
        let entry_path = destination.join("SKILL.md");
        write_new_file(&entry_path, &entry.bytes)?;
        Ok(entry_path)
    })();
    if result.is_err() {
        remove_reserved(&destination);
    }
    result
}

pub fn export_owned_skill_zip(
    registry: &SkillRegistry,
    skill_id: &str,
    generation: &str,
    save_path: &Path,
) -> Result<PathBuf> {
    let source = current_skill(registry, skill_id, generation)?;
    if !source.source.editable() {
        bail!("External skills cannot be exported; duplicate the skill to Koma first");
    }
    revalidate_owned_entry(source)?;
    let inventory = inventory_source(source)?;
    let current = load_skill_file(&source.file_path, source.source, source.skill_dir.clone())?;
    if current.skill_id != skill_id || current.generation != generation {
        bail!("Skill changed on disk; refresh before downloading");
    }
    revalidate_owned_entry(source)?;
    let parent = save_path
        .parent()
        .ok_or_else(|| anyhow!("ZIP destination has no parent"))?;
    if !parent.is_dir() {
        bail!("ZIP destination directory does not exist");
    }
    let temp = parent.join(format!(
        ".koma-skill-export-{}.zip.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<PathBuf> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for item in &inventory.files {
            let name = item.relative.to_string_lossy().replace('\\', "/");
            writer.start_file(name, options)?;
            writer.write_all(&item.bytes)?;
        }
        let file = writer.finish()?;
        file.sync_all()?;
        std::fs::rename(&temp, save_path)?;
        Ok(save_path.to_path_buf())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub fn duplicate_to_owned(
    registry: &SkillRegistry,
    skill_id: &str,
    generation: &str,
    workdir: Option<&Path>,
    target: OwnedSkillTarget,
    destination_name: &str,
) -> Result<PathBuf> {
    let source = current_skill(registry, skill_id, generation)?;
    if source.source.editable() {
        bail!("Duplicate to Koma is only available for External skills");
    }
    let name = checked_name(destination_name)?;
    let root = canonical_owned_root(workdir, target)?;
    let root_identity = std::fs::canonicalize(&root)?;
    let inventory = inventory_source(source)?;
    let destination = reserve_destination(&root_identity, &name)?;

    let result = (|| -> Result<PathBuf> {
        revalidate_reserved_destination(&root_identity, &destination)?;
        for file in inventory.files.iter().filter(|file| !file.entry) {
            revalidate_reserved_destination(&root_identity, &destination)?;
            let output = destination.join(&file.relative);
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
                revalidate_descendant_directory(&destination, parent)?;
            }
            write_new_file(&output, &file.bytes)?;
        }

        let after = inventory_source(source)?;
        if after.digest != inventory.digest {
            bail!("Skill source changed while it was being duplicated");
        }
        if std::fs::canonicalize(&root)? != root_identity {
            bail!("Destination skill root changed during duplication");
        }
        let entry = inventory
            .files
            .iter()
            .find(|file| file.entry)
            .ok_or_else(|| anyhow!("Source skill entry disappeared"))?;
        revalidate_reserved_destination(&root_identity, &destination)?;
        let entry_path = destination.join("SKILL.md");
        write_new_file(&entry_path, &entry.bytes)?;
        Ok(entry_path)
    })();

    if result.is_err() {
        remove_reserved(&destination);
    }
    result
}

fn current_skill<'a>(
    registry: &'a SkillRegistry,
    skill_id: &str,
    generation: &str,
) -> Result<&'a SkillDef> {
    let skill = registry
        .get_by_identity(skill_id)
        .ok_or_else(|| anyhow!("Skill source is no longer the current winner"))?;
    if skill.generation != generation {
        bail!("Skill changed on disk; refresh and retry");
    }
    Ok(skill)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn inventory_source(skill: &SkillDef) -> Result<SourceInventory> {
    let mut files = Vec::new();
    let mut total = 0u64;
    if let Some(root) = &skill.skill_dir {
        inventory_dir(root, root, 0, &mut total, &mut files)?;
    } else {
        let bytes = std::fs::read(&skill.file_path)?;
        if bytes.len() as u64 > MAX_SKILL_ENTRY_BYTES {
            bail!("SKILL.md exceeds the 2 MiB text limit");
        }
        total = bytes.len() as u64;
        files.push(InventoryFile {
            relative: PathBuf::from("SKILL.md"),
            bytes,
            entry: true,
        });
    }
    if total > MAX_DUPLICATE_TOTAL_BYTES {
        bail!("Skill exceeds the 64 MiB duplicate limit");
    }
    if files.len() > MAX_DUPLICATE_FILES {
        bail!("Skill exceeds the 1,000-file duplicate limit");
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    let mut digest = Sha256::new();
    digest.update(b"koma-skill-tree-v1\0");
    for file in &files {
        digest.update(file.relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update((file.bytes.len() as u64).to_le_bytes());
        digest.update(&file.bytes);
    }
    Ok(SourceInventory {
        files,
        digest: format!("{:x}", digest.finalize()),
    })
}

fn inventory_dir(
    root: &Path,
    dir: &Path,
    depth: usize,
    total: &mut u64,
    files: &mut Vec<InventoryFile>,
) -> Result<()> {
    if depth > MAX_DUPLICATE_DEPTH {
        bail!("Skill exceeds the maximum duplicate depth of 16");
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("Skill contains a symbolic link: {}", path.display());
        }
        if metadata.is_dir() {
            inventory_dir(root, &path, depth + 1, total, files)?;
            continue;
        }
        if !metadata.is_file() {
            bail!("Skill contains a non-regular file: {}", path.display());
        }
        if files.len() >= MAX_DUPLICATE_FILES {
            bail!("Skill exceeds the 1,000-file duplicate limit");
        }
        let relative = path.strip_prefix(root)?.to_path_buf();
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            bail!("Invalid source path {}", relative.display());
        }
        let entry = relative
            .file_name()
            .map(|name| name.to_string_lossy().eq_ignore_ascii_case("SKILL.md"))
            .unwrap_or(false)
            && relative
                .parent()
                .is_none_or(|parent| parent.as_os_str().is_empty());
        let limit = if entry {
            MAX_SKILL_ENTRY_BYTES
        } else {
            MAX_DUPLICATE_COMPANION_BYTES
        };
        if metadata.len() > limit {
            bail!(
                "{} exceeds the {} MiB file limit",
                relative.display(),
                limit / 1024 / 1024
            );
        }
        *total = total.saturating_add(metadata.len());
        if *total > MAX_DUPLICATE_TOTAL_BYTES {
            bail!("Skill exceeds the 64 MiB duplicate limit");
        }
        files.push(InventoryFile {
            relative,
            bytes: std::fs::read(&path)?,
            entry,
        });
    }
    Ok(())
}

fn reject_unsafe_tree(path: &Path, depth: usize, files: &mut usize) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("Refusing to delete a skill containing a symbolic link");
    }
    if metadata.is_file() {
        *files += 1;
        if *files > MAX_DUPLICATE_FILES {
            bail!("Skill exceeds the safe delete file limit");
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("Refusing to delete a non-regular skill path");
    }
    if depth > MAX_DUPLICATE_DEPTH {
        bail!("Skill exceeds the safe delete depth limit");
    }
    for entry in std::fs::read_dir(path)? {
        reject_unsafe_tree(&entry?.path(), depth + 1, files)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_package(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (path, bytes) in files {
                writer.start_file(*path, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn edit(description: &str) -> SkillEdit {
        SkillEdit {
            description: description.to_string(),
            triggers: "when asked".to_string(),
            allowed_tools: vec!["Read".to_string()],
            instruction: "Instructions".to_string(),
        }
    }

    #[test]
    fn create_reserves_destination_and_publishes_entry_last() {
        let tmp = std::env::temp_dir().join(format!("koma-skill-create-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let entry =
            create_owned_skill(Some(&tmp), OwnedSkillTarget::Project, "demo", &edit("Demo"))
                .unwrap();
        assert!(entry.ends_with(".agents/skills/demo/SKILL.md"));
        assert!(entry.is_file());
        let mut invalid = edit("");
        invalid.description.clear();
        assert!(
            create_owned_skill(Some(&tmp), OwnedSkillTarget::Project, "invalid", &invalid).is_err()
        );
        assert!(!tmp.join(".agents/skills/invalid").exists());
        assert!(create_owned_skill(
            Some(&tmp),
            OwnedSkillTarget::Project,
            "demo",
            &edit("Other")
        )
        .unwrap_err()
        .to_string()
        .contains("already exists"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn update_preserves_unknown_mapping_and_rejects_stale_generation() {
        let tmp = std::env::temp_dir().join(format!("koma-skill-update-{}", uuid::Uuid::new_v4()));
        let root = tmp.join(".agents/skills/demo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            "---\ndescription: Old\nmetadata:\n  author: Ada\n---\nOld body",
        )
        .unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[]);
        let skill = registry.get("demo").unwrap();
        update_owned_skill(&registry, &skill.skill_id, &skill.generation, &edit("New")).unwrap();
        let written = std::fs::read_to_string(root.join("SKILL.md")).unwrap();
        assert!(written.contains("author: Ada"));
        assert!(written.contains("description: New"));
        assert!(update_owned_skill(
            &registry,
            &skill.skill_id,
            &skill.generation,
            &edit("Again")
        )
        .unwrap_err()
        .to_string()
        .contains("changed on disk"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn advanced_yaml_save_fails_without_changing_bytes() {
        let tmp = std::env::temp_dir().join(format!("koma-skill-yaml-{}", uuid::Uuid::new_v4()));
        let root = tmp.join(".agents/skills/demo");
        let entry = root.join("SKILL.md");
        std::fs::create_dir_all(&root).unwrap();
        let original = "---\ndescription: Advanced\ndefaults: &defaults\n  owner: Ada\nmetadata: *defaults\n---\nBody";
        std::fs::write(&entry, original).unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[]);
        let skill = registry.get("demo").unwrap();
        assert!(
            update_owned_skill(&registry, &skill.skill_id, &skill.generation, &edit("New"))
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), original);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn external_update_and_delete_are_rejected_without_changing_bytes() {
        let tmp =
            std::env::temp_dir().join(format!("koma-skill-external-{}", uuid::Uuid::new_v4()));
        let external = tmp.join("external/vendor");
        let entry = external.join("SKILL.md");
        std::fs::create_dir_all(&external).unwrap();
        let original = b"---\ndescription: Vendor\nmetadata: { owner: Ada }\n---\nVendor body\n";
        std::fs::write(&entry, original).unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[tmp.join("external")]);
        let skill = registry.get("vendor").unwrap();

        assert!(update_owned_skill(
            &registry,
            &skill.skill_id,
            &skill.generation,
            &edit("Changed")
        )
        .unwrap_err()
        .to_string()
        .contains("read-only"));
        assert!(
            delete_owned_skill(&registry, &skill.skill_id, &skill.generation)
                .unwrap_err()
                .to_string()
                .contains("cannot be deleted")
        );
        assert_eq!(std::fs::read(&entry).unwrap(), original);
        assert!(entry.exists());
        let export = tmp.join("vendor.zip");
        assert!(
            export_owned_skill_zip(&registry, &skill.skill_id, &skill.generation, &export).is_err()
        );
        assert!(!export.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn install_zip_strips_one_wrapper_and_preserves_companions() {
        let tmp = std::env::temp_dir().join(format!("koma-skill-zip-{}", uuid::Uuid::new_v4()));
        let entry = b"---\ndescription: Packaged\nmetadata: { owner: Ada }\n---\nBody\n";
        let package = zip_package(&[
            ("wrapper/SKILL.md", entry),
            ("wrapper/references/guide.md", b"guide"),
        ]);
        let installed =
            install_owned_skill_zip(Some(&tmp), OwnedSkillTarget::Project, "packaged", &package)
                .unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), entry);
        assert_eq!(
            std::fs::read(installed.parent().unwrap().join("references/guide.md")).unwrap(),
            b"guide"
        );
        assert!(install_owned_skill_zip(
            Some(&tmp),
            OwnedSkillTarget::Project,
            "packaged",
            &package
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn export_zip_preserves_owned_entry_and_companions() {
        let tmp = std::env::temp_dir().join(format!("koma-skill-export-{}", uuid::Uuid::new_v4()));
        let root = tmp.join(".agents/skills/demo");
        let entry = b"---\ndescription: Export\n---\nBody\n";
        std::fs::create_dir_all(root.join("references")).unwrap();
        std::fs::write(root.join("SKILL.md"), entry).unwrap();
        std::fs::write(root.join("references/guide.md"), b"guide").unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[]);
        let skill = registry.get("demo").unwrap();
        let output = tmp.join("demo.zip");
        export_owned_skill_zip(&registry, &skill.skill_id, &skill.generation, &output).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let mut actual_entry = Vec::new();
        archive
            .by_name("SKILL.md")
            .unwrap()
            .read_to_end(&mut actual_entry)
            .unwrap();
        assert_eq!(actual_entry, entry);
        let mut guide = String::new();
        archive
            .by_name("references/guide.md")
            .unwrap()
            .read_to_string(&mut guide)
            .unwrap();
        assert_eq!(guide, "guide");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn install_zip_rejects_unsafe_paths_and_cleans_reservation() {
        let tmp =
            std::env::temp_dir().join(format!("koma-skill-zip-unsafe-{}", uuid::Uuid::new_v4()));
        let package = zip_package(&[
            ("SKILL.md", b"---\ndescription: Unsafe\n---\nBody"),
            ("../escape", b"secret"),
        ]);
        assert!(
            install_owned_skill_zip(Some(&tmp), OwnedSkillTarget::Project, "unsafe", &package)
                .is_err()
        );
        assert!(!tmp.join("escape").exists());
        assert!(!tmp.join(".agents/skills/unsafe").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn duplicate_external_preserves_bytes_and_rejects_second_reservation() {
        let tmp =
            std::env::temp_dir().join(format!("koma-skill-duplicate-{}", uuid::Uuid::new_v4()));
        let external = tmp.join("external/vendor");
        std::fs::create_dir_all(external.join("refs")).unwrap();
        let source = "---\ndescription: Vendor\nmetadata: { owner: Ada }\n---\nVendor body\n";
        std::fs::write(external.join("SKILL.md"), source).unwrap();
        std::fs::write(external.join("refs/guide.md"), "guide").unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[tmp.join("external")]);
        let skill = registry.get("vendor").unwrap();
        let entry = duplicate_to_owned(
            &registry,
            &skill.skill_id,
            &skill.generation,
            Some(&tmp),
            OwnedSkillTarget::Project,
            "copy",
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), source);
        assert_eq!(
            std::fs::read_to_string(entry.parent().unwrap().join("refs/guide.md")).unwrap(),
            "guide"
        );
        assert!(duplicate_to_owned(
            &registry,
            &skill.skill_id,
            &skill.generation,
            Some(&tmp),
            OwnedSkillTarget::Project,
            "copy",
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn owned_mutations_reject_symlinked_roots_and_entries() {
        use std::os::unix::fs::symlink;
        let tmp =
            std::env::temp_dir().join(format!("koma-skill-owned-link-{}", uuid::Uuid::new_v4()));
        let outside = tmp.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(tmp.join(".agents")).unwrap();
        symlink(&outside, tmp.join(".agents/skills")).unwrap();
        assert!(
            create_owned_skill(Some(&tmp), OwnedSkillTarget::Project, "demo", &edit("Demo"))
                .is_err()
        );
        assert!(!outside.join("demo").exists());

        std::fs::remove_file(tmp.join(".agents/skills")).unwrap();
        let owned = tmp.join(".agents/skills/demo");
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::write(owned.join("SKILL.md"), "---\ndescription: Demo\n---\nBody").unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[]);
        let skill = registry.get("demo").unwrap();
        let escaped = tmp.join("escaped");
        std::fs::create_dir_all(&escaped).unwrap();
        let original = b"---\ndescription: Demo\n---\nBody";
        std::fs::write(escaped.join("SKILL.md"), original).unwrap();
        std::fs::remove_dir_all(&owned).unwrap();
        symlink(&escaped, &owned).unwrap();
        assert!(update_owned_skill(
            &registry,
            &skill.skill_id,
            &skill.generation,
            &edit("Changed")
        )
        .is_err());
        assert!(delete_owned_skill(&registry, &skill.skill_id, &skill.generation).is_err());
        assert_eq!(std::fs::read(escaped.join("SKILL.md")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn duplicate_rejects_symlink_and_cleans_only_reserved_destination() {
        use std::os::unix::fs::symlink;
        let tmp = std::env::temp_dir().join(format!("koma-skill-symlink-{}", uuid::Uuid::new_v4()));
        let external = tmp.join("external/vendor");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(
            external.join("SKILL.md"),
            "---\ndescription: Vendor\n---\nBody",
        )
        .unwrap();
        std::fs::write(tmp.join("secret"), "secret").unwrap();
        symlink(tmp.join("secret"), external.join("leak")).unwrap();
        let registry = SkillRegistry::load_isolated(Some(&tmp), &[tmp.join("external")]);
        let skill = registry.get("vendor").unwrap();
        assert!(duplicate_to_owned(
            &registry,
            &skill.skill_id,
            &skill.generation,
            Some(&tmp),
            OwnedSkillTarget::Project,
            "copy",
        )
        .is_err());
        assert!(!tmp.join(".agents/skills/copy").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.join("secret")).unwrap(),
            "secret"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

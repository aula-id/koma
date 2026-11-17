//! Chunked staging of a skill ZIP uploaded by the GUI. The webview sends the package in
//! base64 chunks; the GUI host appends them to a scratch file under the koma base dir
//! (shared by the host and the daemon, unlike the temp dir) and the install then reads the
//! finished file, so a package is bounded by [`MAX_SKILL_ZIP_BYTES`] and not by one IPC frame.

// Chunk staging is only driven by the GUI host; the pure helpers are still built (and tested) everywhere.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Result};
use base64::Engine;

use super::{install_owned_skill_zip_file, OwnedSkillTarget, MAX_SKILL_ZIP_BYTES};

/// The webgui sends 4 MiB raw (about 5.6 MB base64) per chunk.
pub(crate) const MAX_CHUNK_B64_BYTES: usize = 12 * 1024 * 1024;

pub(crate) enum Staged {
    More,
    Complete(PathBuf),
}

pub(crate) fn upload_dir() -> Result<PathBuf> {
    Ok(crate::model::store::base_dir()?.join("skill-upload"))
}

pub(crate) fn valid_upload_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn part_path(dir: &Path, id: &str) -> Option<PathBuf> {
    valid_upload_id(id).then(|| dir.join(format!("{id}.part")))
}

/// Append one chunk. `offset` must equal the current `.part` length (0 starts the file) and
/// `total` is the whole package size. The chunk that reaches `total` renames the file to
/// `<id>.zip`. Any error removes the `.part` and returns a user-facing message.
pub(crate) fn append_chunk(
    dir: &Path,
    id: &str,
    offset: u64,
    total: u64,
    data_b64: &str,
) -> Result<Staged, String> {
    let part = part_path(dir, id).ok_or_else(|| "Upload id is invalid".to_string())?;
    let result = append_chunk_inner(dir, id, &part, offset, total, data_b64);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn append_chunk_inner(
    dir: &Path,
    id: &str,
    part: &Path,
    offset: u64,
    total: u64,
    data_b64: &str,
) -> Result<Staged, String> {
    if total == 0 || total > MAX_SKILL_ZIP_BYTES {
        return Err("Skill package exceeds the 256 MiB upload limit.".to_string());
    }
    if data_b64.len() > MAX_CHUNK_B64_BYTES {
        return Err("Upload chunk is too large".to_string());
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|_| "Invalid upload data".to_string())?;
    let end = offset
        .checked_add(data.len() as u64)
        .filter(|end| *end <= total)
        .ok_or_else(|| "Upload size does not match".to_string())?;
    let stage_err = |e: std::io::Error| format!("Could not stage the upload: {e}");
    std::fs::create_dir_all(dir).map_err(stage_err)?;
    let mut options = std::fs::OpenOptions::new();
    if offset == 0 {
        options.create(true).truncate(true).write(true);
    } else {
        let len = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if len != offset {
            return Err("Upload chunk out of order".to_string());
        }
        options.append(true);
    }
    let mut file = options.open(part).map_err(stage_err)?;
    file.write_all(&data).map_err(stage_err)?;
    drop(file);
    if end < total {
        return Ok(Staged::More);
    }
    let zip = dir.join(format!("{id}.zip"));
    std::fs::rename(part, &zip).map_err(stage_err)?;
    Ok(Staged::Complete(zip))
}

/// True when `path` is a direct child of `dir` named `<valid id>.zip`: the only files the
/// daemon will read and delete on behalf of an install request.
pub(crate) fn is_staged_zip(dir: &Path, path: &Path) -> bool {
    path.parent() == Some(dir)
        && path.extension().is_some_and(|ext| ext == "zip")
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(valid_upload_id)
}

/// Install from a staged file, then always delete it.
pub(crate) fn install_staged(
    dir: &Path,
    workdir: Option<&Path>,
    target: OwnedSkillTarget,
    name: &str,
    path: &Path,
) -> Result<PathBuf> {
    if !is_staged_zip(dir, path) {
        bail!("not a staged upload");
    }
    let result = install_owned_skill_zip_file(workdir, target, name, path);
    let _ = std::fs::remove_file(path);
    result
}

pub(crate) fn install_staged_upload(
    workdir: Option<&Path>,
    target: OwnedSkillTarget,
    name: &str,
    path: &Path,
) -> Result<PathBuf> {
    install_staged(&upload_dir()?, workdir, target, name, path)
}

/// Delete `*.part` / `*.zip` files older than `max_age` (best effort).
pub(crate) fn sweep_stale(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let staged = path
            .extension()
            .is_some_and(|ext| ext == "part" || ext == "zip");
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        if staged && age.is_some_and(|age| age >= max_age) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

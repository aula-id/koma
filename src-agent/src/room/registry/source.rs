//! Where registry files come from: the copy embedded at build time and the
//! per-session overlay on disk. Both are read into a [`Tree`] of
//! `relative/path -> text` so the parser does not care which one it got.

use std::collections::BTreeMap;
use std::path::Path;

use include_dir::{include_dir, Dir, DirEntry};

/// `relative/path -> file text`, with `/` separators on every platform.
pub type Tree = BTreeMap<String, String>;

/// `src-misc/room`, embedded at compile time. This is the fallback that is
/// always valid, so a fresh binary works with no files on disk.
static ROOM_ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../src-misc/room");

const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_DEPTH: usize = 4;
const EXTENSIONS: &[&str] = &["md", "toml", "json"];

/// The files baked into the binary.
pub fn embedded_tree() -> Tree {
    fn walk(dir: &Dir<'_>, out: &mut Tree) {
        for entry in dir.entries() {
            match entry {
                DirEntry::Dir(sub) => walk(sub, out),
                DirEntry::File(file) => {
                    if let Some(text) = file.contents_utf8() {
                        let path = file.path().to_string_lossy().replace('\\', "/");
                        out.insert(path, text.to_string());
                    }
                }
            }
        }
    }
    let mut out = Tree::new();
    walk(&ROOM_ASSETS, &mut out);
    out
}

/// Read the overlay directory. Never fails: anything unusable is skipped and
/// recorded in `warnings`, so a bad file cannot take the room down.
///
/// Limits: symlinks are not followed, files must be `.md`/`.toml`/`.json` under
/// 256 KiB, at most 512 files and 4 directory levels, and the directory must be
/// owned by the user running koma or the owner of the home directory (unix).
pub fn read_overlay(dir: &Path, warnings: &mut Vec<String>) -> Tree {
    let mut out = Tree::new();
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return out;
    };
    if meta.file_type().is_symlink() {
        warnings.push(format!("overlay {} is a symlink; ignored", dir.display()));
        return out;
    }
    if !meta.is_dir() {
        return out;
    }
    if let Err(reason) = check_owner(dir) {
        warnings.push(format!("overlay ignored: {reason}"));
        return out;
    }
    let mut count = 0usize;
    walk_fs(dir, dir, 0, &mut out, &mut count, warnings);
    out
}

fn walk_fs(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Tree,
    count: &mut usize,
    warnings: &mut Vec<String>,
) {
    let Ok(read) = std::fs::read_dir(dir) else {
        warnings.push(format!("cannot read directory {}", dir.display()));
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            warnings.push(format!("{}: symlink ignored", path.display()));
        } else if kind.is_dir() {
            if depth + 1 > MAX_DEPTH {
                warnings.push(format!("{}: too deep; ignored", path.display()));
            } else {
                walk_fs(root, &path, depth + 1, out, count, warnings);
            }
        } else if kind.is_file() {
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.contains(&e));
            if !ext_ok {
                continue;
            }
            if *count >= MAX_FILES {
                warnings.push(format!("more than {MAX_FILES} overlay files; the rest are ignored"));
                return;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                warnings.push(format!("{}: larger than {MAX_FILE_BYTES} bytes; ignored", path.display()));
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    *count += 1;
                    out.insert(rel, text);
                }
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            }
        }
    }
}

/// The overlay is user-controlled input that changes agent behaviour, so it is
/// only read when the same user owns it and their home directory. If either
/// owner cannot be determined the check is skipped.
#[cfg(unix)]
fn check_owner(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    let (Ok(dir_meta), Ok(home_meta)) = (std::fs::metadata(dir), std::fs::metadata(&home)) else {
        return Ok(());
    };
    // The user the process runs as counts too: a container can run as root with another user's home.
    // SAFETY: geteuid has no preconditions and cannot fail.
    let running_as = unsafe { libc::geteuid() };
    if dir_meta.uid() == home_meta.uid() || dir_meta.uid() == running_as {
        Ok(())
    } else {
        Err(format!("{} is not owned by the current user", dir.display()))
    }
}

#[cfg(not(unix))]
fn check_owner(_dir: &Path) -> Result<(), String> {
    Ok(())
}

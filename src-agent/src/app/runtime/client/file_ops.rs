//! Host-side Coding panel workspace file operations.
//!
//! Tree / read / save / create / rename / delete all run entirely off the daemon
//! (direct filesystem access), so they answer identically whether a session is
//! attached or not. Every request ALWAYS produces a matching push envelope so the
//! webview never hangs on a spinner.
//!
//! Security: every relative path is joined onto a configured workspace root with
//! component-based `..` rejection, then containment-checked after partial
//! canonicalize (symlink-escape rejection). Tree listings hide `.git` / `.koma`
//! / `node_modules` / `target`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use super::push_proto::PushEnvelope;
use super::push_rows::PushFileTreeEntry;
use super::HostCtl;

/// Cap on a Coding-panel file read (~5 MiB). Past this we reply with
/// `tooLarge: true` rather than shipping multi-megabyte content into Monaco.
const FILE_READ_SIZE_CAP: u64 = 5 * 1024 * 1024;

/// Directory basenames excluded from Coding panel tree listings.
const EXCLUDED_DIRS: &[&str] = &[".git", ".koma", "node_modules", "target"];

/// Handle one File* HostCtl by computing the result and pushing it back.
/// Called from the host-relay's control-message loop.
///
/// `session` is the foreground session uuid (if any) — used only to refresh the
/// GIT panel after a successful mutation (save/create/rename/delete).
pub(super) fn handle_file_ctl(
    ctl: &HostCtl,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
    session: Option<&str>,
) {
    match ctl {
        HostCtl::FileTree {
            root,
            path,
            request_id,
        } => file_tree(root, path, request_id, push, workdirs),
        HostCtl::FileRead {
            root,
            path,
            request_id,
        } => file_read(root, path, request_id, push, workdirs),
        HostCtl::FileSave {
            root,
            path,
            content,
            expected_fingerprint,
            request_id,
        } => {
            if file_save(
                root,
                path,
                content,
                expected_fingerprint,
                request_id,
                push,
                workdirs,
            ) {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileCreate {
            root,
            path,
            kind,
            request_id,
        } => {
            if file_create(root, path, kind, request_id, push, workdirs) {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileRename {
            root,
            old_path,
            new_path,
            request_id,
        } => {
            if file_rename(root, old_path, new_path, request_id, push, workdirs) {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileDelete {
            root,
            path,
            request_id,
        } => {
            if file_delete(root, path, request_id, push, workdirs) {
                refresh_git_status(push, session);
            }
        }
        _ => {}
    }
}

fn emit(push: &dyn Fn(String), env: &PushEnvelope) {
    if let Ok(json) = serde_json::to_string(env) {
        push(json);
    }
}

fn file_tree(
    root: &str,
    path: &str,
    request_id: &str,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
) {
    let reply = |entries: Vec<PushFileTreeEntry>, error: Option<String>| {
        emit(
            push,
            &PushEnvelope::FileTree {
                root: root.to_string(),
                path: path.to_string(),
                request_id: request_id.to_string(),
                entries,
                error,
            },
        );
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(Vec::new(), Some(e));
            return;
        }
    };

    let rd = match std::fs::read_dir(&abs) {
        Ok(rd) => rd,
        Err(e) => {
            reply(Vec::new(), Some(format!("failed to list directory: {e}")));
            return;
        }
    };

    let mut entries: Vec<PushFileTreeEntry> = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && is_excluded_dir(&name) {
            continue;
        }
        let rel = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        entries.push(PushFileTreeEntry {
            name,
            path: rel,
            is_dir,
        });
    }
    sort_entries(&mut entries);
    reply(entries, None);
}

fn file_read(
    root: &str,
    path: &str,
    request_id: &str,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
) {
    let reply = |content: Option<String>,
                 fingerprint: String,
                 binary: bool,
                 too_large: bool,
                 error: Option<String>| {
        emit(
            push,
            &PushEnvelope::FileRead {
                root: root.to_string(),
                path: path.to_string(),
                request_id: request_id.to_string(),
                content,
                fingerprint,
                binary,
                too_large,
                error,
            },
        );
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(None, String::new(), false, false, Some(e));
            return;
        }
    };

    let meta = match std::fs::metadata(&abs) {
        Ok(m) => m,
        Err(e) => {
            reply(
                None,
                String::new(),
                false,
                false,
                Some(format!("failed to read file: {e}")),
            );
            return;
        }
    };
    if meta.is_dir() {
        reply(
            None,
            String::new(),
            false,
            false,
            Some("path is a directory".to_string()),
        );
        return;
    }
    if meta.len() > FILE_READ_SIZE_CAP {
        reply(None, String::new(), false, true, None);
        return;
    }

    let bytes = match std::fs::read(&abs) {
        Ok(b) => b,
        Err(e) => {
            reply(
                None,
                String::new(),
                false,
                false,
                Some(format!("failed to read file: {e}")),
            );
            return;
        }
    };
    if looks_binary(&bytes) {
        reply(None, String::new(), true, false, None);
        return;
    }
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let fingerprint = compute_fingerprint(&abs);
    reply(Some(content), fingerprint, false, false, None);
}

fn file_save(
    root: &str,
    path: &str,
    content: &str,
    expected_fingerprint: &str,
    request_id: &str,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
) -> bool {
    let reply = |fingerprint: String, error: Option<String>| {
        emit(
            push,
            &PushEnvelope::FileSave {
                root: root.to_string(),
                path: path.to_string(),
                request_id: request_id.to_string(),
                fingerprint,
                error,
            },
        );
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(String::new(), Some(e));
            return false;
        }
    };

    if abs.exists() {
        let current = compute_fingerprint(&abs);
        if current != expected_fingerprint {
            reply(
                current,
                Some("conflict: file changed on disk since last read".to_string()),
            );
            return false;
        }
    } else if !expected_fingerprint.is_empty() {
        reply(
            String::new(),
            Some("conflict: file changed on disk since last read".to_string()),
        );
        return false;
    }

    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            reply(
                String::new(),
                Some(format!("failed to create parent dirs: {e}")),
            );
            return false;
        }
    }
    if let Err(e) = std::fs::write(&abs, content.as_bytes()) {
        reply(String::new(), Some(format!("failed to write file: {e}")));
        return false;
    }
    reply(compute_fingerprint(&abs), None);
    true
}

fn file_create(
    root: &str,
    path: &str,
    kind: &str,
    request_id: &str,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
) -> bool {
    let reply = |error: Option<String>| {
        emit(
            push,
            &PushEnvelope::FileCreate {
                root: root.to_string(),
                path: path.to_string(),
                request_id: request_id.to_string(),
                error,
            },
        );
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(Some(e));
            return false;
        }
    };

    if abs.exists() {
        reply(Some("path already exists".to_string()));
        return false;
    }

    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            reply(Some(format!("failed to create parent dirs: {e}")));
            return false;
        }
    }

    let result = match kind {
        "dir" => std::fs::create_dir(&abs).map_err(|e| format!("failed to create directory: {e}")),
        "file" => std::fs::write(&abs, b"").map_err(|e| format!("failed to create file: {e}")),
        other => Err(format!("unknown kind '{other}' (expected 'file' or 'dir')")),
    };
    match result {
        Ok(()) => {
            reply(None);
            true
        }
        Err(e) => {
            reply(Some(e));
            false
        }
    }
}

fn file_rename(
    root: &str,
    old_path: &str,
    new_path: &str,
    request_id: &str,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
) -> bool {
    let reply = |error: Option<String>| {
        emit(
            push,
            &PushEnvelope::FileRename {
                root: root.to_string(),
                old_path: old_path.to_string(),
                new_path: new_path.to_string(),
                request_id: request_id.to_string(),
                error,
            },
        );
    };

    let old_abs = match resolve_contained(root, old_path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(Some(e));
            return false;
        }
    };
    let new_abs = match resolve_contained(root, new_path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(Some(e));
            return false;
        }
    };

    if !old_abs.exists() {
        reply(Some("source path does not exist".to_string()));
        return false;
    }
    if new_abs.exists() {
        reply(Some("destination already exists".to_string()));
        return false;
    }
    if let Some(parent) = new_abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            reply(Some(format!("failed to create parent dirs: {e}")));
            return false;
        }
    }
    if let Err(e) = std::fs::rename(&old_abs, &new_abs) {
        reply(Some(format!("failed to rename: {e}")));
        return false;
    }
    reply(None);
    true
}

fn file_delete(
    root: &str,
    path: &str,
    request_id: &str,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
) -> bool {
    let reply = |error: Option<String>| {
        emit(
            push,
            &PushEnvelope::FileDelete {
                root: root.to_string(),
                path: path.to_string(),
                request_id: request_id.to_string(),
                error,
            },
        );
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => {
            reply(Some(e));
            return false;
        }
    };

    if path.is_empty() || path == "." {
        reply(Some("refusing to delete workspace root".to_string()));
        return false;
    }

    if !abs.exists() {
        reply(Some("path does not exist".to_string()));
        return false;
    }

    let result = if abs.is_dir() {
        std::fs::remove_dir_all(&abs).map_err(|e| format!("failed to delete directory: {e}"))
    } else {
        std::fs::remove_file(&abs).map_err(|e| format!("failed to delete file: {e}"))
    };
    match result {
        Ok(()) => {
            reply(None);
            true
        }
        Err(e) => {
            reply(Some(e));
            false
        }
    }
}

/// Best-effort GIT panel refresh after a Coding-panel mutation.
fn refresh_git_status(push: &dyn Fn(String), session: Option<&str>) {
    let result = super::git::compute_git_status(session);
    super::push_proto_git::push_git_status(push, result);
}

/// Resolve `root` + relative `path` to an absolute path that is contained inside
/// one of the configured `workdirs`. Rejects absolute relative portions, `..`
/// traversal, roots that aren't configured, and symlink escapes.
fn resolve_contained(root: &str, path: &str, workdirs: &[PathBuf]) -> Result<PathBuf, String> {
    let root_path = Path::new(root);
    if !root_path.is_absolute() {
        return Err("workspace root must be an absolute path".to_string());
    }

    let root_canon = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());

    // Root must match one of the configured workdirs (canonical compare).
    // When workdirs is empty (detached / no-session), still allow the absolute
    // root itself, with path-containment enforced below.
    if !workdirs.is_empty() {
        let root_s = root_canon.to_string_lossy();
        let ok = workdirs.iter().any(|wd| {
            let wd_canon = std::fs::canonicalize(wd).unwrap_or_else(|_| wd.clone());
            wd_canon == root_canon
                || wd.as_path() == root_path
                || wd_canon == root_path
                || wd_canon.to_string_lossy() == root_s
                || wd.to_string_lossy() == root
        });
        if !ok {
            return Err("workspace root is not a configured workdir".to_string());
        }
    }

    let joined =
        safe_join(&root_canon, path).ok_or_else(|| "path escapes workspace root".to_string())?;

    // Partial-canonicalize: walk up to the longest existing prefix, re-append the
    // non-existent tail, then check containment. This rejects symlink escapes
    // through existing ancestors while still allowing create paths that don't
    // exist yet.
    let candidate = partial_canonicalize(&joined);
    if !candidate.starts_with(&root_canon) {
        return Err("path escapes workspace root".to_string());
    }
    Ok(candidate)
}

/// Anchor an untrusted relative path onto `root`, rejecting absolute paths and
/// any `..` / root / prefix component. Mirrors `git::safe_join`.
fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let relp = Path::new(rel);
    if relp.is_absolute() {
        return None;
    }
    for c in relp.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            // Empty relative path ("") yields zero components — allowed (the root).
            _ => return None, // ParentDir, RootDir, Prefix
        }
    }
    Some(root.join(relp))
}

/// Canonicalize as far as the path exists, re-appending the non-existent tail.
/// Resolves symlink ancestors so a link pointing outside the root fails the
/// subsequent `starts_with` containment check.
fn partial_canonicalize(path: &Path) -> PathBuf {
    match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let mut existing = path;
            let mut tail: Vec<std::ffi::OsString> = Vec::new();
            while !existing.exists() {
                match existing.file_name() {
                    Some(n) => tail.push(n.to_os_string()),
                    None => break,
                }
                match existing.parent() {
                    Some(p) => existing = p,
                    None => break,
                }
            }
            let mut base = existing
                .canonicalize()
                .unwrap_or_else(|_| existing.to_path_buf());
            for seg in tail.iter().rev() {
                base.push(seg);
            }
            base
        }
    }
}

/// Fingerprint for stale-save detection: mtime + size + first 4KB content hash.
fn compute_fingerprint(path: &Path) -> String {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let head = std::fs::read(path)
        .ok()
        .map(|b| {
            let slice = &b[..std::cmp::min(b.len(), 4096)];
            let mut h = DefaultHasher::new();
            slice.hash(&mut h);
            h.finish()
        })
        .unwrap_or(0);
    let mut h = DefaultHasher::new();
    mtime.hash(&mut h);
    size.hash(&mut h);
    head.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// NUL byte in the first 8KiB ⇒ binary (matches the harness/diff sniff).
fn looks_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0)
}

fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name)
}

/// Directories first, then alphabetical by name (case-insensitive).
fn sort_entries(entries: &mut [PushFileTreeEntry]) {
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    fn temp_workspace(tag: &str) -> (PathBuf, String, Vec<PathBuf>) {
        let dir = std::env::temp_dir().join(format!(
            "koma-fileops-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.canonicalize().unwrap();
        let root_s = root.to_string_lossy().into_owned();
        let workdirs = vec![root.clone()];
        (dir, root_s, workdirs)
    }

    fn capture_push() -> (Arc<Mutex<Vec<String>>>, Box<dyn Fn(String)>) {
        let sink = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink2 = Arc::clone(&sink);
        let push = Box::new(move |json: String| {
            sink2.lock().unwrap().push(json);
        });
        (sink, push)
    }

    fn last_json(sink: &Arc<Mutex<Vec<String>>>) -> serde_json::Value {
        let guard = sink.lock().unwrap();
        let last = guard.last().expect("expected at least one push");
        serde_json::from_str(last).expect("push must be valid json")
    }

    fn json_for_request(sink: &Arc<Mutex<Vec<String>>>, request_id: &str) -> serde_json::Value {
        let guard = sink.lock().unwrap();
        guard
            .iter()
            .rev()
            .map(|json| {
                serde_json::from_str::<serde_json::Value>(json).expect("push must be valid json")
            })
            .find(|env| env["requestId"] == request_id)
            .expect("expected push for request id")
    }

    #[test]
    fn fingerprint_is_stable_for_unchanged_file() {
        let dir = std::env::temp_dir().join(format!("koma-fileops-fp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            write!(f, "hello fingerprint").unwrap();
        }
        let a = compute_fingerprint(&path);
        let b = compute_fingerprint(&path);
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_changes_when_content_changes() {
        let dir = std::env::temp_dir().join(format!("koma-fileops-fp2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.txt");
        std::fs::write(&path, b"one").unwrap();
        let a = compute_fingerprint(&path);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"two").unwrap();
        let b = compute_fingerprint(&path);
        assert_ne!(a, b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_join_rejects_traversal_and_absolute() {
        let root = PathBuf::from("/tmp/ws");
        assert!(safe_join(&root, "src/main.rs").is_some());
        assert!(safe_join(&root, "").is_some());
        assert!(safe_join(&root, "a/./b").is_some());
        assert!(safe_join(&root, "../escape").is_none());
        assert!(safe_join(&root, "a/../../escape").is_none());
        assert!(safe_join(&root, "/etc/passwd").is_none());
    }

    #[test]
    fn resolve_contained_rejects_escape() {
        let dir = std::env::temp_dir().join(format!("koma-fileops-esc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.canonicalize().unwrap();
        let workdirs = vec![root.clone()];
        let root_s = root.to_string_lossy().into_owned();

        assert!(resolve_contained(&root_s, "ok.txt", &workdirs).is_ok());
        assert!(resolve_contained(&root_s, "../escape", &workdirs).is_err());
        assert!(resolve_contained(&root_s, "/etc/passwd", &workdirs).is_err());
        assert!(resolve_contained("/not/a/configured/root", "x", &workdirs).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tree_sort_dirs_first_then_alpha() {
        let mut entries = vec![
            PushFileTreeEntry {
                name: "z.txt".into(),
                path: "z.txt".into(),
                is_dir: false,
            },
            PushFileTreeEntry {
                name: "B".into(),
                path: "B".into(),
                is_dir: true,
            },
            PushFileTreeEntry {
                name: "a.txt".into(),
                path: "a.txt".into(),
                is_dir: false,
            },
            PushFileTreeEntry {
                name: "A".into(),
                path: "A".into(),
                is_dir: true,
            },
        ];
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["A", "B", "a.txt", "z.txt"]);
    }

    #[test]
    fn excluded_dirs_match() {
        assert!(is_excluded_dir(".git"));
        assert!(is_excluded_dir("node_modules"));
        assert!(is_excluded_dir("target"));
        assert!(is_excluded_dir(".koma"));
        assert!(!is_excluded_dir("src"));
    }

    #[test]
    fn file_create_tree_rename_delete_roundtrip_in_temp_workspace() {
        let (dir, root_s, workdirs) = temp_workspace("crud");
        let (sink, push) = capture_push();

        // Create a nested file + a directory.
        assert!(file_create(
            &root_s,
            "src/hello.txt",
            "file",
            "req-create-file",
            push.as_ref(),
            &workdirs,
        ));
        let created = last_json(&sink);
        assert_eq!(created["k"], "FileCreate");
        assert_eq!(created["requestId"], "req-create-file");
        assert_eq!(created["root"], root_s);
        assert_eq!(created["path"], "src/hello.txt");
        assert!(created["error"].is_null());
        assert!(dir.join("src/hello.txt").is_file());

        assert!(file_create(
            &root_s,
            "src/nested",
            "dir",
            "req-create-dir",
            push.as_ref(),
            &workdirs,
        ));
        let created_dir = last_json(&sink);
        assert_eq!(created_dir["k"], "FileCreate");
        assert_eq!(created_dir["requestId"], "req-create-dir");
        assert!(created_dir["error"].is_null());
        assert!(dir.join("src/nested").is_dir());

        // Tree listing of src/ — dirs first, excludes nothing here.
        file_tree(&root_s, "src", "req-tree-src", push.as_ref(), &workdirs);
        let tree = last_json(&sink);
        assert_eq!(tree["k"], "FileTree");
        assert_eq!(tree["requestId"], "req-tree-src");
        assert_eq!(tree["path"], "src");
        assert!(tree["error"].is_null());
        let entries = tree["entries"].as_array().expect("entries array");
        let names: Vec<&str> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap_or(""))
            .collect();
        assert!(names.contains(&"hello.txt"));
        assert!(names.contains(&"nested"));
        // Excluded dirs must not appear even if present.
        std::fs::create_dir_all(dir.join("src/target")).unwrap();
        std::fs::create_dir_all(dir.join("src/.git")).unwrap();
        file_tree(&root_s, "src", "req-tree-excl", push.as_ref(), &workdirs);
        let tree2 = last_json(&sink);
        let names2: Vec<&str> = tree2["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap_or(""))
            .collect();
        assert!(!names2.contains(&"target"));
        assert!(!names2.contains(&".git"));

        // Rename file, echo request id on success.
        assert!(file_rename(
            &root_s,
            "src/hello.txt",
            "src/hi.txt",
            "req-rename",
            push.as_ref(),
            &workdirs,
        ));
        let renamed = last_json(&sink);
        assert_eq!(renamed["k"], "FileRename");
        assert_eq!(renamed["requestId"], "req-rename");
        assert_eq!(renamed["oldPath"], "src/hello.txt");
        assert_eq!(renamed["newPath"], "src/hi.txt");
        assert!(renamed["error"].is_null());
        assert!(!dir.join("src/hello.txt").exists());
        assert!(dir.join("src/hi.txt").is_file());

        // Delete renamed file.
        assert!(file_delete(
            &root_s,
            "src/hi.txt",
            "req-delete",
            push.as_ref(),
            &workdirs,
        ));
        let deleted = last_json(&sink);
        assert_eq!(deleted["k"], "FileDelete");
        assert_eq!(deleted["requestId"], "req-delete");
        assert_eq!(deleted["path"], "src/hi.txt");
        assert!(deleted["error"].is_null());
        assert!(!dir.join("src/hi.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_ops_error_replies_echo_request_ids() {
        let (dir, root_s, workdirs) = temp_workspace("errs");
        let (sink, push) = capture_push();

        // Create collision.
        std::fs::write(dir.join("exists.txt"), b"x").unwrap();
        assert!(!file_create(
            &root_s,
            "exists.txt",
            "file",
            "req-exists",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["k"], "FileCreate");
        assert_eq!(v["requestId"], "req-exists");
        assert_eq!(v["error"], "path already exists");

        // Unknown kind.
        assert!(!file_create(
            &root_s,
            "weird",
            "symlink",
            "req-kind",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["requestId"], "req-kind");
        assert!(v["error"].as_str().unwrap_or("").contains("unknown kind"));

        // Rename missing source.
        assert!(!file_rename(
            &root_s,
            "missing.txt",
            "other.txt",
            "req-ren-miss",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["k"], "FileRename");
        assert_eq!(v["requestId"], "req-ren-miss");
        assert_eq!(v["error"], "source path does not exist");

        // Rename destination collision.
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::fs::write(dir.join("b.txt"), b"b").unwrap();
        assert!(!file_rename(
            &root_s,
            "a.txt",
            "b.txt",
            "req-ren-coll",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["requestId"], "req-ren-coll");
        assert_eq!(v["error"], "destination already exists");

        // Delete missing + refuse workspace root.
        assert!(!file_delete(
            &root_s,
            "nope.txt",
            "req-del-miss",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["requestId"], "req-del-miss");
        assert_eq!(v["error"], "path does not exist");

        assert!(!file_delete(
            &root_s,
            "",
            "req-del-root",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["requestId"], "req-del-root");
        assert_eq!(v["error"], "refusing to delete workspace root");

        // Path escape is rejected with a push error (not a panic).
        file_tree(
            &root_s,
            "../escape",
            "req-tree-esc",
            push.as_ref(),
            &workdirs,
        );
        let v = last_json(&sink);
        assert_eq!(v["k"], "FileTree");
        assert_eq!(v["requestId"], "req-tree-esc");
        assert!(!v["error"].is_null());

        // Unconfigured root.
        let unconfigured = std::env::temp_dir()
            .join("koma-unconfigured-root")
            .to_string_lossy()
            .into_owned();
        assert!(!file_create(
            &unconfigured,
            "x.txt",
            "file",
            "req-bad-root",
            push.as_ref(),
            &workdirs,
        ));
        let v = last_json(&sink);
        assert_eq!(v["requestId"], "req-bad-root");
        assert_eq!(v["error"], "workspace root is not a configured workdir");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handle_file_ctl_routes_create_tree_rename_delete() {
        let (dir, root_s, workdirs) = temp_workspace("ctl");
        let (sink, push) = capture_push();

        handle_file_ctl(
            &HostCtl::FileCreate {
                root: root_s.clone(),
                path: "note.md".into(),
                kind: "file".into(),
                request_id: "ctl-c".into(),
            },
            push.as_ref(),
            &workdirs,
            None,
        );
        assert_eq!(json_for_request(&sink, "ctl-c")["requestId"], "ctl-c");
        assert!(dir.join("note.md").is_file());

        handle_file_ctl(
            &HostCtl::FileTree {
                root: root_s.clone(),
                path: "".into(),
                request_id: "ctl-t".into(),
            },
            push.as_ref(),
            &workdirs,
            None,
        );
        let tree = json_for_request(&sink, "ctl-t");
        assert_eq!(tree["requestId"], "ctl-t");
        assert!(tree["error"].is_null());

        handle_file_ctl(
            &HostCtl::FileRename {
                root: root_s.clone(),
                old_path: "note.md".into(),
                new_path: "readme.md".into(),
                request_id: "ctl-r".into(),
            },
            push.as_ref(),
            &workdirs,
            None,
        );
        assert_eq!(json_for_request(&sink, "ctl-r")["requestId"], "ctl-r");
        assert!(dir.join("readme.md").is_file());

        handle_file_ctl(
            &HostCtl::FileDelete {
                root: root_s.clone(),
                path: "readme.md".into(),
                request_id: "ctl-d".into(),
            },
            push.as_ref(),
            &workdirs,
            None,
        );
        assert_eq!(json_for_request(&sink, "ctl-d")["requestId"], "ctl-d");
        assert!(!dir.join("readme.md").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

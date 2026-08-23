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
//!
//! The `exec_*` functions are the pure compute surface reused by both the local
//! host path (`handle_file_ctl`) and the remote thin client (`koma remote-fs`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use super::push_proto::PushEnvelope;
use super::push_rows::PushFileTreeEntry;
use super::HostCtl;

/// Cap on a Coding-panel file read (~5 MiB). Past this we reply with
/// `tooLarge: true` rather than shipping multi-megabyte content into Monaco.
const FILE_READ_SIZE_CAP: u64 = 5 * 1024 * 1024;

/// Cap on binary upload/download through the GUI bridge (~25 MiB decoded).
/// Larger than the Monaco text-read cap: drag-upload / save-as carry raw bytes
/// as base64, not editor buffers.
const FILE_BYTES_SIZE_CAP: u64 = 25 * 1024 * 1024;

/// Directory basenames excluded from Coding panel tree listings.
const EXCLUDED_DIRS: &[&str] = &[".git", ".koma", "node_modules", "target"];

// ─── Extractable result types (remote-fs + local host share these) ───────────

/// Directory listing result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileTreeResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub entries: Vec<PushFileTreeEntry>,
    pub error: Option<String>,
}

/// File read result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileReadResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub content: Option<String>,
    pub fingerprint: String,
    pub binary: bool,
    pub too_large: bool,
    pub error: Option<String>,
}

/// File save result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileSaveResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub fingerprint: String,
    pub error: Option<String>,
    /// `true` when the write landed — caller may refresh git status.
    #[serde(skip)]
    pub mutated: bool,
}

/// File create result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileCreateResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub error: Option<String>,
    #[serde(skip)]
    pub mutated: bool,
}

/// File rename result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileRenameResult {
    pub root: String,
    pub old_path: String,
    pub new_path: String,
    pub request_id: String,
    pub error: Option<String>,
    #[serde(skip)]
    pub mutated: bool,
}

/// File delete result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDeleteResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub error: Option<String>,
    #[serde(skip)]
    pub mutated: bool,
}

/// Binary write (drag-upload) result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileWriteBytesResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub error: Option<String>,
    #[serde(skip)]
    pub mutated: bool,
}

/// Binary download result for Coding panel / remote-fs.
/// `bytes_b64` is standard base64 of the file body when successful.
/// When `saved` is true the host already wrote the bytes via a native save
/// dialog (save-as path) and `bytes_b64` is cleared — the webview must not
/// attempt an in-page blob download (wry ignores `<a download>`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDownloadBytesResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub bytes_b64: Option<String>,
    pub size: u64,
    pub too_large: bool,
    pub error: Option<String>,
    /// True only after a successful native save-as write on the host.
    #[serde(default)]
    pub saved: bool,
}

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
        } => {
            let r = exec_file_tree(root, path, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileTree {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    entries: r.entries,
                    error: r.error,
                },
            );
        }
        HostCtl::FileRead {
            root,
            path,
            request_id,
        } => {
            let r = exec_file_read(root, path, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileRead {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    content: r.content,
                    fingerprint: r.fingerprint,
                    binary: r.binary,
                    too_large: r.too_large,
                    error: r.error,
                },
            );
        }
        HostCtl::FileSave {
            root,
            path,
            content,
            expected_fingerprint,
            request_id,
        } => {
            let r = exec_file_save(root, path, content, expected_fingerprint, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileSave {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    fingerprint: r.fingerprint,
                    error: r.error,
                },
            );
            if r.mutated {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileCreate {
            root,
            path,
            kind,
            request_id,
        } => {
            let r = exec_file_create(root, path, kind, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileCreate {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    error: r.error,
                },
            );
            if r.mutated {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileRename {
            root,
            old_path,
            new_path,
            request_id,
        } => {
            let r = exec_file_rename(root, old_path, new_path, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileRename {
                    root: r.root,
                    old_path: r.old_path,
                    new_path: r.new_path,
                    request_id: r.request_id,
                    error: r.error,
                },
            );
            if r.mutated {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileDelete {
            root,
            path,
            request_id,
        } => {
            let r = exec_file_delete(root, path, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileDelete {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    error: r.error,
                },
            );
            if r.mutated {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileWriteBytes {
            root,
            path,
            bytes_b64,
            overwrite,
            request_id,
        } => {
            let r = exec_file_write_bytes(root, path, bytes_b64, *overwrite, request_id, workdirs);
            emit(
                push,
                &PushEnvelope::FileWriteBytes {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    error: r.error,
                },
            );
            if r.mutated {
                refresh_git_status(push, session);
            }
        }
        HostCtl::FileDownloadBytes {
            root,
            path,
            request_id,
            save_as,
        } => {
            let r = exec_file_download_bytes(root, path, request_id, workdirs);
            let r = finalize_download_bytes(r, *save_as);
            emit(
                push,
                &PushEnvelope::FileDownloadBytes {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    bytes_b64: r.bytes_b64,
                    size: r.size,
                    too_large: r.too_large,
                    error: r.error,
                    saved: r.saved,
                },
            );
        }
        _ => {}
    }
}

fn emit(push: &dyn Fn(String), env: &PushEnvelope) {
    if let Ok(json) = serde_json::to_string(env) {
        push(json);
    }
}

// ─── Pure exec surface ───────────────────────────────────────────────────────

/// List immediate children of `root`/`path` under the workdir sandbox.
pub(crate) fn exec_file_tree(
    root: &str,
    path: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileTreeResult {
    let fail = |error: String| FileTreeResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        entries: Vec::new(),
        error: Some(error),
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    let rd = match std::fs::read_dir(&abs) {
        Ok(rd) => rd,
        Err(e) => return fail(format!("failed to list directory: {e}")),
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
    FileTreeResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        entries,
        error: None,
    }
}

/// Read a text file under the workdir sandbox.
pub(crate) fn exec_file_read(
    root: &str,
    path: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileReadResult {
    let fail = |error: Option<String>, binary: bool, too_large: bool| FileReadResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        content: None,
        fingerprint: String::new(),
        binary,
        too_large,
        error,
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(Some(e), false, false),
    };

    let meta = match std::fs::metadata(&abs) {
        Ok(m) => m,
        Err(e) => return fail(Some(format!("failed to read file: {e}")), false, false),
    };
    if meta.is_dir() {
        return fail(Some("path is a directory".to_string()), false, false);
    }
    if meta.len() > FILE_READ_SIZE_CAP {
        return fail(None, false, true);
    }

    let bytes = match std::fs::read(&abs) {
        Ok(b) => b,
        Err(e) => return fail(Some(format!("failed to read file: {e}")), false, false),
    };
    if looks_binary(&bytes) {
        return fail(None, true, false);
    }
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let fingerprint = compute_fingerprint(&abs);
    FileReadResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        content: Some(content),
        fingerprint,
        binary: false,
        too_large: false,
        error: None,
    }
}

/// Save a text file with stale-fingerprint protection.
pub(crate) fn exec_file_save(
    root: &str,
    path: &str,
    content: &str,
    expected_fingerprint: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileSaveResult {
    let fail = |fingerprint: String, error: String| FileSaveResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        fingerprint,
        error: Some(error),
        mutated: false,
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(String::new(), e),
    };

    if abs.exists() {
        let current = compute_fingerprint(&abs);
        if current != expected_fingerprint {
            return fail(
                current,
                "conflict: file changed on disk since last read".to_string(),
            );
        }
    } else if !expected_fingerprint.is_empty() {
        return fail(
            String::new(),
            "conflict: file changed on disk since last read".to_string(),
        );
    }

    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return fail(String::new(), format!("failed to create parent dirs: {e}"));
        }
    }
    if let Err(e) = std::fs::write(&abs, content.as_bytes()) {
        return fail(String::new(), format!("failed to write file: {e}"));
    }
    FileSaveResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        fingerprint: compute_fingerprint(&abs),
        error: None,
        mutated: true,
    }
}

/// Create a new file or directory under the workdir sandbox.
pub(crate) fn exec_file_create(
    root: &str,
    path: &str,
    kind: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileCreateResult {
    let fail = |error: String| FileCreateResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        error: Some(error),
        mutated: false,
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    if abs.exists() {
        return fail("path already exists".to_string());
    }

    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return fail(format!("failed to create parent dirs: {e}"));
        }
    }

    let result = match kind {
        "dir" => std::fs::create_dir(&abs).map_err(|e| format!("failed to create directory: {e}")),
        "file" => std::fs::write(&abs, b"").map_err(|e| format!("failed to create file: {e}")),
        other => Err(format!("unknown kind '{other}' (expected 'file' or 'dir')")),
    };
    match result {
        Ok(()) => FileCreateResult {
            root: root.to_string(),
            path: path.to_string(),
            request_id: request_id.to_string(),
            error: None,
            mutated: true,
        },
        Err(e) => fail(e),
    }
}

/// Rename within the same workspace root.
pub(crate) fn exec_file_rename(
    root: &str,
    old_path: &str,
    new_path: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileRenameResult {
    let fail = |error: String| FileRenameResult {
        root: root.to_string(),
        old_path: old_path.to_string(),
        new_path: new_path.to_string(),
        request_id: request_id.to_string(),
        error: Some(error),
        mutated: false,
    };

    let old_abs = match resolve_contained(root, old_path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let new_abs = match resolve_contained(root, new_path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    if !old_abs.exists() {
        return fail("source path does not exist".to_string());
    }
    if new_abs.exists() {
        return fail("destination already exists".to_string());
    }
    if let Some(parent) = new_abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return fail(format!("failed to create parent dirs: {e}"));
        }
    }
    if let Err(e) = std::fs::rename(&old_abs, &new_abs) {
        return fail(format!("failed to rename: {e}"));
    }
    FileRenameResult {
        root: root.to_string(),
        old_path: old_path.to_string(),
        new_path: new_path.to_string(),
        request_id: request_id.to_string(),
        error: None,
        mutated: true,
    }
}

/// Delete a file or directory under the workdir sandbox.
pub(crate) fn exec_file_delete(
    root: &str,
    path: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileDeleteResult {
    let fail = |error: String| FileDeleteResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        error: Some(error),
        mutated: false,
    };

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    if path.is_empty() || path == "." {
        return fail("refusing to delete workspace root".to_string());
    }

    if !abs.exists() {
        return fail("path does not exist".to_string());
    }

    let result = if abs.is_dir() {
        std::fs::remove_dir_all(&abs).map_err(|e| format!("failed to delete directory: {e}"))
    } else {
        std::fs::remove_file(&abs).map_err(|e| format!("failed to delete file: {e}"))
    };
    match result {
        Ok(()) => FileDeleteResult {
            root: root.to_string(),
            path: path.to_string(),
            request_id: request_id.to_string(),
            error: None,
            mutated: true,
        },
        Err(e) => fail(e),
    }
}

/// Write raw bytes (base64-encoded on the wire) under the workdir sandbox.
/// Used for drag-upload from the Coding tree (local copy or remote upload).
pub(crate) fn exec_file_write_bytes(
    root: &str,
    path: &str,
    bytes_b64: &str,
    overwrite: bool,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileWriteBytesResult {
    let fail = |error: String| FileWriteBytesResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        error: Some(error),
        mutated: false,
    };

    if path.is_empty() || path == "." {
        return fail("refusing to write workspace root".to_string());
    }

    let bytes = match decode_b64(bytes_b64) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    if bytes.len() as u64 > FILE_BYTES_SIZE_CAP {
        return fail(format!(
            "file too large (max {} MiB)",
            FILE_BYTES_SIZE_CAP / (1024 * 1024)
        ));
    }

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    if abs.exists() {
        if abs.is_dir() {
            return fail("destination is a directory".to_string());
        }
        if !overwrite {
            return fail("path already exists".to_string());
        }
    }

    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return fail(format!("failed to create parent dirs: {e}"));
        }
    }

    if let Err(e) = std::fs::write(&abs, &bytes) {
        return fail(format!("failed to write file: {e}"));
    }

    FileWriteBytesResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        error: None,
        mutated: true,
    }
}

/// Read raw bytes for download / save-as. Returns standard base64.
pub(crate) fn exec_file_download_bytes(
    root: &str,
    path: &str,
    request_id: &str,
    workdirs: &[PathBuf],
) -> FileDownloadBytesResult {
    let fail = |error: String, too_large: bool| FileDownloadBytesResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        bytes_b64: None,
        size: 0,
        too_large,
        error: Some(error),
        saved: false,
    };

    if path.is_empty() || path == "." {
        return fail("refusing to download workspace root".to_string(), false);
    }

    let abs = match resolve_contained(root, path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e, false),
    };

    if !abs.exists() {
        return fail("path does not exist".to_string(), false);
    }
    if abs.is_dir() {
        return fail("cannot download a directory".to_string(), false);
    }

    let meta = match std::fs::metadata(&abs) {
        Ok(m) => m,
        Err(e) => return fail(format!("failed to stat file: {e}"), false),
    };
    let size = meta.len();
    if size > FILE_BYTES_SIZE_CAP {
        return FileDownloadBytesResult {
            root: root.to_string(),
            path: path.to_string(),
            request_id: request_id.to_string(),
            bytes_b64: None,
            size,
            too_large: true,
            error: Some(format!(
                "file too large to download (max {} MiB)",
                FILE_BYTES_SIZE_CAP / (1024 * 1024)
            )),
            saved: false,
        };
    }

    let bytes = match std::fs::read(&abs) {
        Ok(b) => b,
        Err(e) => return fail(format!("failed to read file: {e}"), false),
    };

    use base64::Engine as _;
    let bytes_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    FileDownloadBytesResult {
        root: root.to_string(),
        path: path.to_string(),
        request_id: request_id.to_string(),
        bytes_b64: Some(bytes_b64),
        size,
        too_large: false,
        error: None,
        saved: false,
    }
}

/// When `save_as` is set, open a native save dialog and write the decoded
/// bytes on the host. Clears `bytes_b64` so the webview never tries a blob
/// download (dead in wry). Preview loads (`save_as=false`) pass through.
/// Safe to call on the local worker thread or after a remote-fs fetch.
pub(crate) fn finalize_download_bytes(
    mut r: FileDownloadBytesResult,
    save_as: bool,
) -> FileDownloadBytesResult {
    if !save_as {
        return r;
    }
    // Propagate read failures / size caps without opening a dialog.
    if r.error.is_some() || r.too_large {
        r.saved = false;
        r.bytes_b64 = None;
        return r;
    }
    let Some(b64) = r.bytes_b64.take() else {
        r.error = Some("empty download".to_string());
        r.saved = false;
        return r;
    };
    let bytes = match decode_b64(&b64) {
        Ok(b) => b,
        Err(e) => {
            r.error = Some(e);
            r.saved = false;
            return r;
        }
    };
    let name = Path::new(&r.path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("download")
        .to_string();
    match save_bytes_with_dialog(&name, &bytes) {
        Ok(true) => {
            r.saved = true;
            r.error = None;
        }
        Ok(false) => {
            // User cancelled the dialog — silent no-op for the GUI.
            r.saved = false;
            r.error = None;
        }
        Err(e) => {
            r.saved = false;
            r.error = Some(e);
        }
    }
    r
}

/// Native save-file dialog + write. `Ok(true)` written, `Ok(false)` cancelled.
fn save_bytes_with_dialog(suggested_name: &str, bytes: &[u8]) -> Result<bool, String> {
    let chosen = rfd::FileDialog::new()
        .set_file_name(suggested_name)
        .save_file();
    match chosen {
        None => Ok(false),
        Some(path) => std::fs::write(&path, bytes)
            .map(|_| true)
            .map_err(|e| format!("failed to write {}: {e}", path.display())),
    }
}

fn decode_b64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(s.trim()))
        .map_err(|e| format!("invalid base64: {e}"))
}

/// Best-effort GIT panel refresh after a Coding-panel mutation.
fn refresh_git_status(push: &dyn Fn(String), session: Option<&str>) {
    let result = super::git::compute_git_status(session);
    super::push_proto_git::push_git_status(push, result);
}

/// Best-effort GIT panel refresh after a Coding-panel mutation (pub for content_search).
pub(crate) fn refresh_git_status_pub(push: &dyn Fn(String), session: Option<&str>) {
    refresh_git_status(push, session);
}

/// Resolve `root` + relative `path` under the workdir sandbox (pub for content_search).
pub(crate) fn resolve_contained_pub(
    root: &str,
    path: &str,
    workdirs: &[PathBuf],
) -> Result<PathBuf, String> {
    resolve_contained(root, path, workdirs)
}

/// Binary sniff used by content_search (NUL in first 8KiB).
pub(crate) fn looks_binary_pub(bytes: &[u8]) -> bool {
    looks_binary(bytes)
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
#[path = "file_ops_test.rs"]
mod tests;

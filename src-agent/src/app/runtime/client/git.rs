//! Host-side GIT STATUS + GIT DIFF computation for the GUI Explore "GIT" panel —
//! both run entirely off the daemon (direct git + fs access, via `std::process::Command`
//! shelling out to the `git` binary) so they answer identically whether a session is
//! attached or not. Clones the [`super::diff`] `FileDiff` feature's plumbing: reuses its
//! `session_workdirs_for` / `looks_binary` / `FILE_DIFF_SIZE_CAP` helpers (bumped to
//! `pub(super)` there) rather than duplicating any of that logic. Unlike `FileDiff`
//! (whose paths are workdir-relative and go through `resolve_diff_path`), every path
//! here is repo-root-relative (straight off `git status --porcelain=v2` run at the repo
//! toplevel) — so it's anchored via [`repo_root_for`] + a plain `root.join(path)`
//! instead.
//!
//! `compute_git_status` / `compute_git_diff` are `pub(super)` since [`super::host`]'s
//! `host_swapper` (a sibling module) and [`super::push_loop`] both call them off a
//! worker thread — exactly mirroring [`super::diff::compute_file_diff`].

use super::diff::{looks_binary, session_workdirs_for, FILE_DIFF_SIZE_CAP};

/// One file entry in a [`GitStatusResult`]'s `staged`/`unstaged` list, mirroring one
/// `XY` half of a `git status --porcelain=v2` record. `status` is a single-character
/// token (`"M"`/`"A"`/`"D"`/`"R"`/`"C"`/`"U"`/`"?"`) the GUI renders as a badge.
/// `orig_path` is `Some` only for a rename/copy record (`2 <XY> …`), showing as
/// `orig -> path`. A single on-disk path can appear in BOTH the `staged` and
/// `unstaged` lists (e.g. `MM` — staged AND further modified since), mirroring
/// VSCode's source-control view — this is intentional, not a bug.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitFileEntry {
    pub path: String,
    pub orig_path: Option<String>,
    pub status: String,
    pub staged: bool,
}

/// The result of a host-side [`compute_git_status`], pushed to the GUI as a
/// `GitStatus` envelope. `error` set means the working directory isn't a git
/// repository (or `git status` itself failed) — every other field is then a neutral
/// default (`root`/`branch` `None`, empty lists) rather than a panic.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitStatusResult {
    pub root: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub staged: Vec<GitFileEntry>,
    pub unstaged: Vec<GitFileEntry>,
    pub error: Option<String>,
}

/// The result of a host-side [`compute_git_diff`], pushed to the GUI as a `GitDiff`
/// envelope. `error` set means the diff could not be computed at all (both strings
/// then empty); `binary` set means either side isn't valid UTF-8 text (both strings
/// then empty, no `error`). `staged` echoes the request — `true` = index-vs-HEAD,
/// `false` = worktree-vs-index — so the reply can't be misapplied to the wrong tab.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitDiffResult {
    pub path: String,
    pub staged: bool,
    pub original: String,
    pub modified: String,
    pub error: Option<String>,
    pub binary: bool,
}

/// Run `git <args>` with `dir` as cwd, `GIT_TERMINAL_PROMPT=0` (never block on an
/// interactive credential prompt — same guard `git_operator.rs` uses), returning
/// `None` on any spawn failure rather than panicking.
fn git_cmd(dir: &std::path::Path, args: &[&str]) -> Option<std::process::Output> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()
}

/// Resolve the git repository root for `session`, probing EVERY one of its configured
/// workdirs in order (mirroring how [`super::diff::resolve_diff_path`] tries each root)
/// via `git rev-parse --show-toplevel`, returning the first that succeeds. Falls back to
/// the host process's own cwd when the session has no workdirs, none of them resolve, or
/// there's no session at all (the StartScreen case). `None` when nothing — not even the
/// cwd fallback — is inside a git repository.
fn repo_root_for(session: Option<&str>) -> Option<std::path::PathBuf> {
    let toplevel = |dir: &std::path::Path| -> Option<std::path::PathBuf> {
        match git_cmd(dir, &["rev-parse", "--show-toplevel"]) {
            Some(out) if out.status.success() => Some(std::path::PathBuf::from(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            )),
            _ => None,
        }
    };

    let dirs = session.and_then(session_workdirs_for).unwrap_or_default();
    for dir in &dirs {
        if let Some(root) = toplevel(dir) {
            return Some(root);
        }
    }
    toplevel(&std::env::current_dir().ok()?)
}

/// Split an ordinary/renamed-or-copied `<XY>` pair into the `staged` (index/`X`) and
/// `unstaged` (worktree/`Y`) lists — `.` means unmodified on that side and is skipped.
/// A file with BOTH sides dirty (e.g. `MM`) lands in both lists, one entry each,
/// mirroring VSCode's source-control view.
fn push_ordinary(
    staged: &mut Vec<GitFileEntry>,
    unstaged: &mut Vec<GitFileEntry>,
    xy: &str,
    path: String,
    orig_path: Option<String>,
) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x != '.' {
        staged.push(GitFileEntry {
            path: path.clone(),
            orig_path: orig_path.clone(),
            status: x.to_string(),
            staged: true,
        });
    }
    if y != '.' {
        unstaged.push(GitFileEntry {
            path,
            orig_path,
            status: y.to_string(),
            staged: false,
        });
    }
}

/// Compute a host-side GIT STATUS for the Explore "GIT" panel, answering a
/// [`super::HostCtl::GitStatus`]. Resolves the repo root off `session`'s primary
/// workdir ([`repo_root_for`]), then parses `git status --porcelain=v2 -z --branch`
/// (NUL-separated records — a rename/copy `2` record consumes an EXTRA NUL field for
/// its original path). ALWAYS returns a result — a non-git workdir sets `error`
/// rather than panicking, mirroring [`super::diff::compute_file_diff`]'s always-reply
/// rule so the GUI panel can never hang waiting on a spinner.
pub(super) fn compute_git_status(session: Option<&str>) -> GitStatusResult {
    let empty = |error: Option<String>| GitStatusResult {
        root: None,
        branch: None,
        detached: false,
        ahead: None,
        behind: None,
        staged: Vec::new(),
        unstaged: Vec::new(),
        error,
    };

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()));
    };

    let status_out = match git_cmd(&root, &["status", "--porcelain=v2", "-z", "--branch"]) {
        Some(out) if out.status.success() => out.stdout,
        _ => return empty(Some("git status failed".to_string())),
    };

    let mut branch = None;
    let mut detached = false;
    let mut ahead = None;
    let mut behind = None;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    // `-z` NUL-terminates every record; a rename/copy record ALSO uses NUL (not tab)
    // between its new path and its original path, so it spans two consecutive fields.
    let fields: Vec<String> = status_out
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();

    let mut i = 0;
    while i < fields.len() {
        let rec = &fields[i];
        let mut head = rec.splitn(2, ' ');
        let kind = head.next().unwrap_or("");
        let rest = head.next().unwrap_or("");
        match kind {
            "#" => {
                // Branch header sub-records: `branch.oid <oid>`, `branch.head <name>`
                // (or `(detached)`), `branch.ab +<ahead> -<behind>` (absent with no
                // upstream). `branch.upstream` is ignored — not surfaced in the panel.
                if let Some(name) = rest.strip_prefix("branch.head ") {
                    if name == "(detached)" {
                        detached = true;
                    } else {
                        branch = Some(name.to_string());
                    }
                } else if let Some(ab) = rest.strip_prefix("branch.ab ") {
                    let mut nums = ab.split_whitespace();
                    ahead = nums.next().and_then(|s| s.strip_prefix('+')).and_then(|s| s.parse().ok());
                    behind = nums.next().and_then(|s| s.strip_prefix('-')).and_then(|s| s.parse().ok());
                }
            }
            // Ordinary changed entry: `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`.
            "1" => {
                let cols: Vec<&str> = rest.splitn(8, ' ').collect();
                if cols.len() == 8 {
                    push_ordinary(&mut staged, &mut unstaged, cols[0], cols[7].to_string(), None);
                }
            }
            // Renamed/copied entry: `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>`,
            // then a SEPARATE NUL field carrying `<origPath>`.
            "2" => {
                let cols: Vec<&str> = rest.splitn(9, ' ').collect();
                i += 1;
                let orig_path = fields.get(i).cloned();
                if cols.len() == 9 {
                    push_ordinary(&mut staged, &mut unstaged, cols[0], cols[8].to_string(), orig_path);
                }
            }
            // Unmerged/conflict: `<XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`.
            "u" => {
                let cols: Vec<&str> = rest.splitn(10, ' ').collect();
                if cols.len() == 10 {
                    unstaged.push(GitFileEntry {
                        path: cols[9].to_string(),
                        orig_path: None,
                        status: "U".to_string(),
                        staged: false,
                    });
                }
            }
            // Untracked: `? <path>`.
            "?" => {
                unstaged.push(GitFileEntry {
                    path: rest.to_string(),
                    orig_path: None,
                    status: "?".to_string(),
                    staged: false,
                });
            }
            // Ignored: `! <path>` — skipped, never surfaced in the panel.
            "!" => {}
            _ => {}
        }
        i += 1;
    }

    GitStatusResult {
        root: Some(root.to_string_lossy().into_owned()),
        branch,
        detached,
        ahead,
        behind,
        staged,
        unstaged,
        error: None,
    }
}

/// Compute a host-side GIT DIFF for `path` (a `GitStatus` file-row's path — ALREADY
/// repo-root-relative, straight off `compute_git_status`'s `git status --porcelain=v2`
/// parse), answering a [`super::HostCtl::GitDiff`]. Resolves the repo root via
/// [`repo_root_for`] FIRST (probing every one of `session`'s configured workdirs, so
/// this works both before a session is attached and when the session's workdir is a
/// subdirectory of the actual repo root), then reads BOTH sides via `git show` using
/// `path` directly (no `resolve_diff_path` / `strip_prefix` — that machinery is for the
/// `FileDiff` feature's workdir-relative addressing, a different scheme):
///
/// - `staged: true` (index vs HEAD) — `original` = `git show HEAD:<path>` (empty for a
///   file newly added / absent at HEAD), `modified` = `git show :<path>` (the staged
///   blob).
/// - `staged: false` (worktree vs index) — `original` = `git show :<path>` (empty for
///   an unstaged/untracked file), `modified` = the CURRENT on-disk contents, read from
///   `root.join(path)`.
///
/// Mirrors `compute_file_diff`'s binary/size-cap/UTF-8 handling exactly (reusing its
/// [`looks_binary`]/[`FILE_DIFF_SIZE_CAP`]). A non-existent blob on either side (`git
/// show` exits non-zero, or the on-disk read fails) is NOT an error — that side is
/// just empty (a valid all-added/all-removed diff); only "not inside a git repository"
/// is reported as `error`.
pub(super) fn compute_git_diff(path: &str, staged: bool, session: Option<&str>) -> GitDiffResult {
    let empty = |error: Option<String>, binary: bool| GitDiffResult {
        path: path.to_string(),
        staged,
        original: String::new(),
        modified: String::new(),
        error,
        binary,
    };

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()), false);
    };

    // `path` is already repo-root-relative — join straight onto `root` for the on-disk
    // read, and pass it as-is into `git show`'s `<rev>:<path>` spec.
    let abs = root.join(path);

    let show = |spec: &str| -> Vec<u8> {
        match git_cmd(&root, &["show", spec]) {
            Some(out) if out.status.success() => out.stdout,
            _ => Vec::new(),
        }
    };

    let (original_bytes, modified_bytes) = if staged {
        (show(&format!("HEAD:{path}")), show(&format!(":{path}")))
    } else {
        let modified = std::fs::read(&abs).unwrap_or_default();
        (show(&format!(":{path}")), modified)
    };

    if original_bytes.len() > FILE_DIFF_SIZE_CAP || modified_bytes.len() > FILE_DIFF_SIZE_CAP {
        return empty(Some("file too large to diff".to_string()), false);
    }
    if looks_binary(&original_bytes) || looks_binary(&modified_bytes) {
        return empty(None, true);
    }

    GitDiffResult {
        path: path.to_string(),
        staged,
        original: String::from_utf8_lossy(&original_bytes).into_owned(),
        modified: String::from_utf8_lossy(&modified_bytes).into_owned(),
        error: None,
        binary: false,
    }
}

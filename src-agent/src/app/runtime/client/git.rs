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

use super::diff::{looks_binary, FILE_DIFF_SIZE_CAP};
use super::git_remote::assigned_key;

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
    /// The SSH key (by name in the vault) currently assigned to this repo root for
    /// remote ops (wave 4b — [`super::git_remote::git_fetch`]/`git_pull`/`git_push`),
    /// or `None` when no key is assigned (remote ops then run with no `GIT_SSH_COMMAND`
    /// override — the system default agent/keys). Looked up via
    /// [`super::git_remote::assigned_key`] keyed by `root`. Additive field — a `None`
    /// repo (`root` itself `None`) always carries `key_name: None` too.
    pub key_name: Option<String>,
    /// Which sequencer op (if any) is currently mid-flight — G5b: `"merge"` /
    /// `"cherry-pick"` / `"revert"` / `"rebase"`, or `None` when the repo is clean.
    /// Detected via [`detect_in_progress`]; drives the GUI's conflict banner
    /// (Abort/Continue), which reads this ALONGSIDE `conflicted` below.
    pub in_progress: Option<String>,
    /// The porcelain-v2 `u` (unmerged/conflict) records, split OUT of `staged`/
    /// `unstaged` into their own list — a conflicted file shouldn't masquerade as an
    /// ordinary staged/unstaged modification. Empty outside a conflict.
    pub conflicted: Vec<GitFileEntry>,
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

/// The result of a host-side git MUTATION (stage/unstage/discard/commit), pushed to
/// the GUI as a `GitOp` envelope. `op` is `"stage"`/`"unstage"`/`"discard"`/`"commit"`
/// so React can react per-kind (e.g. clear the commit box only on a successful
/// commit); `error` (set only when `ok` is `false`) is git's own stderr (or a
/// host-side rejection, e.g. an empty commit message) so the panel can toast it. This
/// envelope carries NO list data — it is always followed by a fresh `GitStatus` push
/// (the mutation worker computes + pushes that right after), which is what actually
/// refreshes the panel's staged/unstaged lists.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitOpResult {
    pub ok: bool,
    pub op: String,
    pub error: Option<String>,
    /// A short human-readable SUCCESS message (wave 4b remote ops — e.g. `git
    /// fetch`/`pull`/`push`'s own stdout/stderr summary), so the Source Control
    /// toolbar can toast what actually happened rather than a bare "push
    /// complete". `None` for every local mutation (stage/unstage/discard/commit —
    /// their success is silent, only a failure toasts) and omitted from the wire
    /// entirely when absent (`skip_serializing_if`), so this is purely additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn op_ok(op: &str) -> GitOpResult {
    GitOpResult { ok: true, op: op.to_string(), error: None, message: None }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult { ok: false, op: op.to_string(), error: Some(error.into()), message: None }
}

/// Run `git <args>` with `dir` as cwd, `GIT_TERMINAL_PROMPT=0` (never block on an
/// interactive credential prompt — same guard `git_operator.rs` uses), returning
/// `None` on any spawn failure rather than panicking. Thin wrapper over
/// [`git_cmd_env`] with no extra env var — every LOCAL git op (status/diff/stage/
/// unstage/discard/commit) goes through this; only [`super::git_remote`]'s
/// fetch/pull/push need the `extra` slot (a `GIT_SSH_COMMAND` override).
pub(super) fn git_cmd(dir: &std::path::Path, args: &[&str]) -> Option<std::process::Output> {
    git_cmd_env(dir, args, None)
}

/// Process-global choke point serializing EVERY host `git` subprocess spawned via
/// [`git_cmd_env`]. A `git checkout` rewrites the working tree and holds
/// `.git/index.lock` for its duration; a linked worktree shares its `.git` with every
/// other worktree/session off the same repo, so an unserialized `git status`/`git log`
/// racing that checkout can collide on the lock file and stall — exactly the GUI
/// branch-switch freeze this guards against. Held only around the subprocess's own
/// exec+wait (see [`git_cmd_env`]), never longer, and only on WORKER threads (never the
/// UI thread), so this purely ORDERS git ops relative to each other — it does not block
/// the GUI. `const Mutex::new` in a `static` needs no `Lazy`/`OnceLock` wrapper on
/// modern Rust.
static GIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// [`git_cmd`]'s general form: same `GIT_TERMINAL_PROMPT=0` guard, plus an optional
/// extra `(name, value)` env var. `pub(super)` so the sibling [`super::git_remote`]
/// module can inject `GIT_SSH_COMMAND` for a repo's assigned key without duplicating
/// the `Command` plumbing (or losing the terminal-prompt guard). This is the SINGLE
/// leaf that actually spawns a `git` process for every host git op (status/diff/stage/
/// unstage/discard/commit/branch/remote/graph/destructive) — every caller in `client/`
/// funnels through here (directly or via a thin same-signature wrapper), which is what
/// makes the [`GIT_LOCK`] below an effective, single choke point rather than one of
/// several. Never called re-entrantly while already holding [`GIT_LOCK`] (this fn is a
/// leaf — it spawns one subprocess and returns — so no nested acquire is possible).
pub(super) fn git_cmd_env(
    dir: &std::path::Path,
    args: &[&str],
    extra: Option<(&str, &str)>,
) -> Option<std::process::Output> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(dir).env("GIT_TERMINAL_PROMPT", "0");
    if let Some((k, v)) = extra {
        cmd.env(k, v);
    }
    // Poison-safe acquire: a panicked holder must not permanently brick every future
    // git op (a bricked lock here would freeze the WHOLE git panel, worse than the bug
    // being fixed). Held only for the scope of this call — the guard drops (and the
    // lock releases) as soon as `cmd.output()` returns, right before this fn returns.
    let _guard = GIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    cmd.output().ok()
}

/// Resolve the git repository root for `session` — the SINGLE choke point every git
/// op funnels through. Now delegates to [`super::git_repos::resolve_repo_root`], which
/// consults the per-session active-repo registry (multi-repo support) and only walks
/// the filesystem to discover repos on first touch. `None` when the session has no
/// workdirs, none hold a repo, or there's no session at all (the StartScreen case) —
/// deliberately does NOT fall back to the host process's own cwd. Signature is
/// unchanged, so all 27 callers stay untouched.
pub(super) fn repo_root_for(session: Option<&str>) -> Option<std::path::PathBuf> {
    super::git_repos::resolve_repo_root(session)
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
        key_name: None,
        in_progress: None,
        conflicted: Vec::new(),
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
    let mut conflicted = Vec::new();

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
            // Split into its OWN `conflicted` list (G5b) — a conflicted file shouldn't
            // masquerade as an ordinary staged/unstaged modification.
            "u" => {
                let cols: Vec<&str> = rest.splitn(10, ' ').collect();
                if cols.len() == 10 {
                    conflicted.push(GitFileEntry {
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

    let key_name = assigned_key(&root);
    let in_progress = detect_in_progress(&root);

    GitStatusResult {
        root: Some(root.to_string_lossy().into_owned()),
        branch,
        detached,
        ahead,
        behind,
        staged,
        unstaged,
        error: None,
        key_name,
        in_progress,
        conflicted,
    }
}

/// Detect an in-flight sequencer op for repo `root` (G5b — feeds
/// [`GitStatusResult::in_progress`]), checked in priority order (first match wins;
/// in practice at most one is ever true):
/// - `"merge"` — `git rev-parse -q --verify MERGE_HEAD` succeeds.
/// - `"cherry-pick"` — `CHERRY_PICK_HEAD` succeeds.
/// - `"revert"` — `REVERT_HEAD` succeeds.
/// - `"rebase"` — the rebase state DIRECTORY exists. Resolved via `git rev-parse
///   --git-path rebase-merge`/`rebase-apply` (NEVER a hardcoded `.git/rebase-*` — a
///   linked worktree's git-dir lives elsewhere entirely, e.g.
///   `.git/worktrees/<name>/rebase-merge`) and then checked for existence on disk —
///   `--git-path` only prints the PATH, it doesn't tell you whether a rebase is
///   actually in progress. `git-path`'s output may be relative (to `root`, since git
///   was run with `root` as cwd) or absolute (some git versions / linked worktrees);
///   `Path::join` handles both correctly (an absolute joinee replaces the base
///   entirely, exactly the semantics wanted here).
///
/// `None` when nothing is in flight.
fn detect_in_progress(root: &std::path::Path) -> Option<String> {
    let head_ref_exists = |name: &str| {
        git_cmd(root, &["rev-parse", "-q", "--verify", name]).is_some_and(|out| out.status.success())
    };
    if head_ref_exists("MERGE_HEAD") {
        return Some("merge".to_string());
    }
    if head_ref_exists("CHERRY_PICK_HEAD") {
        return Some("cherry-pick".to_string());
    }
    if head_ref_exists("REVERT_HEAD") {
        return Some("revert".to_string());
    }

    let rebase_state_dir_exists = |git_path_arg: &str| -> bool {
        let Some(out) = git_cmd(root, &["rev-parse", "--git-path", git_path_arg]) else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if raw.is_empty() {
            return false;
        }
        root.join(raw).is_dir()
    };
    if rebase_state_dir_exists("rebase-merge") || rebase_state_dir_exists("rebase-apply") {
        return Some("rebase".to_string());
    }

    None
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

    // `path` is already repo-root-relative but wire-supplied (untrusted) — anchor it
    // via `safe_join` rather than a plain `root.join(path)` so an absolute path or a
    // `..` traversal can't read arbitrary files into the diff viewer. It's still
    // passed as-is into `git show`'s `<rev>:<path>` spec below — git itself rejects
    // out-of-repo paths there, so only the raw on-disk read needs the guard.
    let Some(abs) = safe_join(&root, path) else {
        return empty(Some("invalid path".to_string()), false);
    };

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

/// Anchor an untrusted, wire-supplied repo-root-relative `rel` onto `root`, rejecting
/// anything that could escape it — an absolute path (`PathBuf::join` would otherwise
/// replace `root` entirely) or any `..`/root/prefix component (would resolve outside
/// `root` at the syscall level). Component-based rejection is the guard; deliberately
/// NOT `canonicalize`-based since the target file may not exist yet (delete path) and
/// symlink resolution semantics differ from what we want here. `None` means reject.
fn safe_join(root: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let relp = std::path::Path::new(rel);
    if relp.is_absolute() {
        return None;
    }
    for c in relp.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return None, // ParentDir, RootDir, Prefix -> reject
        }
    }
    Some(root.join(relp))
}

/// Extract git's own failure message from a non-zero `Output`: prefer stderr
/// (where most git errors land), falling back to stdout (e.g. `git commit`'s
/// "nothing to commit, working tree clean" prints there, not stderr), then a
/// generic fallback if both are empty.
pub(super) fn git_failure(out: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_string()
}

/// Stage `paths` (repo-root-relative, straight off a `GitStatus` row) via `git add --
/// <paths...>`, answering a [`super::HostCtl::GitStage`]. This ALSO stages the removal
/// of a tracked file deleted on disk (`git add`'s own behaviour on a missing path) —
/// intentional, matching VSCode's "Stage All Changes" / per-row stage.
pub(super) fn git_stage(paths: &[String], session: Option<&str>) -> GitOpResult {
    const OP: &str = "stage";
    if paths.is_empty() {
        return op_ok(OP);
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let mut args: Vec<&str> = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    match git_cmd(&root, &args) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git add failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// Unstage `paths` via `git restore --staged -- <paths...>` (moves the index back to
/// HEAD for just those paths, leaving worktree content untouched), answering a
/// [`super::HostCtl::GitUnstage`].
pub(super) fn git_unstage(paths: &[String], session: Option<&str>) -> GitOpResult {
    const OP: &str = "unstage";
    if paths.is_empty() {
        return op_ok(OP);
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let mut args: Vec<&str> = vec!["restore", "--staged", "--"];
    args.extend(paths.iter().map(String::as_str));
    match git_cmd(&root, &args) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git restore --staged failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// Discard unstaged changes for `paths` — VSCode's "Discard Changes" on the Changes
/// (unstaged) group, answering a [`super::HostCtl::GitDiscard`]. PER PATH: a path
/// untracked by git (absent from the index — checked via `git ls-files
/// --error-unmatch`, which also covers "absent at HEAD" since an index-less path was
/// never committed either) is DELETED straight off disk (best-effort — a failed
/// remove is reported but doesn't abort the rest of the batch); a path git already
/// tracks gets `git restore -- <path>`, which resets the WORKTREE from the INDEX —
/// i.e. discards only the unstaged edit, never touching staged content. Restorable
/// paths are batched into a single `git restore` call; deletes are unavoidably
/// per-path (`std::fs::remove_file`).
pub(super) fn git_discard(paths: &[String], session: Option<&str>) -> GitOpResult {
    const OP: &str = "discard";
    if paths.is_empty() {
        return op_ok(OP);
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };

    let mut restore_paths: Vec<&str> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for path in paths {
        let tracked = git_cmd(&root, &["ls-files", "--error-unmatch", "--", path])
            .is_some_and(|out| out.status.success());
        if tracked {
            restore_paths.push(path.as_str());
        } else {
            // Untracked (not in the index) — "discard" means delete the file. A
            // remove failure (permissions, already gone) is collected, not fatal
            // to the rest of the batch. `path` is wire-supplied, so anchor it via
            // `safe_join` FIRST — an absolute path or `..` traversal is rejected
            // (recorded as an error, nothing deleted) rather than escaping `root`.
            let Some(abs) = safe_join(&root, path) else {
                errors.push(format!("{path}: unsafe path"));
                continue;
            };
            if let Err(e) = std::fs::remove_file(abs) {
                errors.push(format!("{path}: {e}"));
            }
        }
    }

    if !restore_paths.is_empty() {
        let mut args: Vec<&str> = vec!["restore", "--"];
        args.extend(restore_paths);
        match git_cmd(&root, &args) {
            Some(out) if out.status.success() => {}
            Some(out) => errors.push(git_failure(&out, "git restore failed")),
            None => errors.push("failed to run git".to_string()),
        }
    }

    if errors.is_empty() {
        op_ok(OP)
    } else {
        op_err(OP, errors.join("; "))
    }
}

/// Commit whatever is CURRENTLY STAGED with `message`, answering a
/// [`super::HostCtl::GitCommit`]. An empty/whitespace-only `message` is rejected
/// OUTRIGHT — no git invocation at all, never an accidental empty-message commit.
/// Runs `git commit -m <message>` (never `-a`, so unstaged changes are untouched);
/// failure (e.g. nothing staged, no configured identity) surfaces git's own message
/// (stderr, falling back to stdout for "nothing to commit, working tree clean").
pub(super) fn git_commit(message: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "commit";
    if message.trim().is_empty() {
        return op_err(OP, "commit message is empty");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd(&root, &["commit", "-m", message]) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git commit failed")),
        None => op_err(OP, "failed to run git"),
    }
}

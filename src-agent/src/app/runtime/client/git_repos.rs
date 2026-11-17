//! MULTI-REPO DISCOVERY + the active-repo registry backing the GUI Source Control
//! panel's repo picker (single-active-repo + picker MVP). Host-local only — never
//! the daemon, exactly like every other `git_*` module here.
//!
//! This is the one place multi-repo lives: the 27 existing git ops (across
//! `git`/`git_graph`/`git_branch`/`git_destructive`/`git_stash`/`git_activity`/
//! `git_remote`) each resolve their repo root through the SINGLE choke point
//! [`super::git::repo_root_for`], whose body now just delegates to
//! [`resolve_repo_root`] here. So a session with several checked-out repos (a bare
//! `workspace/` container holding many projects, or several `/adddir` roots) picks
//! exactly ONE active repo, and every op targets it — no op signature changes.
//!
//! Two cheap paths, one expensive:
//! - [`resolve_repo_root`] is the HOT path (every git op). It validates the cached
//!   active root with a single `.git` existence check and returns — O(1) after the
//!   first touch. Only a missing/invalidated active triggers a discovery.
//! - [`discover_repos`] is the EXPENSIVE path (a bounded filesystem scan, cached
//!   per session for [`REPOS_TTL`]). It runs on first-touch and whenever the picker
//!   is opened (a `GitRepos` request), off a worker thread — NEVER inline on a
//!   control loop.
//! - [`set_active_repo`]/[`active_repo`] are the in-memory registry (GUI-only,
//!   fine to lose on restart), keyed by session id.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Safety cap on discovered repos, so a pathological tree cannot balloon the picker
/// or the wire payload. Silently truncated past this.
const MAX_REPOS: usize = 200;
/// Directory levels scanned below a workdir (`a/b/c/d/e` is found, `a/b/c/d/e/f` is not).
const MAX_SCAN_DEPTH: usize = 5;
/// Directories read per workdir.
const MAX_SCAN_ENTRIES: usize = 20_000;
/// Wall-clock budget for one discovery, shared by all workdirs.
const SCAN_BUDGET: Duration = Duration::from_millis(750);
/// How long a session's discovery result is reused.
const REPOS_TTL: Duration = Duration::from_secs(30);
/// Heavy directories never scanned (compared case-insensitively). Dot-directories
/// are skipped separately.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "venv",
    "__pycache__",
    "Pods",
    "$RECYCLE.BIN",
    "System Volume Information",
];

/// What kind of checkout a discovered repo is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RepoKind {
    #[default]
    Repo,
    Worktree,
    Submodule,
}

/// One discovered repository, serialized to the webview for the picker. `root` is
/// the absolute repo toplevel; `name` is its last path component; `rel` is the path
/// relative to the workdir it was found under ('/'-separated, "." for the workdir's
/// own repo). `rel` and `kind` default so lists from older peers still decode.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepoInfo {
    pub root: String,
    pub name: String,
    #[serde(default)]
    pub rel: String,
    #[serde(default)]
    pub kind: RepoKind,
}

/// The result of a host-side [`discover_repos`] + current-active lookup, pushed to
/// the GUI as a `RepoList` envelope. Mirrors [`super::git_branch::BranchListResult`]
/// (its domain module's result struct) — carried verbatim by the newtype envelope
/// variant, already camelCase. `active` is the currently-selected repo's `root`
/// (matches one of `repos[].root`), or `None` when nothing is selected yet.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepoListResult {
    pub repos: Vec<RepoInfo>,
    pub active: Option<String>,
}

/// Process-global active-repo registry, keyed by session id. GUI-only, in-memory
/// (fine to lose on restart). `const Mutex::new` needs no `Lazy`/`OnceLock`
/// wrapper — but `HashMap::new` isn't `const` (its `RandomState` seed isn't), so
/// the map is wrapped in an `Option` const-initialized to `None` and lazily
/// created on first insert. Poison-safe on every acquire (`unwrap_or_else(|e|
/// e.into_inner())`), same as [`super::git`]'s `GIT_LOCK`: a panicked holder must
/// not permanently brick repo resolution.
static ACTIVE_REPO: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);

type RepoCache = HashMap<String, (Instant, Vec<RepoInfo>)>;

/// Discovery results per session id, reused for [`REPOS_TTL`].
static REPO_CACHE: Mutex<Option<RepoCache>> = Mutex::new(None);

fn cache_get(map: &RepoCache, key: &str, now: Instant, ttl: Duration) -> Option<Vec<RepoInfo>> {
    let (at, repos) = map.get(key)?;
    (now.saturating_duration_since(*at) < ttl).then(|| repos.clone())
}

/// Assign `session`'s active repo to `root`. No-op when there's no session (the
/// StartScreen case — nothing to key on).
pub(crate) fn set_active_repo(session: Option<&str>, root: &str) {
    let Some(session) = session else { return };
    let mut guard = ACTIVE_REPO.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(session.to_string(), PathBuf::from(root));
}

/// Adopt a webview-supplied repo root ONLY if it is one of the session's
/// discovered repos. The picker must only ever hand back a root it was given;
/// this is the trust boundary that keeps the 27 git ops scoped to the
/// session's workspace (mirrors the session_workdirs_for sandbox every other
/// host op enforces). Returns true if accepted.
pub(crate) fn set_active_repo_checked(session: Option<&str>, root: &str) -> bool {
    if session.is_none() {
        return false;
    }
    // Canonicalize the incoming path; a non-existent path fails here and is rejected.
    let want = match std::fs::canonicalize(root) {
        Ok(p) => p,
        Err(_) => {
            crate::model::store::append_global_error_log(
                "gui/git: SetActiveRepo rejected (uncanonicalizable path)",
                &format!("root={root}"),
            );
            return false;
        }
    };
    // Must match a discovered repo for this session. Canonicalize BOTH sides so
    // symlinked/`..` forms compare equal regardless of how discovery stored them.
    let matched = discover_repos(session)
        .into_iter()
        .find(|r| std::fs::canonicalize(&r.root).ok().as_deref() == Some(want.as_path()));
    match matched {
        Some(r) => {
            set_active_repo(session, &r.root);
            true
        }
        None => {
            crate::model::store::append_global_error_log(
                "gui/git: SetActiveRepo rejected (root not among session repos)",
                &format!("root={root}"),
            );
            false
        }
    }
}

/// Look up `session`'s active repo. `None` when unset, or when there's no session.
pub(crate) fn active_repo(session: Option<&str>) -> Option<PathBuf> {
    let session = session?;
    let guard = ACTIVE_REPO.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref()?.get(session).cloned()
}

/// Resolve the repo root every git op targets (the [`super::git::repo_root_for`]
/// body). CHEAP after first touch: a valid cached active root is validated with a
/// single `.git` existence check (no subprocess) and returned. Only a missing or
/// vanished active triggers a [`discover_repos`] scan, whose first result (the first
/// workdir's own repo when it has one) becomes the new active. `None` when no repo can be found at all (no session, no
/// workdirs, or none contain/hold a repo).
pub(crate) fn resolve_repo_root(session: Option<&str>) -> Option<PathBuf> {
    if let Some(active) = active_repo(session) {
        // Cheap validate — no subprocess. `.git` may be a dir OR a gitdir file
        // (linked worktree), `.exists()` covers both.
        if active.join(".git").exists() {
            return Some(active);
        }
    }
    // No valid active -> discover ONCE, adopt the first, cache it as active.
    let repos = discover_repos(session);
    if let Some(first) = repos.first() {
        set_active_repo(session, &first.root);
        return Some(PathBuf::from(&first.root));
    }
    None
}

/// Discover every git repo reachable from `session`'s configured workdirs — the
/// EXPENSIVE path, for the picker + first-touch resolution. Cached per session for
/// [`REPOS_TTL`]; see [`discover_repos_fresh`] to bypass the cache.
///
/// Each workdir's own repo (the `git rev-parse --show-toplevel` probe) comes first,
/// in workdir order. Every repo nested below a workdir follows, sorted by relative
/// path. The nested scan ignores `.gitignore`, so checkouts a parent repo ignores
/// are found; see [`scan_nested`] for its bounds.
pub(crate) fn discover_repos(session: Option<&str>) -> Vec<RepoInfo> {
    discover_cached(session, false)
}

/// [`discover_repos`] that always rescans and refills the cache.
pub(crate) fn discover_repos_fresh(session: Option<&str>) -> Vec<RepoInfo> {
    discover_cached(session, true)
}

fn discover_cached(session: Option<&str>, force: bool) -> Vec<RepoInfo> {
    let Some(session) = session else {
        return Vec::new();
    };
    let dirs = match super::diff::session_workdirs_for(session) {
        Some(d) if !d.is_empty() => d,
        _ => return Vec::new(),
    };
    if !force {
        let guard = REPO_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = guard
            .as_ref()
            .and_then(|m| cache_get(m, session, Instant::now(), REPOS_TTL))
        {
            return hit;
        }
    }
    let repos = discover_repos_uncached(&dirs);
    // An empty result is not cached: a `git init` in a non-repo folder must be found
    // on the next call, not after the TTL.
    if !repos.is_empty() {
        REPO_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(session.to_string(), (Instant::now(), repos.clone()));
    }
    repos
}

fn discover_repos_uncached(dirs: &[PathBuf]) -> Vec<RepoInfo> {
    let deadline = Instant::now() + SCAN_BUDGET;
    let mut primary: Vec<(PathBuf, String)> = Vec::new();
    for workdir in dirs {
        if let Some(top) = workdir_toplevel(workdir) {
            let same = match (std::fs::canonicalize(&top), std::fs::canonicalize(workdir)) {
                (Ok(a), Ok(b)) => a == b,
                _ => top == *workdir,
            };
            let rel = if same {
                ".".to_string()
            } else {
                rel_label(workdir, &top)
            };
            primary.push((top, rel));
        }
    }
    let mut nested: Vec<(PathBuf, String)> = Vec::new();
    for workdir in dirs {
        let mut found = Vec::new();
        scan_nested(
            workdir,
            deadline,
            MAX_SCAN_DEPTH,
            MAX_SCAN_ENTRIES,
            MAX_REPOS,
            &mut found,
        );
        nested.extend(found.into_iter().map(|p| {
            let rel = rel_label(workdir, &p);
            (p, rel)
        }));
    }
    nested.sort_by_key(|(_, rel)| rel.to_lowercase());

    // Canonicalize so a repo reachable via two workdirs or a symlink dedups to one
    // entry; the first occurrence wins, which keeps the primary entries first.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut infos: Vec<RepoInfo> = Vec::new();
    for (p, rel) in primary.into_iter().chain(nested) {
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if !seen.insert(p.clone()) {
            continue;
        }
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        infos.push(RepoInfo {
            root: display_path(&p),
            name,
            rel,
            kind: classify_repo(&p),
        });
    }
    infos.truncate(MAX_REPOS);
    infos
}

/// `git rev-parse --show-toplevel` in `dir` — the repo toplevel `dir` is inside, or
/// `None` when `dir` isn't in a git repo. Same probe (and choke point,
/// [`super::git::git_cmd`]) the old [`super::git::repo_root_for`] body used.
fn workdir_toplevel(dir: &Path) -> Option<PathBuf> {
    match super::git::git_cmd(dir, &["rev-parse", "--show-toplevel"]) {
        Some(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(PathBuf::from(s))
            }
        }
        _ => None,
    }
}

/// Walk `root`'s subtree iteratively, appending every directory holding a `.git`
/// entry (dir or gitdir file) to `out`. `root` itself is never recorded. The walk
/// descends into repos too and ignores `.gitignore`, never follows symlinks, skips
/// dot-directories and [`SKIP_DIRS`], and stops at `deadline`, after `max_entries`
/// directories, or once `out` holds `max_repos`. `max_depth` counts directory
/// levels below `root`. Unreadable directories are skipped.
fn scan_nested(
    root: &Path,
    deadline: Instant,
    max_depth: usize,
    max_entries: usize,
    max_repos: usize,
    out: &mut Vec<PathBuf>,
) {
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if Instant::now() > deadline || visited >= max_entries || out.len() >= max_repos {
            return;
        }
        visited += 1;
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP_DIRS.iter().any(|d| d.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let child = entry.path();
            if child.join(".git").exists() {
                if out.len() >= max_repos {
                    return;
                }
                out.push(child.clone());
            }
            if depth + 1 < max_depth {
                stack.push((child, depth + 1));
            }
        }
    }
}

/// Classify a `gitdir: <path>` pointer file: a path through `worktrees/` is a linked
/// worktree (checked first, since a submodule's worktree also passes through
/// `modules/`), through `modules/` a submodule, anything else a plain repo.
fn classify_gitdir_text(text: &str) -> RepoKind {
    let text = text.replace('\\', "/");
    let Some(path) = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:")) else {
        return RepoKind::Repo;
    };
    if path.contains("/worktrees/") {
        RepoKind::Worktree
    } else if path.contains("/modules/") {
        RepoKind::Submodule
    } else {
        RepoKind::Repo
    }
}

fn classify_repo(dir: &Path) -> RepoKind {
    let git = dir.join(".git");
    if git.is_file() {
        std::fs::read_to_string(git)
            .map(|t| classify_gitdir_text(&t))
            .unwrap_or_default()
    } else {
        RepoKind::Repo
    }
}

/// `root` relative to `workdir`, '/'-separated; "." when equal; the root's own name
/// when it is not below the workdir (a toplevel above it).
fn rel_label(workdir: &Path, root: &Path) -> String {
    match root.strip_prefix(workdir) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Drop a Windows verbatim prefix: `\\?\UNC\srv\share` -> `\\srv\share`,
/// `\\?\C:\x` -> `C:\x`. Anything else is returned unchanged.
#[cfg_attr(not(windows), allow(dead_code))]
fn strip_verbatim_prefix(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// The string form of a repo root sent to the webview and compared back by the
/// store. `canonicalize` yields verbatim `\\?\` paths on Windows; strip them so
/// every consumer sees the same plain form.
fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        strip_verbatim_prefix(&s)
    }
    #[cfg(not(windows))]
    {
        s
    }
}

#[cfg(test)]
#[path = "git_repos_test.rs"]
mod tests;

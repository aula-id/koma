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
//! - [`discover_repos`] is the EXPENSIVE path (filesystem walks). It runs on
//!   first-touch and whenever the picker is opened (a `GitRepos` request), off a
//!   worker thread — NEVER inline on a control loop.
//! - [`set_active_repo`]/[`active_repo`] are the in-memory registry (GUI-only,
//!   fine to lose on restart), keyed by session id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Safety cap on discovered repos — a pathological container (thousands of nested
/// checkouts) must not balloon the picker or the wire payload. Silently truncated
/// past this for the MVP.
const MAX_REPOS: usize = 50;

/// Depth cap for the sub-repo subtree walk — a safety net so an accidentally-huge
/// container tree can't run the walker unbounded (gitignore/.dockerignore pruning
/// plus the stop-descend-on-repo rule already keep it small in practice).
const MAX_WALK_DEPTH: usize = 8;

/// One discovered repository, serialized to the webview for the picker. `root` is
/// the absolute repo toplevel (a `git rev-parse --show-toplevel`, or a discovered
/// `.git`-bearing dir); `name` is its last path component, the picker's label.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepoInfo {
    pub root: String,
    pub name: String,
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
/// vanished active triggers a [`discover_repos`] walk, whose first result becomes
/// the new active. `None` when no repo can be found at all (no session, no
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
/// EXPENSIVE path (filesystem walks), for the picker + first-touch resolution.
///
/// Per workdir: probe whether the workdir itself is inside a repo (`git rev-parse
/// --show-toplevel`, the same probe [`super::git::repo_root_for`] used); if so it's
/// a normal single-project open — record that one toplevel and DON'T scan children
/// (they belong to that repo). Otherwise the workdir is a bare container — walk its
/// subtree (respecting `.gitignore` by default + `.dockerignore`, no symlink
/// following, depth-capped) recording every `.git`-bearing dir and STOPPING descent
/// at each (a repo's inner tree holds no separate repos we care about). Walk errors
/// are skipped silently.
///
/// The combined candidates are canonicalized to absolute paths, deduped (a repo
/// reachable from two workdirs appears once), nesting-filtered (a root inside
/// another discovered root is dropped — top-most wins), capped at [`MAX_REPOS`],
/// and returned sorted by `name` (case-insensitive) for a stable picker order.
pub(crate) fn discover_repos(session: Option<&str>) -> Vec<RepoInfo> {
    let dirs = match session.and_then(super::diff::session_workdirs_for) {
        Some(d) if !d.is_empty() => d,
        _ => return Vec::new(),
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    for workdir in &dirs {
        match workdir_toplevel(workdir) {
            // Workdir is itself inside a repo — one entry, no child scan.
            Some(top) => candidates.push(top),
            // Bare container — scan for sub-repos.
            None => scan_subrepos(workdir, &mut candidates),
        }
    }

    // Canonicalize (resolve symlinks + absolutize) so a repo reachable via two
    // workdirs — or a symlinked path — dedups to one entry. Fall back to the raw
    // path if canonicalize fails (path vanished mid-walk — rare).
    let mut roots: Vec<PathBuf> = candidates
        .into_iter()
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect();
    roots.sort();
    roots.dedup();

    // Drop any root nested INSIDE another discovered root (keep the top-most).
    // Sorted ascending, an ancestor always precedes its descendants, so a root is
    // kept only when no already-kept root is a path-prefix of it.
    let mut kept: Vec<PathBuf> = Vec::new();
    for r in roots {
        if !kept.iter().any(|k| r.starts_with(k)) {
            kept.push(r);
        }
    }
    kept.truncate(MAX_REPOS);

    let mut infos: Vec<RepoInfo> = kept
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            RepoInfo {
                root: p.to_string_lossy().into_owned(),
                name,
            }
        })
        .collect();
    infos.sort_by_key(|a| a.name.to_lowercase());
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

/// Walk `container`'s subtree, appending every discovered repo root (a dir holding
/// a `.git` entry) to `out` and STOPPING descent at each. Respects `.gitignore`
/// (the `ignore` crate default) plus `.dockerignore`, never follows symlinks
/// (default — avoids loops), and is depth-capped ([`MAX_WALK_DEPTH`]) as a safety
/// net. `filter_entry` returning `false` both records the repo (via a captured
/// `Arc<Mutex<..>>`, poison-safe) and prunes its subtree; the walk is driven by
/// consuming the iterator, whose `DirEntry`s are otherwise unused. Unreadable dirs
/// / walk errors are skipped silently (`.flatten()`).
fn scan_subrepos(container: &Path, out: &mut Vec<PathBuf>) {
    let found: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = found.clone();
    // Crate default `hidden(true)` skips dot-directories (e.g. `.local/`), so repos
    // nested inside one are NOT discovered here — intentional, matches VSCode-ish
    // behavior; not a bug to fix.
    let walker = ignore::WalkBuilder::new(container)
        // Treat `.dockerignore` as an extra ignore file, on top of the default
        // `.gitignore` handling — a container often carries both.
        .add_custom_ignore_filename(".dockerignore")
        .max_depth(Some(MAX_WALK_DEPTH))
        .filter_entry(move |dent| {
            if dent.file_type().is_some_and(|t| t.is_dir()) {
                let p = dent.path();
                // `.git` may be a dir OR a gitdir file (linked worktree) —
                // `.exists()` covers both.
                if p.join(".git").exists() {
                    sink.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(p.to_path_buf());
                    return false; // record repo root, stop descending into it
                }
            }
            true
        })
        .build();
    // Drive the walk (filter_entry side-effects do the recording); entries unused.
    for _ in walker.flatten() {}

    let repos = std::mem::take(&mut *found.lock().unwrap_or_else(|e| e.into_inner()));
    out.extend(repos);
}

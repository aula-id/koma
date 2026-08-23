//! Host-side BRANCH LIST / CHECKOUT / CREATE-BRANCH computation for the GUI's
//! branch-switcher (footer button + `GitPanel` header) and the commit-graph
//! right-click context menu — G4 (safe branch interactions only; no
//! conflict-capable ops — cherry-pick/merge/rebase/reset/revert are G5).
//! Mirrors [`super::git`]'s exact host-relay pattern (`git_cmd`/`git_cmd_env`
//! choke point, an error-flagged result struct instead of a panic,
//! `#[serde(rename_all = "camelCase")]` DTOs, a `GitOpResult { ok, op, error,
//! message }` reply for a mutation) rather than duplicating any of its
//! plumbing: reuses [`super::git::repo_root_for`]/`git_failure`/`GitOpResult`
//! and [`super::git_graph::valid_commit_ref`]. Host-local only — never the
//! daemon, never `git_cred.rs`/`git_operator.rs` (the model's own git
//! credential machinery).
//!
//! Every mutation here is deliberately SAFE — a bare `git checkout`/`git
//! branch`, never `--force`, never touching the working tree's staged/
//! unstaged content — so a failure (dirty worktree in the way, a name
//! collision) surfaces git's own stderr rather than clobbering anything.

use super::git::{git_cmd_env, git_failure, repo_root_for, GitOpResult};
use super::git_graph::valid_commit_ref;

/// Run `git <args>` with `dir` as cwd, no extra env — thin wrapper over
/// [`git_cmd_env`], mirroring [`super::git_graph`]'s own private `git_cmd`
/// (a deliberate one-line duplicate per file, not a new plumbing layer —
/// [`super::git`]'s own `git_cmd` is a bare private fn, not reachable here).
fn git_cmd(dir: &std::path::Path, args: &[&str]) -> Option<std::process::Output> {
    git_cmd_env(dir, args, None)
}

fn op_ok(op: &str) -> GitOpResult {
    GitOpResult {
        ok: true,
        op: op.to_string(),
        error: None,
        message: None,
    }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult {
        ok: false,
        op: op.to_string(),
        error: Some(error.into()),
        message: None,
    }
}

/// One ref entry in a [`BranchListResult`], parsed off one `git for-each-ref`
/// record. `kind` is `"local"` (`refs/heads/…`), `"remote"` (`refs/remotes/…` —
/// a real remote-tracking ref, e.g. `origin/main`), or `"tag"` (`refs/tags/…` —
/// GK4a, listed alongside branches for the React ref-tree; a tag is never a
/// switch/checkout TARGET's `is_current`, so it always carries `false`);
/// `is_current` marks the SINGLE entry `git for-each-ref`'s `%(HEAD)` flags with
/// `*` (the branch HEAD currently points at — never set for a remote or tag
/// entry, since HEAD can only point at a local branch or be detached).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BranchInfo {
    pub name: String,
    pub kind: String,
    pub is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// The result of a host-side [`git_branch_list`], pushed to the GUI as a
/// `BranchList` envelope. `error` set means the workdir isn't a git repository
/// (or the `for-each-ref` itself failed) — `branches` is then empty rather
/// than the caller panicking.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BranchListResult {
    pub branches: Vec<BranchInfo>,
    pub error: Option<String>,
    /// Repo identity used to reject stale picker actions after an active-repo switch.
    pub root: Option<String>,
    /// Echoed request generation. Optional for protocol compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<u64>,
}

/// Reject anything that isn't a syntactically valid git branch NAME before it
/// is ever interpolated into a `git branch`/`git checkout -b` arg. Shells out
/// to `git check-ref-format --branch <name>` — the authoritative check git
/// itself uses (covers every rule: no leading `-`, no `..`, no whitespace, no
/// `~^:?*[`, no trailing `.`/`.lock`, etc.) rather than hand-rolling a partial
/// allowlist that could drift from git's own rules. No shell is involved (the
/// name rides `Command`'s arg vector, never a shell string); `root` is only a
/// cwd for the subprocess (the check itself doesn't need to be run inside a
/// repository, but every other call site already has one resolved).
fn valid_branch_name(root: &std::path::Path, name: &str) -> bool {
    if name.is_empty() || name.len() > 200 {
        return false;
    }
    match git_cmd(root, &["check-ref-format", "--branch", name]) {
        Some(out) => out.status.success(),
        None => false,
    }
}

/// Parse NUL-delimited `git worktree list --porcelain -z` fields. NUL framing
/// preserves paths containing newlines as well as spaces.
fn parse_worktree_output(stdout: &[u8]) -> std::collections::HashMap<String, String> {
    fn finish_record(
        worktrees: &mut std::collections::HashMap<String, String>,
        path: &mut Option<String>,
        branch: &mut Option<String>,
    ) {
        if let (Some(path), Some(branch)) = (path.take(), branch.take()) {
            worktrees.insert(branch, path);
        } else {
            *path = None;
            *branch = None;
        }
    }

    let mut worktrees = std::collections::HashMap::new();
    let mut path = None;
    let mut branch = None;
    for field in stdout.split(|b| *b == 0) {
        if field.is_empty() {
            finish_record(&mut worktrees, &mut path, &mut branch);
        } else if let Some(value) = field.strip_prefix(b"worktree ") {
            if path.is_some() {
                finish_record(&mut worktrees, &mut path, &mut branch);
            }
            path = Some(String::from_utf8_lossy(value).into_owned());
        } else if let Some(value) = field.strip_prefix(b"branch refs/heads/") {
            branch = Some(String::from_utf8_lossy(value).into_owned());
        }
    }
    finish_record(&mut worktrees, &mut path, &mut branch);
    worktrees
}

fn parse_branch_output(
    stdout: &[u8],
    worktrees: &std::collections::HashMap<String, String>,
) -> Vec<BranchInfo> {
    let text = String::from_utf8_lossy(stdout);
    let mut branches = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let refname = parts.next().unwrap_or("");
        let is_current = parts.next().unwrap_or("").trim() == "*";
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            branches.push(BranchInfo {
                name: name.to_string(),
                kind: "local".to_string(),
                is_current,
                worktree_path: worktrees.get(name).cloned(),
            });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            if !name.ends_with("/HEAD") {
                branches.push(BranchInfo {
                    name: name.to_string(),
                    kind: "remote".to_string(),
                    is_current: false,
                    worktree_path: None,
                });
            }
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            branches.push(BranchInfo {
                name: name.to_string(),
                kind: "tag".to_string(),
                is_current: false,
                worktree_path: None,
            });
        }
    }
    branches
}

fn worktree_branches(
    root: &std::path::Path,
) -> Result<std::collections::HashMap<String, String>, String> {
    match git_cmd(root, &["worktree", "list", "--porcelain", "-z"]) {
        Some(out) if out.status.success() => Ok(parse_worktree_output(&out.stdout)),
        Some(out) => Err(git_failure(&out, "git worktree list failed")),
        None => Err("failed to run git".to_string()),
    }
}

fn occupied_worktree_path<'a>(
    worktrees: &'a std::collections::HashMap<String, String>,
    branch: &str,
    current_root: &std::path::Path,
) -> Option<&'a str> {
    worktrees
        .get(branch)
        .filter(|path| std::path::Path::new(path.as_str()) != current_root)
        .map(String::as_str)
}

fn local_branch_exists(root: &std::path::Path, name: &str) -> Result<bool, String> {
    let full_ref = format!("refs/heads/{name}");
    match git_cmd(root, &["show-ref", "--verify", "--quiet", &full_ref]) {
        Some(out) if out.status.success() => Ok(true),
        Some(out) if out.status.code() == Some(1) => Ok(false),
        Some(out) => Err(git_failure(&out, "git show-ref failed")),
        None => Err("failed to run git".to_string()),
    }
}

/// List local branches, remote-tracking branches, and tags. Worktree occupancy
/// comes from `git worktree list --porcelain`, rather than the version-dependent
/// `for-each-ref %(worktreepath)` atom.
pub(crate) fn git_branch_list(session: Option<&str>, request_id: Option<u64>) -> BranchListResult {
    let empty = |error: Option<String>| BranchListResult {
        branches: Vec::new(),
        error,
        root: None,
        request_id,
    };

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()));
    };

    let stdout = match git_cmd(
        &root,
        &[
            "for-each-ref",
            "--format=%(refname)%09%(HEAD)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    ) {
        Some(out) if out.status.success() => out.stdout,
        Some(out) => return empty(Some(git_failure(&out, "git for-each-ref failed"))),
        None => return empty(Some("failed to run git".to_string())),
    };
    let worktrees = match worktree_branches(&root) {
        Ok(worktrees) => worktrees,
        Err(error) => return empty(Some(error)),
    };

    BranchListResult {
        branches: parse_branch_output(&stdout, &worktrees),
        error: None,
        root: Some(root.to_string_lossy().into_owned()),
        request_id,
    }
}

/// Safely switch to `ref_name` without force. Before checking out an existing
/// local branch, reject it when another worktree already has it checked out.
pub(crate) fn git_checkout(
    ref_name: &str,
    expected_root: Option<&str>,
    session: Option<&str>,
) -> GitOpResult {
    const OP: &str = "checkout";
    if !valid_commit_ref(ref_name) {
        return op_err(OP, "invalid ref");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    // Keep this stale-picker guard before any action based on the requested ref.
    if expected_root.is_some_and(|expected| root != std::path::Path::new(expected)) {
        return op_err(OP, "repository changed; refresh branches and try again");
    }

    match local_branch_exists(&root, ref_name) {
        Ok(true) => match worktree_branches(&root) {
            Ok(worktrees) => {
                if let Some(path) = occupied_worktree_path(&worktrees, ref_name, &root) {
                    return op_err(
                        OP,
                        format!("branch '{ref_name}' is already checked out at '{path}'"),
                    );
                }
            }
            Err(error) => return op_err(OP, error),
        },
        Ok(false) => {}
        Err(error) => return op_err(OP, error),
    }

    super::git_remote::invalidate_rebase_proofs(&root);
    match git_cmd(&root, &["checkout", ref_name]) {
        Some(out) if out.status.success() => {
            super::git_remote::invalidate_rebase_proofs(&root);
            op_ok(OP)
        }
        Some(out) => op_err(OP, git_failure(&out, "git checkout failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// Create branch `name`, optionally checking it out, without force.
pub(crate) fn git_create_branch(
    name: &str,
    start: Option<&str>,
    checkout: bool,
    expected_root: Option<&str>,
    session: Option<&str>,
) -> GitOpResult {
    const OP: &str = "createBranch";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    if expected_root.is_some_and(|expected| root != std::path::Path::new(expected)) {
        return op_err(OP, "repository changed; refresh branches and try again");
    }
    if !valid_branch_name(&root, name) {
        return op_err(OP, "invalid branch name");
    }
    if let Some(s) = start {
        if !valid_commit_ref(s) {
            return op_err(OP, "invalid start point");
        }
    }

    let mut args: Vec<&str> = if checkout {
        vec!["checkout", "-b", name]
    } else {
        vec!["branch", name]
    };
    if let Some(s) = start {
        args.push(s);
    }

    super::git_remote::invalidate_rebase_proofs(&root);
    match git_cmd(&root, &args) {
        Some(out) if out.status.success() => {
            super::git_remote::invalidate_rebase_proofs(&root);
            op_ok(OP)
        }
        Some(out) => op_err(OP, git_failure(&out, "git branch failed")),
        None => op_err(OP, "failed to run git"),
    }
}

#[cfg(test)]
#[path = "git_branch_test.rs"]
mod tests;

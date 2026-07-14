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
    GitOpResult { ok: true, op: op.to_string(), error: None, message: None }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult { ok: false, op: op.to_string(), error: Some(error.into()), message: None }
}

/// One ref entry in a [`BranchListResult`], parsed off one `git for-each-ref`
/// record. `kind` is `"local"` (`refs/heads/…`), `"remote"` (`refs/remotes/…` —
/// a real remote-tracking ref, e.g. `origin/main`), or `"tag"` (`refs/tags/…` —
/// GK4a, listed alongside branches for the React ref-tree; a tag is never a
/// switch/checkout TARGET's `is_current`, so it always carries `false`);
/// `is_current` marks the SINGLE entry `git for-each-ref`'s `%(HEAD)` flags with
/// `*` (the branch HEAD currently points at — never set for a remote or tag
/// entry, since HEAD can only point at a local branch or be detached).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BranchInfo {
    pub name: String,
    pub kind: String,
    pub is_current: bool,
}

/// The result of a host-side [`git_branch_list`], pushed to the GUI as a
/// `BranchList` envelope. `error` set means the workdir isn't a git repository
/// (or the `for-each-ref` itself failed) — `branches` is then empty rather
/// than the caller panicking.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BranchListResult {
    pub branches: Vec<BranchInfo>,
    pub error: Option<String>,
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

/// Compute a host-side BRANCH LIST (every local + remote-tracking branch, PLUS
/// every tag — GK4a), answering a [`super::HostCtl::GitBranchList`]. Resolves the
/// repo root off `session` ([`repo_root_for`]), then runs `git for-each-ref --format=
/// %(refname)%09%(HEAD) refs/heads refs/remotes refs/tags`: each record is
/// `<refname>\t<HEAD-marker>`, `HEAD-marker` being `*` for the one entry HEAD
/// currently points at, else empty. `refs/heads/X` -> local branch `X`;
/// `refs/remotes/X` -> remote branch `X` (e.g. `origin/main`) — EXCEPT a
/// remote's symbolic `HEAD` pointer (`refs/remotes/origin/HEAD`, `X` ending in
/// `/HEAD`), which is skipped (not a real branch, just the remote's default-
/// branch pointer); `refs/tags/X` -> tag `X`, `is_current` always `false`. The
/// React ref-tree groups the combined list by `kind`; a branch SWITCHER further
/// filters to local/remote only (a React-side concern, not this fn's). ALWAYS
/// returns a result — a non-git workdir sets `error` rather than panicking,
/// mirroring [`super::git::compute_git_status`].
pub(super) fn git_branch_list(session: Option<&str>) -> BranchListResult {
    let empty = |error: Option<String>| BranchListResult { branches: Vec::new(), error };

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

    let text = String::from_utf8_lossy(&stdout);
    let mut branches = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let refname = parts.next().unwrap_or("");
        let is_current = parts.next().unwrap_or("").trim() == "*";
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            branches.push(BranchInfo { name: name.to_string(), kind: "local".to_string(), is_current });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            // Skip a remote's symbolic HEAD pointer (`origin/HEAD`) — not a real
            // branch to switch to, just the remote's default-branch marker.
            if name.ends_with("/HEAD") {
                continue;
            }
            branches.push(BranchInfo { name: name.to_string(), kind: "remote".to_string(), is_current: false });
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            branches.push(BranchInfo { name: name.to_string(), kind: "tag".to_string(), is_current: false });
        }
    }

    BranchListResult { branches, error: None }
}

/// Switch (or detach onto) `ref_name` — a branch name or a commit sha —
/// answering a [`super::HostCtl::GitCheckout`]. Validated via
/// [`valid_commit_ref`] (accepts both a branch name and a sha/short-sha)
/// before ever touching `git`. Runs a BARE `git checkout <ref_name>` — a
/// branch name switches (creating a remote-tracking local branch via git's own
/// DWIM if it's a bare remote short name like `origin/x` -> `x`), a sha goes
/// DETACHED — never `--force`, so a dirty worktree in the way surfaces git's
/// own stderr (e.g. "Your local changes would be overwritten") rather than
/// being clobbered. `op` is `"checkout"`.
pub(super) fn git_checkout(ref_name: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "checkout";
    if !valid_commit_ref(ref_name) {
        return op_err(OP, "invalid ref");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd(&root, &["checkout", ref_name]) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git checkout failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// Create branch `name`, answering a [`super::HostCtl::GitCreateBranch`].
/// `name` is validated via [`valid_branch_name`] (git's own `check-ref-format`
/// rules); `start` (the commit-ish to branch from — `None` means "current
/// HEAD", git's own default when omitted) is validated via
/// [`valid_commit_ref`] when present. `checkout: true` switches to the new
/// branch immediately (`git checkout -b <name> [<start>]`); `false` only
/// creates it, leaving HEAD where it was (`git branch <name> [<start>]`). Op
/// is `"createBranch"`. A name collision / invalid start point surfaces git's
/// own stderr.
pub(super) fn git_create_branch(
    name: &str,
    start: Option<&str>,
    checkout: bool,
    session: Option<&str>,
) -> GitOpResult {
    const OP: &str = "createBranch";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    if !valid_branch_name(&root, name) {
        return op_err(OP, "invalid branch name");
    }
    if let Some(s) = start {
        if !valid_commit_ref(s) {
            return op_err(OP, "invalid start point");
        }
    }

    let mut args: Vec<&str> = if checkout { vec!["checkout", "-b", name] } else { vec!["branch", name] };
    if let Some(s) = start {
        args.push(s);
    }

    match git_cmd(&root, &args) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git branch failed")),
        None => op_err(OP, "failed to run git"),
    }
}

//! Host-side INTERACTIVE / DESTRUCTIVE git ops for a GitKraken-style commit-graph
//! panel — G5b. Mirrors [`super::git_branch`]'s exact host-relay pattern (a
//! `git_cmd_env` choke point, an error-flagged [`GitOpResult`] reply instead of a
//! panic, every sha/ref/mode/kind validated BEFORE any git call) rather than
//! duplicating any of its plumbing: reuses [`super::git::repo_root_for`]/
//! `git_failure`/`GitOpResult`/`git_cmd_env` and [`super::git_graph::valid_commit_ref`].
//! Host-local only — never the daemon, never `git_cred.rs`/`git_operator.rs`.
//!
//! Every op here can leave the working tree in an IN-PROGRESS / CONFLICTED state
//! (cherry-pick, revert, merge, rebase) — that is NOT reported as `ok: false` here
//! (git itself exits non-zero on a conflict, so it DOES surface as an error message,
//! but it is not a bug): the follow-up [`super::git::compute_git_status`] the caller
//! always re-runs after every mutation carries the authoritative `inProgress` /
//! `conflicted` state (see [`super::git`]'s doc), which is what the GUI's conflict
//! banner (Abort/Continue) actually renders off. [`git_op_abort`]/[`git_op_continue`]
//! resolve that in-progress state; `git_reset --hard`'s destructiveness is gated by a
//! React confirm BEFORE this is ever called — the host just runs it, no double-check.

use super::git::{git_cmd_env, git_failure, repo_root_for, GitOpResult};
use super::git_graph::valid_commit_ref;

fn op_ok(op: &str) -> GitOpResult {
    GitOpResult { ok: true, op: op.to_string(), error: None, message: None }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult { ok: false, op: op.to_string(), error: Some(error.into()), message: None }
}

/// Reject anything that isn't one of the 4 sequencer op kinds a `--abort`/`--continue`
/// can run against — checked BEFORE the kind is ever interpolated into a `git <kind>
/// --abort/--continue` arg (positionally the git SUBCOMMAND itself, so a bogus kind
/// would otherwise be handed straight to `Command` as an arbitrary git subcommand).
fn valid_op_kind(kind: &str) -> bool {
    matches!(kind, "merge" | "rebase" | "cherry-pick" | "revert")
}

/// `git cherry-pick <sha>`, answering a [`super::HostCtl::GitCherryPick`] (commit-graph
/// row context menu). `sha` is validated via [`valid_commit_ref`] first. A conflict
/// (non-zero exit, working tree left conflicted) surfaces git's own stderr as `error`;
/// the caller's follow-up `GitStatus` push is what actually reports the conflicted
/// state to the panel (see the module doc).
pub(super) fn git_cherry_pick(sha: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "cherryPick";
    if !valid_commit_ref(sha) {
        return op_err(OP, "invalid commit reference");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd_env(&root, &["cherry-pick", sha], None) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git cherry-pick failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git revert --no-edit <sha>`, answering a [`super::HostCtl::GitRevert`]. Same
/// validation + conflict reasoning as [`git_cherry_pick`]. `--no-edit` accepts git's
/// default revert message rather than opening an editor (this is a headless GUI host —
/// there is no editor to open).
pub(super) fn git_revert(sha: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "revert";
    if !valid_commit_ref(sha) {
        return op_err(OP, "invalid commit reference");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd_env(&root, &["revert", "--no-edit", sha], None) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git revert failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git reset --<mode> <sha>`, answering a [`super::HostCtl::GitReset`] (commit-graph
/// row context menu "Reset branch to here"). `mode` is checked against a STRICT
/// allowlist (`"soft"`/`"mixed"`/`"hard"`) BEFORE ever building the `--<mode>` flag —
/// rejecting anything else means an arbitrary string can never be interpolated into a
/// git flag. `hard` DISCARDS uncommitted working-tree + index changes; that
/// destructiveness is gated by a confirm on the REACT side before this is ever called —
/// the host runs exactly what it's asked, no extra safety net.
pub(super) fn git_reset(sha: &str, mode: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "reset";
    if !valid_commit_ref(sha) {
        return op_err(OP, "invalid commit reference");
    }
    if !matches!(mode, "soft" | "mixed" | "hard") {
        return op_err(OP, "invalid reset mode");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let flag = format!("--{mode}");
    match git_cmd_env(&root, &["reset", &flag, sha], None) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git reset failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git merge --no-edit <ref_>`, answering a [`super::HostCtl::GitMerge`]
/// (branch-switcher / graph context menu "Merge into current branch"). `ref_` (a
/// branch name or a sha) is validated via [`valid_commit_ref`] first. May conflict —
/// same reasoning as [`git_cherry_pick`].
pub(super) fn git_merge(ref_: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "merge";
    if !valid_commit_ref(ref_) {
        return op_err(OP, "invalid ref");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd_env(&root, &["merge", "--no-edit", ref_], None) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git merge failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git rebase <upstream> [branch]`, answering a [`super::HostCtl::GitRebase`].
/// `upstream` (a branch name or a sha) is validated via [`valid_commit_ref`] first;
/// when `branch` is `Some` (the GitKraken-style drag-to-rebase: drag branch `branch`
/// onto `upstream`) it is validated the same way before being appended — git checks
/// `branch` out and rebases IT onto `upstream`, leaving the current branch untouched.
/// `branch: None` rebases the CURRENT branch instead (the plain G5b op). No
/// interactive/`--onto` support here. May conflict — same reasoning as
/// [`git_cherry_pick`].
pub(super) fn git_rebase(upstream: &str, branch: Option<&str>, session: Option<&str>) -> GitOpResult {
    const OP: &str = "rebase";
    if !valid_commit_ref(upstream) {
        return op_err(OP, "invalid upstream");
    }
    if let Some(b) = branch {
        if !valid_commit_ref(b) {
            return op_err(OP, "invalid branch");
        }
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let args: Vec<&str> = match branch {
        Some(b) => vec!["rebase", upstream, b],
        None => vec!["rebase", upstream],
    };
    match git_cmd_env(&root, &args, None) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git rebase failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git <kind> --abort`, answering a [`super::HostCtl::GitOpAbort`] (the conflict
/// banner's Abort button). `kind` is checked against [`valid_op_kind`]'s strict
/// allowlist BEFORE ever being interpolated as the git SUBCOMMAND itself.
pub(super) fn git_op_abort(kind: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "abort";
    if !valid_op_kind(kind) {
        return op_err(OP, "invalid operation kind");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd_env(&root, &[kind, "--abort"], None) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git abort failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git <kind> --continue`, answering a [`super::HostCtl::GitOpContinue`] (the conflict
/// banner's Continue button). `kind` is checked against [`valid_op_kind`]'s strict
/// allowlist, same as [`git_op_abort`]. Injects `GIT_EDITOR=true` as [`git_cmd_env`]'s
/// extra env pair — ADDITIONAL to that function's own always-on `GIT_TERMINAL_PROMPT=0`
/// — so a `cherry-pick`/`revert`/`rebase --continue` that would otherwise open an
/// editor for a commit message never hangs waiting on one (this is a headless GUI
/// host, there is no editor to open; `true` as a command exits 0 immediately, so git
/// treats the message as accepted unedited).
pub(super) fn git_op_continue(kind: &str, session: Option<&str>) -> GitOpResult {
    const OP: &str = "continue";
    if !valid_op_kind(kind) {
        return op_err(OP, "invalid operation kind");
    }
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd_env(&root, &[kind, "--continue"], Some(("GIT_EDITOR", "true"))) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git continue failed")),
        None => op_err(OP, "failed to run git"),
    }
}

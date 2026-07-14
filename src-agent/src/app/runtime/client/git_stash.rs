//! Host-side git STASH ops (push/pop/list) for the GitKraken-style Source Control
//! toolbar's stash indicator — GK4a. Mirrors [`super::git_branch`]'s exact
//! host-relay pattern (the [`super::git`] `git_cmd` choke point, an error-flagged
//! result struct instead of a panic, `#[serde(rename_all = "camelCase")]` DTOs, a
//! `GitOpResult { ok, op, error, message }` reply for a mutation) rather than
//! duplicating any of its plumbing: reuses [`super::git::repo_root_for`]/
//! `git_failure`/`git_cmd`/`GitOpResult`. Host-local only — never the daemon,
//! never `git_cred.rs`/`git_operator.rs`.
//!
//! Every op here takes no user-supplied ref/path, so there's nothing to validate
//! beyond resolving the repo root — unlike [`super::git_branch`]/
//! [`super::git_destructive`], no `valid_commit_ref`/`valid_branch_name` gate
//! applies.

use super::git::{git_cmd, git_failure, repo_root_for, GitOpResult};

fn op_ok(op: &str) -> GitOpResult {
    GitOpResult { ok: true, op: op.to_string(), error: None, message: None }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult { ok: false, op: op.to_string(), error: Some(error.into()), message: None }
}

/// `git stash push` — stashes tracked + staged changes (untracked files are left
/// alone, matching plain `git stash`'s own default; no `-u`), answering a
/// [`super::HostCtl::GitStash`]. A clean working tree isn't treated as a host-side
/// error case specially: git itself exits non-zero with "No local changes to
/// save", which [`git_failure`] surfaces verbatim as `error` — so the toolbar
/// toasts it rather than silently doing nothing.
pub(super) fn git_stash(session: Option<&str>) -> GitOpResult {
    const OP: &str = "stash";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd(&root, &["stash", "push"]) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git stash failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git stash pop`, answering a [`super::HostCtl::GitStashPop`]. May conflict
/// (leaving the stash entry in place and the tree half-applied) — that is NOT
/// specially handled here: the caller's follow-up
/// [`super::git::compute_git_status`] carries the authoritative `conflicted`
/// state (the EXISTING G5 conflict banner), same reasoning as
/// [`super::git_destructive::git_cherry_pick`]'s module doc.
pub(super) fn git_stash_pop(session: Option<&str>) -> GitOpResult {
    const OP: &str = "stashPop";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    match git_cmd(&root, &["stash", "pop"]) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, git_failure(&out, "git stash pop failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// One stash entry in a [`StashListResult`], parsed off one `git stash list`
/// line (`stash@{N}: <message>`).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StashEntry {
    pub index: u32,
    pub message: String,
}

/// The result of a host-side [`git_stash_list`], pushed to the GUI as a
/// `StashList` envelope for the Source Control toolbar's stash count/indicator.
/// `error` set means the workdir isn't a git repository (or `git stash list`
/// itself failed) — `entries` is then empty rather than the caller panicking.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StashListResult {
    pub entries: Vec<StashEntry>,
    pub error: Option<String>,
}

/// Compute a host-side STASH LIST (`git stash list`), answering a
/// [`super::HostCtl::GitStashList`]. Each output line is `stash@{N}: <message>` —
/// `N` parsed out of the `stash@{…}` marker, everything after it (minus a
/// leading `: `) kept verbatim as `message` (covers both git's default "WIP on
/// <branch>: <sha> <subject>" and a custom `git stash push -m <msg>` message). A
/// malformed line (shouldn't happen — this is git's own fixed format) is simply
/// skipped rather than aborting the whole list. ALWAYS returns a result — a
/// non-git workdir sets `error` rather than panicking, mirroring
/// [`super::git_branch::git_branch_list`].
pub(super) fn git_stash_list(session: Option<&str>) -> StashListResult {
    let empty = |error: Option<String>| StashListResult { entries: Vec::new(), error };

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()));
    };

    let stdout = match git_cmd(&root, &["stash", "list"]) {
        Some(out) if out.status.success() => out.stdout,
        Some(out) => return empty(Some(git_failure(&out, "git stash list failed"))),
        None => return empty(Some("failed to run git".to_string())),
    };

    let text = String::from_utf8_lossy(&stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("stash@{") else { continue };
        let Some(close) = rest.find('}') else { continue };
        let Ok(index) = rest[..close].parse::<u32>() else { continue };
        let message = rest[close + 1..].trim_start_matches(':').trim().to_string();
        entries.push(StashEntry { index, message });
    }

    StashListResult { entries, error: None }
}

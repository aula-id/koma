//! Off-thread bodies for the GIT / SSH-key-vault `HostCtl` handlers, shared by
//! both host-relay loops: [`super::host`]'s DETACHED `host_swapper` (no daemon
//! attached — pushes replies straight through the cloned `push` sink) and
//! [`super::push_loop`]'s ATTACHED fold loop (a session IS attached — replies
//! ride an `mpsc` channel back to the loop, which pushes them at its own
//! drain points so the 16ms cadence is never blocked). Split out of those two
//! files for size — PURE code motion, no behaviour change: same functions
//! called, same threads spawned, same channels/push order.
//!
//! Every op here shells out to `git`/`ssh-keygen` or touches the filesystem
//! (blocking), so each ALWAYS runs on a one-shot [`std::thread::spawn`]
//! worker — never inline on a host control loop — and NEVER touches the
//! daemon in either host state (this is host-only git/SSH-key machinery,
//! entirely separate from the model's own `git_cred.rs`/`git_operator.rs`).
//! A mutation (stage/unstage/discard/commit/fetch/pull/push/key-generate/
//! key-import/key-delete) additionally recomputes + replies with the fresh
//! GIT status (or key list), so the panel's lists refresh from authoritative
//! state right after.

use std::sync::mpsc::Sender;

use super::git::{
    compute_git_diff, compute_git_status, git_commit, git_discard, git_stage, git_unstage,
    GitDiffResult, GitOpResult, GitStatusResult,
};
use super::git_branch::{git_branch_list, git_checkout, git_create_branch, BranchListResult};
use super::git_graph::{
    compute_commit_detail, compute_commit_diff, compute_git_graph, CommitDetailResult,
    CommitDiffResult, GitGraphResult,
};
use super::git_remote::{git_fetch, git_pull, git_push, set_current_key};
use super::keys::{
    delete_key, generate_key, import_key, list_keys, reveal_key, KeyInfo, KeyOpResult,
    KeyRevealResult,
};
use super::push_proto::{
    push_branch_list, push_commit_detail, push_commit_diff, push_git_diff, push_git_graph,
    push_git_op, push_git_status, push_key_list, push_key_op, push_key_reveal,
};

// ─── DETACHED (host_swapper): push the reply straight through the cloned sink ───

/// `HostCtl::GitStatus` while detached.
pub(super) fn spawn_git_status(push: impl Fn(String) + Send + 'static, cur: Option<String>) {
    std::thread::spawn(move || {
        let result = compute_git_status(cur.as_deref());
        push_git_status(&push, result);
    });
}

/// `HostCtl::GitDiff` while detached.
pub(super) fn spawn_git_diff(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    path: String,
    staged: bool,
) {
    std::thread::spawn(move || {
        let result = compute_git_diff(&path, staged, cur.as_deref());
        push_git_diff(&push, result);
    });
}

/// `HostCtl::GitStage` while detached.
pub(super) fn spawn_git_stage(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    paths: Vec<String>,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_stage(&paths, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitUnstage` while detached.
pub(super) fn spawn_git_unstage(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    paths: Vec<String>,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_unstage(&paths, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitDiscard` while detached.
pub(super) fn spawn_git_discard(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    paths: Vec<String>,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_discard(&paths, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitCommit` while detached.
pub(super) fn spawn_git_commit(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    message: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_commit(&message, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitGraph` while detached.
pub(super) fn spawn_git_graph(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    limit: u32,
    skip: u32,
) {
    std::thread::spawn(move || {
        let result = compute_git_graph(limit, skip, cur.as_deref());
        push_git_graph(&push, result);
    });
}

/// `HostCtl::GitCommitDetail` while detached.
pub(super) fn spawn_commit_detail(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    sha: String,
) {
    std::thread::spawn(move || {
        let result = compute_commit_detail(&sha, cur.as_deref());
        push_commit_detail(&push, result);
    });
}

/// `HostCtl::GitCommitDiff` while detached.
pub(super) fn spawn_commit_diff(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    sha: String,
    path: String,
) {
    std::thread::spawn(move || {
        let result = compute_commit_diff(&sha, &path, cur.as_deref());
        push_commit_diff(&push, result);
    });
}

/// `HostCtl::SetGitKey` while detached.
pub(super) fn spawn_set_git_key(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    name: Option<String>,
) {
    std::thread::spawn(move || {
        set_current_key(cur.as_deref(), name);
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitFetch` while detached.
pub(super) fn spawn_git_fetch(push: impl Fn(String) + Send + 'static, cur: Option<String>) {
    std::thread::spawn(move || {
        push_git_op(&push, git_fetch(cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitPull` while detached.
pub(super) fn spawn_git_pull(push: impl Fn(String) + Send + 'static, cur: Option<String>) {
    std::thread::spawn(move || {
        push_git_op(&push, git_pull(cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitPush` while detached.
pub(super) fn spawn_git_push(push: impl Fn(String) + Send + 'static, cur: Option<String>) {
    std::thread::spawn(move || {
        push_git_op(&push, git_push(cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitBranchList` while detached.
pub(super) fn spawn_git_branch_list(push: impl Fn(String) + Send + 'static, cur: Option<String>) {
    std::thread::spawn(move || {
        let result = git_branch_list(cur.as_deref());
        push_branch_list(&push, result);
    });
}

/// `HostCtl::GitCheckout` while detached.
pub(super) fn spawn_git_checkout(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    ref_name: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_checkout(&ref_name, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitCreateBranch` while detached.
pub(super) fn spawn_git_create_branch(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    name: String,
    start: Option<String>,
    checkout: bool,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_create_branch(&name, start.as_deref(), checkout, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::KeyList` while detached.
pub(super) fn spawn_key_list(push: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        push_key_list(&push, list_keys());
    });
}

/// `HostCtl::KeyGenerate` while detached.
pub(super) fn spawn_key_generate(push: impl Fn(String) + Send + 'static, name: String, comment: String) {
    std::thread::spawn(move || {
        push_key_op(&push, generate_key(&name, &comment));
        push_key_list(&push, list_keys());
    });
}

/// `HostCtl::KeyImport` while detached.
pub(super) fn spawn_key_import(
    push: impl Fn(String) + Send + 'static,
    name: String,
    private_key: String,
) {
    std::thread::spawn(move || {
        push_key_op(&push, import_key(&name, &private_key));
        push_key_list(&push, list_keys());
    });
}

/// `HostCtl::KeyDelete` while detached.
pub(super) fn spawn_key_delete(push: impl Fn(String) + Send + 'static, name: String) {
    std::thread::spawn(move || {
        push_key_op(&push, delete_key(&name));
        push_key_list(&push, list_keys());
    });
}

/// `HostCtl::KeyReveal` while detached.
pub(super) fn spawn_key_reveal(push: impl Fn(String) + Send + 'static, name: String, private: bool) {
    std::thread::spawn(move || {
        push_key_reveal(&push, reveal_key(&name, private));
    });
}

// ─── ATTACHED (push_loop): reply over an mpsc channel, drained by the fold loop ───

/// `HostCtl::GitStatus` while attached.
pub(super) fn spawn_git_status_attached(tx: Sender<GitStatusResult>, cur: Option<String>) {
    std::thread::spawn(move || {
        let result = compute_git_status(cur.as_deref());
        let _ = tx.send(result);
    });
}

/// `HostCtl::GitDiff` while attached.
pub(super) fn spawn_git_diff_attached(
    tx: Sender<GitDiffResult>,
    cur: Option<String>,
    path: String,
    staged: bool,
) {
    std::thread::spawn(move || {
        let result = compute_git_diff(&path, staged, cur.as_deref());
        let _ = tx.send(result);
    });
}

/// `HostCtl::GitGraph` while attached.
pub(super) fn spawn_git_graph_attached(
    tx: Sender<GitGraphResult>,
    cur: Option<String>,
    limit: u32,
    skip: u32,
) {
    std::thread::spawn(move || {
        let result = compute_git_graph(limit, skip, cur.as_deref());
        let _ = tx.send(result);
    });
}

/// `HostCtl::GitCommitDetail` while attached.
pub(super) fn spawn_commit_detail_attached(
    tx: Sender<CommitDetailResult>,
    cur: Option<String>,
    sha: String,
) {
    std::thread::spawn(move || {
        let result = compute_commit_detail(&sha, cur.as_deref());
        let _ = tx.send(result);
    });
}

/// `HostCtl::GitCommitDiff` while attached.
pub(super) fn spawn_commit_diff_attached(
    tx: Sender<CommitDiffResult>,
    cur: Option<String>,
    sha: String,
    path: String,
) {
    std::thread::spawn(move || {
        let result = compute_commit_diff(&sha, &path, cur.as_deref());
        let _ = tx.send(result);
    });
}

/// `HostCtl::GitStage` while attached: the `GitOp` reply rides `op_tx`, THEN a
/// follow-up refreshed status rides the EXISTING `status_tx` (drained by the
/// same point the loop drains a plain `GitStatus` reply).
pub(super) fn spawn_git_stage_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    paths: Vec<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_stage(&paths, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitUnstage` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_unstage_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    paths: Vec<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_unstage(&paths, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitDiscard` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_discard_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    paths: Vec<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_discard(&paths, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitCommit` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_commit_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    message: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_commit(&message, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::SetGitKey` while attached: no `GitOp` reply of its own — only a
/// follow-up refreshed status over `status_tx`.
pub(super) fn spawn_set_git_key_attached(
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    name: Option<String>,
) {
    std::thread::spawn(move || {
        set_current_key(cur.as_deref(), name);
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitFetch` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_fetch_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_fetch(cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitPull` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_pull_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_pull(cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitPush` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_push_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_push(cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitBranchList` while attached.
pub(super) fn spawn_git_branch_list_attached(tx: Sender<BranchListResult>, cur: Option<String>) {
    std::thread::spawn(move || {
        let result = git_branch_list(cur.as_deref());
        let _ = tx.send(result);
    });
}

/// `HostCtl::GitCheckout` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_checkout_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    ref_name: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_checkout(&ref_name, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitCreateBranch` while attached. Same reply pattern as
/// [`spawn_git_stage_attached`].
pub(super) fn spawn_git_create_branch_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    name: String,
    start: Option<String>,
    checkout: bool,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_create_branch(&name, start.as_deref(), checkout, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::KeyList` while attached.
pub(super) fn spawn_key_list_attached(tx: Sender<Vec<KeyInfo>>) {
    std::thread::spawn(move || {
        let _ = tx.send(list_keys());
    });
}

/// `HostCtl::KeyGenerate` while attached: the `KeyOp` reply rides `op_tx`,
/// THEN a follow-up refreshed list rides the EXISTING `list_tx` (drained by
/// the same point the loop drains a plain `KeyList` reply).
pub(super) fn spawn_key_generate_attached(
    op_tx: Sender<KeyOpResult>,
    list_tx: Sender<Vec<KeyInfo>>,
    name: String,
    comment: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(generate_key(&name, &comment));
        let _ = list_tx.send(list_keys());
    });
}

/// `HostCtl::KeyImport` while attached. Same reply pattern as
/// [`spawn_key_generate_attached`].
pub(super) fn spawn_key_import_attached(
    op_tx: Sender<KeyOpResult>,
    list_tx: Sender<Vec<KeyInfo>>,
    name: String,
    private_key: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(import_key(&name, &private_key));
        let _ = list_tx.send(list_keys());
    });
}

/// `HostCtl::KeyDelete` while attached. Same reply pattern as
/// [`spawn_key_generate_attached`].
pub(super) fn spawn_key_delete_attached(
    op_tx: Sender<KeyOpResult>,
    list_tx: Sender<Vec<KeyInfo>>,
    name: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(delete_key(&name));
        let _ = list_tx.send(list_keys());
    });
}

/// `HostCtl::KeyReveal` while attached.
pub(super) fn spawn_key_reveal_attached(tx: Sender<KeyRevealResult>, name: String, private: bool) {
    std::thread::spawn(move || {
        let _ = tx.send(reveal_key(&name, private));
    });
}

//! Off-thread bodies for the G5b DESTRUCTIVE/INTERACTIVE git `HostCtl` handlers
//! (cherry-pick/revert/reset/merge/rebase/abort/continue) — split out of
//! [`super::git_host`] for file size (that file was approaching the 700-line
//! ceiling and GK4a's stash spawn flavors needed the headroom). PURE code
//! motion, no behaviour change: same functions, same threads spawned, same
//! channels/push order — [`super::git_host`] re-exports every name below via a
//! `pub(super) use` so every existing call site (`git_host::spawn_git_cherry_pick`
//! and friends, in `super::host`/`super::push_loop`) keeps resolving unchanged.
//!
//! Shared by both host-relay loops: [`super::host`]'s DETACHED `host_swapper` (no
//! daemon attached — pushes replies straight through the cloned `push` sink) and
//! [`super::push_loop`]'s ATTACHED fold loop (a session IS attached — replies ride
//! an `mpsc` channel back to the loop). Every op here shells out to `git`
//! (blocking), so each ALWAYS runs on a one-shot [`std::thread::spawn`] worker —
//! never inline on a host control loop.

use std::sync::mpsc::Sender;

use super::git::{compute_git_status, GitOpResult, GitStatusResult};
use super::git_destructive::{
    git_cherry_pick, git_merge, git_op_abort, git_op_continue, git_rebase, git_reset, git_revert,
};
use super::push_proto_git::{push_git_op, push_git_status};

// ─── DETACHED (host_swapper): push the reply straight through the cloned sink ───

/// `HostCtl::GitCherryPick` while detached.
pub(super) fn spawn_git_cherry_pick(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    sha: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_cherry_pick(&sha, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitRevert` while detached.
pub(super) fn spawn_git_revert(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    sha: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_revert(&sha, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitReset` while detached.
pub(super) fn spawn_git_reset(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    sha: String,
    mode: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_reset(&sha, &mode, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitMerge` while detached.
pub(super) fn spawn_git_merge(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    ref_name: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_merge(&ref_name, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitRebase` while detached.
pub(super) fn spawn_git_rebase(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    upstream: String,
    branch: Option<String>,
) {
    std::thread::spawn(move || {
        push_git_op(
            &push,
            git_rebase(&upstream, branch.as_deref(), cur.as_deref()),
        );
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitOpAbort` while detached.
pub(super) fn spawn_git_op_abort(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    kind: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_op_abort(&kind, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitOpContinue` while detached.
pub(super) fn spawn_git_op_continue(
    push: impl Fn(String) + Send + 'static,
    cur: Option<String>,
    kind: String,
) {
    std::thread::spawn(move || {
        push_git_op(&push, git_op_continue(&kind, cur.as_deref()));
        push_git_status(&push, compute_git_status(cur.as_deref()));
    });
}

// ─── ATTACHED (push_loop): reply over an mpsc channel, drained by the fold loop ───

/// `HostCtl::GitCherryPick` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_cherry_pick_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    sha: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_cherry_pick(&sha, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitRevert` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_revert_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    sha: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_revert(&sha, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitReset` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_reset_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    sha: String,
    mode: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_reset(&sha, &mode, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitMerge` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_merge_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    ref_name: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_merge(&ref_name, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitRebase` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_rebase_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    upstream: String,
    branch: Option<String>,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_rebase(&upstream, branch.as_deref(), cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitOpAbort` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_op_abort_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    kind: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_op_abort(&kind, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

/// `HostCtl::GitOpContinue` while attached. Same reply pattern as
/// `git_host::spawn_git_stage_attached`.
pub(super) fn spawn_git_op_continue_attached(
    op_tx: Sender<GitOpResult>,
    status_tx: Sender<GitStatusResult>,
    cur: Option<String>,
    kind: String,
) {
    std::thread::spawn(move || {
        let _ = op_tx.send(git_op_continue(&kind, cur.as_deref()));
        let _ = status_tx.send(compute_git_status(cur.as_deref()));
    });
}

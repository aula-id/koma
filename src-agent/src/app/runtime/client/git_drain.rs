//! GIT / SSH-key-vault off-thread reply drain for [`super::push_loop::push_loop`]'s
//! fold — split out of `push_loop.rs` for file size (pure code motion, no
//! behaviour change): same receivers drained, same push targets, same order.
//!
//! [`drain_git_replies`] is called once per fold frame, right after the
//! (b-quin) USAGE PANEL drain and before the (b-ter) attachment-marker mirror —
//! exactly where this logic ran inline before the split. The channels
//! themselves (the `Sender`/`Receiver` pairs, and the `HostCtl` control-arm
//! dispatch that spawns the off-thread workers feeding them) stay in
//! `push_loop.rs`; only the drain-and-push boilerplate moved here.

use std::sync::mpsc::Receiver;

use super::push_proto_git::{
    push_branch_list, push_commit_detail, push_commit_diff, push_git_diff, push_git_graph,
    push_git_op, push_git_status, push_key_list, push_key_op, push_key_reveal, push_stash_list,
};

/// Drain every completed GIT / SSH-key-vault off-thread reply and push each as its
/// own one-shot envelope, in the EXACT order `push_loop`'s fold previously drained
/// them inline (see the module doc).
#[allow(clippy::too_many_arguments)]
pub(super) fn drain_git_replies(
    push: &dyn Fn(String),
    git_status_rx: &Receiver<super::git::GitStatusResult>,
    git_diff_rx: &Receiver<super::git::GitDiffResult>,
    git_op_rx: &Receiver<super::git::GitOpResult>,
    branch_list_rx: &Receiver<super::git_branch::BranchListResult>,
    git_graph_rx: &Receiver<super::git_graph::GitGraphResult>,
    commit_detail_rx: &Receiver<super::git_graph::CommitDetailResult>,
    commit_diff_rx: &Receiver<super::git_graph::CommitDiffResult>,
    key_list_rx: &Receiver<Vec<super::keys::KeyInfo>>,
    key_reveal_rx: &Receiver<super::keys::KeyRevealResult>,
    key_op_rx: &Receiver<super::keys::KeyOpResult>,
    stash_list_rx: &Receiver<super::git_stash::StashListResult>,
) {
    // --- (b-sex) GIT panel: push any completed off-thread status fetch ---
    while let Ok(result) = git_status_rx.try_recv() {
        push_git_status(push, result);
    }

    // --- (b-sept) GIT panel: push any completed off-thread diff fetch ---
    while let Ok(result) = git_diff_rx.try_recv() {
        push_git_diff(push, result);
    }

    // --- (b-oct) GIT panel: push any completed off-thread mutation result ---
    // The worker also sent a follow-up status over `git_status_tx`, drained at
    // (b-sex) above — same frame or the next, whichever the loop happens to reach
    // first (harmless either order: both are one-shot, self-contained pushes).
    while let Ok(result) = git_op_rx.try_recv() {
        push_git_op(push, result);
    }

    // --- (b-octodec) branch list: push any completed off-thread fetch ---
    while let Ok(result) = branch_list_rx.try_recv() {
        push_branch_list(push, result);
    }

    // --- (b-quindec) commit-graph panel: push any completed off-thread graph fetch ---
    while let Ok(result) = git_graph_rx.try_recv() {
        push_git_graph(push, result);
    }

    // --- (b-sexdec) commit-graph panel: push any completed off-thread detail fetch ---
    while let Ok(result) = commit_detail_rx.try_recv() {
        push_commit_detail(push, result);
    }

    // --- (b-septdec) commit-graph panel: push any completed off-thread commit-diff fetch ---
    while let Ok(result) = commit_diff_rx.try_recv() {
        push_commit_diff(push, result);
    }

    // --- (b-undec) SSH key vault: push any completed off-thread list fetch ---
    while let Ok(keys) = key_list_rx.try_recv() {
        push_key_list(push, keys);
    }

    // --- (b-tredec) SSH key vault: push any completed off-thread reveal fetch ---
    while let Ok(result) = key_reveal_rx.try_recv() {
        push_key_reveal(push, result);
    }

    // --- (b-duodec) SSH key vault: push any completed off-thread mutation result ---
    // The worker also sent a follow-up list over `key_list_tx`, drained at
    // (b-undec) above — same frame or the next, whichever the loop happens to
    // reach first (harmless either order: both are one-shot, self-contained
    // pushes).
    while let Ok(result) = key_op_rx.try_recv() {
        push_key_op(push, result);
    }

    // --- (b-quattuordec) stash indicator: push any completed off-thread list fetch ---
    while let Ok(result) = stash_list_rx.try_recv() {
        push_stash_list(push, result);
    }
}

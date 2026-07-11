//! GIT-domain `push_*` one-shot emit helpers for the GUI push-envelope bridge —
//! split out of `push_proto.rs` for file size (pure code motion, no behaviour
//! change). These construct a [`super::push_proto::PushEnvelope`] variant and
//! call `super::render::emit`, exactly as they did when they lived in
//! `push_proto.rs`; only the module they live in changed. `PushEnvelope` itself
//! (and its non-git `push_*` siblings) stays in `push_proto.rs`.

use super::push_proto::PushEnvelope;

/// Emit a one-shot `GitStatus` envelope for the GUI Explore "GIT" panel, carrying a
/// host-computed [`super::git::GitStatusResult`] verbatim. Shared by the UN-ATTACHED
/// swapper fallback and the attached `push_loop`'s off-thread worker, since a
/// `GitStatus` is serviced entirely host-side regardless of attach state.
pub(super) fn push_git_status(push: &dyn Fn(String), result: super::git::GitStatusResult) {
    super::render::emit(push, &PushEnvelope::GitStatus(result));
}

/// Emit a one-shot `GitDiff` envelope for the GIT panel's Monaco diff tab, carrying a
/// host-computed [`super::git::GitDiffResult`] verbatim. Shared by the UN-ATTACHED
/// swapper fallback and the attached `push_loop`'s off-thread worker, since a
/// `GitDiff` is serviced entirely host-side regardless of attach state.
pub(super) fn push_git_diff(push: &dyn Fn(String), result: super::git::GitDiffResult) {
    super::render::emit(push, &PushEnvelope::GitDiff(result));
}

/// Emit a one-shot `GitOp` envelope for the GUI Source Control panel, carrying a
/// host-computed [`super::git::GitOpResult`] verbatim. Shared by the UN-ATTACHED
/// swapper fallback and the attached `push_loop`'s off-thread worker, since a git
/// mutation (stage/unstage/discard/commit) is serviced entirely host-side regardless
/// of attach state — mirrors `push_git_status`.
pub(super) fn push_git_op(push: &dyn Fn(String), result: super::git::GitOpResult) {
    super::render::emit(push, &PushEnvelope::GitOp(result));
}

/// Emit a one-shot `GitGraph` envelope for the commit-graph panel, carrying a
/// host-computed [`super::git_graph::GitGraphResult`] verbatim. Shared by the
/// UN-ATTACHED swapper fallback and the attached `push_loop`'s off-thread worker,
/// since a `GitGraph` is serviced entirely host-side regardless of attach state —
/// mirrors `push_git_status`.
pub(super) fn push_git_graph(push: &dyn Fn(String), result: super::git_graph::GitGraphResult) {
    super::render::emit(push, &PushEnvelope::GitGraph(result));
}

/// Emit a one-shot `CommitDetail` envelope for the commit-detail view, carrying a
/// host-computed [`super::git_graph::CommitDetailResult`] verbatim. Mirrors
/// `push_git_diff`.
pub(super) fn push_commit_detail(push: &dyn Fn(String), result: super::git_graph::CommitDetailResult) {
    super::render::emit(push, &PushEnvelope::CommitDetail(result));
}

/// Emit a one-shot `CommitDiff` envelope for a commit-history file diff, carrying a
/// host-computed [`super::git_graph::CommitDiffResult`] verbatim. Mirrors
/// `push_git_diff`.
pub(super) fn push_commit_diff(push: &dyn Fn(String), result: super::git_graph::CommitDiffResult) {
    super::render::emit(push, &PushEnvelope::CommitDiff(result));
}

/// Emit a one-shot `KeyList` envelope for the GUI Settings "SSH Keys" section,
/// carrying a host-computed [`super::keys::KeyInfo`] list verbatim. Shared by the
/// UN-ATTACHED swapper fallback and the attached `push_loop`'s off-thread worker,
/// since a `KeyList` is serviced entirely host-side regardless of attach state —
/// mirrors `push_git_status`. Also fired as the follow-up refresh after any
/// `KeyOp` mutation.
pub(super) fn push_key_list(push: &dyn Fn(String), keys: Vec<super::keys::KeyInfo>) {
    super::render::emit(push, &PushEnvelope::KeyList { keys });
}

/// Emit a one-shot `KeyReveal` envelope for the "Copy public key" / "Reveal
/// private key" actions, carrying a host-computed
/// [`super::keys::KeyRevealResult`] verbatim. Mirrors `push_git_diff`.
pub(super) fn push_key_reveal(push: &dyn Fn(String), result: super::keys::KeyRevealResult) {
    super::render::emit(push, &PushEnvelope::KeyReveal(result));
}

/// Emit a one-shot `KeyOp` envelope for the Settings "SSH Keys" section, carrying
/// a host-computed [`super::keys::KeyOpResult`] verbatim. Mirrors `push_git_op` —
/// ALWAYS immediately followed by a fresh `KeyList` push so the vault list
/// refreshes from authoritative state.
pub(super) fn push_key_op(push: &dyn Fn(String), result: super::keys::KeyOpResult) {
    super::render::emit(push, &PushEnvelope::KeyOp(result));
}

/// Emit a one-shot `BranchList` envelope, carrying a host-computed
/// [`super::git_branch::BranchListResult`] verbatim — mirrors `push_git_status`.
pub(super) fn push_branch_list(push: &dyn Fn(String), result: super::git_branch::BranchListResult) {
    super::render::emit(push, &PushEnvelope::BranchList(result));
}

/// Emit a one-shot `StashList` envelope for the Source Control toolbar's stash
/// count/indicator (GK4a), carrying a host-computed
/// [`super::git_stash::StashListResult`] verbatim. Shared by the UN-ATTACHED
/// swapper fallback and the attached `push_loop`'s off-thread worker, since a
/// `StashList` is serviced entirely host-side regardless of attach state —
/// mirrors `push_git_status`.
pub(super) fn push_stash_list(push: &dyn Fn(String), result: super::git_stash::StashListResult) {
    super::render::emit(push, &PushEnvelope::StashList(result));
}

/// Emit a one-shot `Activity` envelope for the bubble/activity chart (GK5a),
/// carrying a host-computed [`super::git_activity::ActivityResult`] verbatim.
/// Shared by the UN-ATTACHED swapper fallback and the attached `push_loop`'s
/// off-thread worker, since a `GitActivity` is serviced entirely host-side
/// regardless of attach state — mirrors `push_git_graph`.
pub(super) fn push_activity(push: &dyn Fn(String), result: super::git_activity::ActivityResult) {
    super::render::emit(push, &PushEnvelope::Activity(result));
}

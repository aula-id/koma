//! GIT panel + SSH-key-vault [`GuiReq`](super::proto::GuiReq) routing bodies, split out of
//! [`super::dispatch`] for file size — PURE code motion, no behaviour change. Every one of
//! these is ALWAYS routed to the host-relay control channel (`ctx.ctl`) — NEVER the daemon
//! — regardless of attach state: the host process already has direct git/fs access (see
//! `HostCtl::GitStatus` and friends for the full per-variant reasoning).

use std::sync::mpsc::Sender;

use crate::app::runtime::client::HostCtl;

/// `GuiReq::GitStatus`.
pub(super) fn git_status(ctl: &Sender<HostCtl>) {
    let _ = ctl.send(HostCtl::GitStatus);
}

/// `GuiReq::GitDiff`.
pub(super) fn git_diff(ctl: &Sender<HostCtl>, path: String, staged: bool) {
    let _ = ctl.send(HostCtl::GitDiff { path, staged });
}

/// `GuiReq::GitStage`.
pub(super) fn git_stage(ctl: &Sender<HostCtl>, paths: Vec<String>) {
    let _ = ctl.send(HostCtl::GitStage { paths });
}

/// `GuiReq::GitUnstage`.
pub(super) fn git_unstage(ctl: &Sender<HostCtl>, paths: Vec<String>) {
    let _ = ctl.send(HostCtl::GitUnstage { paths });
}

/// `GuiReq::GitDiscard`.
pub(super) fn git_discard(ctl: &Sender<HostCtl>, paths: Vec<String>) {
    let _ = ctl.send(HostCtl::GitDiscard { paths });
}

/// `GuiReq::GitCommit`.
pub(super) fn git_commit(ctl: &Sender<HostCtl>, message: String) {
    let _ = ctl.send(HostCtl::GitCommit { message });
}

/// `GuiReq::GitGraph`.
pub(super) fn git_graph(ctl: &Sender<HostCtl>, limit: u32, skip: u32) {
    let _ = ctl.send(HostCtl::GitGraph { limit, skip });
}

/// `GuiReq::GitCommitDetail`.
pub(super) fn git_commit_detail(ctl: &Sender<HostCtl>, sha: String) {
    let _ = ctl.send(HostCtl::GitCommitDetail { sha });
}

/// `GuiReq::GitCommitDiff`.
pub(super) fn git_commit_diff(ctl: &Sender<HostCtl>, sha: String, path: String) {
    let _ = ctl.send(HostCtl::GitCommitDiff { sha, path });
}

/// `GuiReq::SetGitKey`.
pub(super) fn set_git_key(ctl: &Sender<HostCtl>, name: Option<String>) {
    let _ = ctl.send(HostCtl::SetGitKey { name });
}

/// `GuiReq::GitFetch`.
pub(super) fn git_fetch(ctl: &Sender<HostCtl>) {
    let _ = ctl.send(HostCtl::GitFetch);
}

/// `GuiReq::GitPull`.
pub(super) fn git_pull(ctl: &Sender<HostCtl>) {
    let _ = ctl.send(HostCtl::GitPull);
}

/// `GuiReq::GitPush`.
pub(super) fn git_push(ctl: &Sender<HostCtl>) {
    let _ = ctl.send(HostCtl::GitPush);
}

/// `GuiReq::GitBranchList` (G4).
pub(super) fn git_branch_list(ctl: &Sender<HostCtl>) {
    let _ = ctl.send(HostCtl::GitBranchList);
}

/// `GuiReq::GitCheckout` (G4).
pub(super) fn git_checkout(ctl: &Sender<HostCtl>, ref_name: String) {
    let _ = ctl.send(HostCtl::GitCheckout { ref_name });
}

/// `GuiReq::GitCreateBranch` (G4).
pub(super) fn git_create_branch(
    ctl: &Sender<HostCtl>,
    name: String,
    start: Option<String>,
    checkout: bool,
) {
    let _ = ctl.send(HostCtl::GitCreateBranch { name, start, checkout });
}

/// `GuiReq::KeyList`.
pub(super) fn key_list(ctl: &Sender<HostCtl>) {
    let _ = ctl.send(HostCtl::KeyList);
}

/// `GuiReq::KeyGenerate`.
pub(super) fn key_generate(ctl: &Sender<HostCtl>, name: String, comment: String) {
    let _ = ctl.send(HostCtl::KeyGenerate { name, comment });
}

/// `GuiReq::KeyImport`.
pub(super) fn key_import(ctl: &Sender<HostCtl>, name: String, private_key: String) {
    let _ = ctl.send(HostCtl::KeyImport { name, private_key });
}

/// `GuiReq::KeyReveal`.
pub(super) fn key_reveal(ctl: &Sender<HostCtl>, name: String, private: bool) {
    let _ = ctl.send(HostCtl::KeyReveal { name, private });
}

/// `GuiReq::KeyDelete`.
pub(super) fn key_delete(ctl: &Sender<HostCtl>, name: String) {
    let _ = ctl.send(HostCtl::KeyDelete { name });
}

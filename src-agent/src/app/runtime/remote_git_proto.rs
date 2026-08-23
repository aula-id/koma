//! Private stdio protocol for the Source Control remote thin client (`koma remote-git`).
//!
//! Separate from session-daemon `ClientRequest`/`DaemonFrame`. Reuses the shared
//! length-prefix framing in [`crate::ipc::frame`] and field shapes that match
//! [`super::client::git*`] result types / `PushEnvelope` Git* bodies so the host
//! can forward replies without remapping.

use super::client::git::{GitDiffResult, GitOpResult, GitStatusResult};
use super::client::git_activity::ActivityResult;
use super::client::git_branch::BranchListResult;
use super::client::git_graph::{CommitDetailResult, CommitDiffResult, GitGraphResult};
use super::client::git_remote::GitPushMode;
use super::client::git_repos::RepoListResult;
use super::client::git_stash::StashListResult;

/// Request from the local host to a remote `koma remote-git` process.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum RemoteGitReq {
    /// Optional handshake: report version + bound session id.
    Hello,
    Status,
    Diff { path: String, staged: bool },
    Stage { paths: Vec<String> },
    Unstage { paths: Vec<String> },
    Discard { paths: Vec<String> },
    Commit { message: String },
    SetGitKey { name: Option<String> },
    Fetch,
    Pull,
    Push {
        mode: Option<GitPushMode>,
        root: Option<String>,
    },
    Stash,
    StashPop,
    StashList,
    BranchList { request_id: Option<u64> },
    Repos,
    SetActiveRepo { root: String },
    Checkout {
        ref_name: String,
        root: Option<String>,
    },
    CreateBranch {
        name: String,
        start: Option<String>,
        checkout: bool,
        root: Option<String>,
    },
    CherryPick { sha: String },
    Revert { sha: String },
    Reset { sha: String, mode: String },
    Merge { ref_name: String },
    Rebase {
        upstream: String,
        branch: Option<String>,
    },
    OpAbort { kind: String },
    OpContinue { kind: String },
    Graph { limit: u32, skip: u32 },
    CommitDetail { sha: String },
    CommitDiff { sha: String, path: String },
    Activity {
        path: Option<String>,
        limit: u32,
    },
}

/// Reply from remote-git. Bodies mirror PushEnvelope Git* fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum RemoteGitRep {
    Hello {
        version: String,
        session: Option<String>,
    },
    Status(GitStatusResult),
    Diff(GitDiffResult),
    /// Mutation result, optionally followed by a fresh status (host pushes both).
    /// Field is `result` (not `op`) so it doesn't collide with the externally-tagged `op` key.
    Op {
        result: GitOpResult,
        status: Option<GitStatusResult>,
    },
    Graph(GitGraphResult),
    CommitDetail(CommitDetailResult),
    CommitDiff(CommitDiffResult),
    BranchList(BranchListResult),
    Repos(RepoListResult),
    StashList(StashListResult),
    Activity(ActivityResult),
    /// Catch-all for protocol/parse errors.
    Error { error: String },
}

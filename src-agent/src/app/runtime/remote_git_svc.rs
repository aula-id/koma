//! `koma remote-git` — long-lived stdio thin client for Source Control Git* ops.
//!
//! Runs on the remote machine (spawned over SSH). Reads length-prefixed
//! [`super::remote_git_proto::RemoteGitReq`] frames from stdin, executes them via
//! the existing host-side git helpers against the remote session registry, writes
//! [`super::remote_git_proto::RemoteGitRep`] frames to stdout.
//!
//! Not a session-daemon bridge — owns git work directly (same model as remote-fs).

use anyhow::Result;

use crate::ipc::frame::{self, FrameReader};

use super::client::{
    git, git_activity, git_branch, git_destructive, git_graph, git_remote, git_repos, git_stash,
};
use super::remote_git_proto::{RemoteGitRep, RemoteGitReq};

/// Entry point for `koma remote-git`.
///
/// Optional `--session <id>` binds git ops to the remote session registry
/// (`session_workdirs_for` / `repo_root_for` on the machine running git).
pub fn run_remote_git(opts: crate::cli::Opts) -> Result<()> {
    // Ignore SIGPIPE — a broken-pipe write returns EPIPE instead of killing us.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let session = opts.session.clone();
    let session_ref = session.as_deref();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = FrameReader::new();
        loop {
            let bytes = match frame::read_frame_from(&mut stdin, &mut reader).await {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            let req: RemoteGitReq = match serde_json::from_slice(&bytes) {
                Ok(r) => r,
                Err(e) => {
                    let rep = RemoteGitRep::Error {
                        error: format!("invalid remote-git request: {e}"),
                    };
                    write_rep(&mut stdout, &rep).await?;
                    continue;
                }
            };
            let rep = handle_req(req, session_ref);
            write_rep(&mut stdout, &rep).await?;
        }
    })
}

async fn write_rep<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut W,
    rep: &RemoteGitRep,
) -> Result<()> {
    let out = serde_json::to_vec(rep)?;
    frame::write_frame_to(stdout, &out).await?;
    Ok(())
}

fn handle_req(req: RemoteGitReq, session: Option<&str>) -> RemoteGitRep {
    match req {
        RemoteGitReq::Hello => RemoteGitRep::Hello {
            version: env!("CARGO_PKG_VERSION").to_string(),
            session: session.map(str::to_string),
        },
        RemoteGitReq::Status => RemoteGitRep::Status(git::compute_git_status(session)),
        RemoteGitReq::Diff { path, staged } => {
            RemoteGitRep::Diff(git::compute_git_diff(&path, staged, session))
        }
        RemoteGitReq::Stage { paths } => op_with_status(git::git_stage(&paths, session), session),
        RemoteGitReq::Unstage { paths } => {
            op_with_status(git::git_unstage(&paths, session), session)
        }
        RemoteGitReq::Discard { paths } => {
            op_with_status(git::git_discard(&paths, session), session)
        }
        RemoteGitReq::Commit { message } => {
            op_with_status(git::git_commit(&message, session), session)
        }
        RemoteGitReq::SetGitKey { name } => {
            git_remote::set_current_key(session, name);
            RemoteGitRep::Status(git::compute_git_status(session))
        }
        RemoteGitReq::Fetch => op_with_status(git_remote::git_fetch(session), session),
        RemoteGitReq::Pull => op_with_status(git_remote::git_pull(session), session),
        RemoteGitReq::Push { mode, root } => {
            op_with_status(git_remote::git_push(mode, root.as_deref(), session), session)
        }
        RemoteGitReq::Stash => op_with_status(git_stash::git_stash(session), session),
        RemoteGitReq::StashPop => op_with_status(git_stash::git_stash_pop(session), session),
        RemoteGitReq::StashList => RemoteGitRep::StashList(git_stash::git_stash_list(session)),
        RemoteGitReq::BranchList { request_id } => {
            RemoteGitRep::BranchList(git_branch::git_branch_list(session, request_id))
        }
        RemoteGitReq::Repos => {
            let repos = git_repos::discover_repos(session);
            let active = git_repos::active_repo(session)
                .map(|p| p.to_string_lossy().into_owned());
            RemoteGitRep::Repos(git_repos::RepoListResult { repos, active })
        }
        RemoteGitReq::SetActiveRepo { root } => {
            let _ = git_repos::set_active_repo_checked(session, &root);
            RemoteGitRep::Status(git::compute_git_status(session))
        }
        RemoteGitReq::Checkout { ref_name, root } => op_with_status(
            git_branch::git_checkout(&ref_name, root.as_deref(), session),
            session,
        ),
        RemoteGitReq::CreateBranch {
            name,
            start,
            checkout,
            root,
        } => op_with_status(
            git_branch::git_create_branch(
                &name,
                start.as_deref(),
                checkout,
                root.as_deref(),
                session,
            ),
            session,
        ),
        RemoteGitReq::CherryPick { sha } => {
            op_with_status(git_destructive::git_cherry_pick(&sha, session), session)
        }
        RemoteGitReq::Revert { sha } => {
            op_with_status(git_destructive::git_revert(&sha, session), session)
        }
        RemoteGitReq::Reset { sha, mode } => {
            op_with_status(git_destructive::git_reset(&sha, &mode, session), session)
        }
        RemoteGitReq::Merge { ref_name } => {
            op_with_status(git_destructive::git_merge(&ref_name, session), session)
        }
        RemoteGitReq::Rebase { upstream, branch } => op_with_status(
            git_destructive::git_rebase(&upstream, branch.as_deref(), session),
            session,
        ),
        RemoteGitReq::OpAbort { kind } => {
            op_with_status(git_destructive::git_op_abort(&kind, session), session)
        }
        RemoteGitReq::OpContinue { kind } => {
            op_with_status(git_destructive::git_op_continue(&kind, session), session)
        }
        RemoteGitReq::Graph { limit, skip } => {
            RemoteGitRep::Graph(git_graph::compute_git_graph(limit, skip, session))
        }
        RemoteGitReq::CommitDetail { sha } => {
            RemoteGitRep::CommitDetail(git_graph::compute_commit_detail(&sha, session))
        }
        RemoteGitReq::CommitDiff { sha, path } => {
            RemoteGitRep::CommitDiff(git_graph::compute_commit_diff(&sha, &path, session))
        }
        RemoteGitReq::Activity { path, limit } => RemoteGitRep::Activity(
            git_activity::compute_git_activity(path.as_deref(), limit, session),
        ),
    }
}

fn op_with_status(result: git::GitOpResult, session: Option<&str>) -> RemoteGitRep {
    let status = git::compute_git_status(session);
    RemoteGitRep::Op {
        result,
        status: Some(status),
    }
}

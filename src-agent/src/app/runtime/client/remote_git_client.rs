//! Host-side client for a long-lived `koma remote-git` SSH child.
//!
//! Owns one SSH process per remote attach. A dedicated IO thread runs a small
//! tokio runtime over the child's stdin/stdout, speaking
//! [`crate::app::runtime::remote_git_proto`] frames. Callers (push_loop Git*
//! arms) block on a request channel and always get a reply so the GUI never
//! hangs.
//!
//! Teardown: [`RemoteGitClient::shutdown`] (or Drop) sends Shutdown, joins the
//! IO thread, and reaps the SSH child — same durability contract as remote-fs.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::app::runtime::remote_git_proto::{RemoteGitRep, RemoteGitReq};
use crate::ipc::frame::{self, FrameReader};
use crate::remote::ssh::{self, SshSession};
use crate::remote::RemoteTarget;

use super::push_proto_git::{
    push_activity, push_branch_list, push_commit_detail, push_commit_diff, push_git_diff,
    push_git_graph, push_git_op, push_git_status, push_repo_list, push_stash_list,
};
use super::remote_ctl::RemoteCtx;

/// How long a single remote-git round-trip may block the host worker.
/// Longer than remote-fs: fetch/pull/push can be slow over the network.
const REQ_TIMEOUT: Duration = Duration::from_secs(120);

enum ClientMsg {
    Req {
        req: RemoteGitReq,
        resp: Sender<RemoteGitRep>,
    },
    Shutdown,
}

/// Long-lived host handle for remote Source Control Git* ops.
pub(super) struct RemoteGitClient {
    tx: Sender<ClientMsg>,
    join: Option<JoinHandle<()>>,
}

impl RemoteGitClient {
    /// Spawn `ssh … koma remote-git [--session <id>]` and start the IO thread.
    ///
    /// Returns `Err` if SSH spawn fails. On success the child is owned by the
    /// IO thread until shutdown.
    pub fn start(
        handle: &tokio::runtime::Handle,
        ctx: &RemoteCtx,
        session_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        let auth = ctx.make_auth()?;
        let auth_ref = auth.as_ref();
        let mut argv: Vec<&str> = vec!["remote-git"];
        let session_owned = session_id.map(str::to_string);
        if let Some(ref s) = session_owned {
            argv.push("--session");
            argv.push(s.as_str());
        }
        let session = {
            let _rt = handle.enter();
            ssh::connect_command(&ctx.target, auth_ref, &ctx.koma_path, &argv)?
        };

        let (tx, rx) = mpsc::channel::<ClientMsg>();
        let join = std::thread::Builder::new()
            .name("koma-remote-git-io".into())
            .spawn(move || io_thread(session, rx))
            .map_err(|e| anyhow::anyhow!("failed to spawn remote-git io thread: {e}"))?;

        let client = Self {
            tx,
            join: Some(join),
        };
        // Handshake (best-effort) so a dead child fails fast.
        let _ = client.request(RemoteGitReq::Hello);
        Ok(client)
    }

    /// Round-trip one request. Always returns a `RemoteGitRep` (Error on timeout/IO).
    pub fn request(&self, req: RemoteGitReq) -> RemoteGitRep {
        let (resp_tx, resp_rx) = mpsc::channel();
        if self
            .tx
            .send(ClientMsg::Req {
                req,
                resp: resp_tx,
            })
            .is_err()
        {
            return RemoteGitRep::Error {
                error: "remote-git client stopped".to_string(),
            };
        }
        match resp_rx.recv_timeout(REQ_TIMEOUT) {
            Ok(rep) => rep,
            Err(_) => RemoteGitRep::Error {
                error: "remote-git request timed out".to_string(),
            },
        }
    }

    /// Map a Git* / SetActiveRepo / SetGitKey HostCtl to a remote-git request,
    /// await the reply, push the matching PushEnvelope(s). Always pushes so the
    /// webview never hangs. Key* vault ops stay host-local (not handled here).
    pub fn handle_git_ctl(&self, ctl: &super::HostCtl, push: &dyn Fn(String)) {
        let req = match hostctl_to_req(ctl) {
            Some(r) => r,
            None => return,
        };
        let rep = self.request(req);
        push_rep(push, rep);
    }

    /// Stop the IO thread and reap the SSH child.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(ClientMsg::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for RemoteGitClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn hostctl_to_req(ctl: &super::HostCtl) -> Option<RemoteGitReq> {
    match ctl {
        super::HostCtl::GitStatus => Some(RemoteGitReq::Status),
        super::HostCtl::GitDiff { path, staged } => Some(RemoteGitReq::Diff {
            path: path.clone(),
            staged: *staged,
        }),
        super::HostCtl::GitStage { paths } => Some(RemoteGitReq::Stage {
            paths: paths.clone(),
        }),
        super::HostCtl::GitUnstage { paths } => Some(RemoteGitReq::Unstage {
            paths: paths.clone(),
        }),
        super::HostCtl::GitDiscard { paths } => Some(RemoteGitReq::Discard {
            paths: paths.clone(),
        }),
        super::HostCtl::GitCommit { message } => Some(RemoteGitReq::Commit {
            message: message.clone(),
        }),
        super::HostCtl::SetGitKey { name } => Some(RemoteGitReq::SetGitKey {
            name: name.clone(),
        }),
        super::HostCtl::GitFetch => Some(RemoteGitReq::Fetch),
        super::HostCtl::GitPull => Some(RemoteGitReq::Pull),
        super::HostCtl::GitPush { mode, root } => Some(RemoteGitReq::Push {
            mode: *mode,
            root: root.clone(),
        }),
        super::HostCtl::GitStash => Some(RemoteGitReq::Stash),
        super::HostCtl::GitStashPop => Some(RemoteGitReq::StashPop),
        super::HostCtl::GitStashList => Some(RemoteGitReq::StashList),
        super::HostCtl::GitBranchList { request_id } => Some(RemoteGitReq::BranchList {
            request_id: *request_id,
        }),
        super::HostCtl::GitRepos => Some(RemoteGitReq::Repos),
        super::HostCtl::SetActiveRepo { root } => Some(RemoteGitReq::SetActiveRepo {
            root: root.clone(),
        }),
        super::HostCtl::GitCheckout { ref_name, root } => Some(RemoteGitReq::Checkout {
            ref_name: ref_name.clone(),
            root: root.clone(),
        }),
        super::HostCtl::GitCreateBranch {
            name,
            start,
            checkout,
            root,
        } => Some(RemoteGitReq::CreateBranch {
            name: name.clone(),
            start: start.clone(),
            checkout: *checkout,
            root: root.clone(),
        }),
        super::HostCtl::GitCherryPick { sha } => Some(RemoteGitReq::CherryPick {
            sha: sha.clone(),
        }),
        super::HostCtl::GitRevert { sha } => Some(RemoteGitReq::Revert { sha: sha.clone() }),
        super::HostCtl::GitReset { sha, mode } => Some(RemoteGitReq::Reset {
            sha: sha.clone(),
            mode: mode.clone(),
        }),
        super::HostCtl::GitMerge { ref_name } => Some(RemoteGitReq::Merge {
            ref_name: ref_name.clone(),
        }),
        super::HostCtl::GitRebase { upstream, branch } => Some(RemoteGitReq::Rebase {
            upstream: upstream.clone(),
            branch: branch.clone(),
        }),
        super::HostCtl::GitOpAbort { kind } => Some(RemoteGitReq::OpAbort {
            kind: kind.clone(),
        }),
        super::HostCtl::GitOpContinue { kind } => Some(RemoteGitReq::OpContinue {
            kind: kind.clone(),
        }),
        super::HostCtl::GitGraph { limit, skip } => Some(RemoteGitReq::Graph {
            limit: *limit,
            skip: *skip,
        }),
        super::HostCtl::GitCommitDetail { sha } => Some(RemoteGitReq::CommitDetail {
            sha: sha.clone(),
        }),
        super::HostCtl::GitCommitDiff { sha, path } => Some(RemoteGitReq::CommitDiff {
            sha: sha.clone(),
            path: path.clone(),
        }),
        super::HostCtl::GitActivity { path, limit } => Some(RemoteGitReq::Activity {
            path: path.clone(),
            limit: *limit,
        }),
        _ => None,
    }
}

fn push_rep(push: &dyn Fn(String), rep: RemoteGitRep) {
    match rep {
        RemoteGitRep::Status(r) => push_git_status(push, r),
        RemoteGitRep::Diff(r) => push_git_diff(push, r),
        RemoteGitRep::Op { result, status } => {
            push_git_op(push, result);
            if let Some(st) = status {
                push_git_status(push, st);
            }
        }
        RemoteGitRep::Graph(r) => push_git_graph(push, r),
        RemoteGitRep::CommitDetail(r) => push_commit_detail(push, r),
        RemoteGitRep::CommitDiff(r) => push_commit_diff(push, r),
        RemoteGitRep::BranchList(r) => push_branch_list(push, r),
        RemoteGitRep::Repos(r) => push_repo_list(push, r),
        RemoteGitRep::StashList(r) => push_stash_list(push, r),
        RemoteGitRep::Activity(r) => push_activity(push, r),
        RemoteGitRep::Error { error } => {
            // No op context — emit a GitStatus-shaped error so the panel surfaces it.
            push_git_status(
                push,
                super::git::GitStatusResult {
                    root: None,
                    branch: None,
                    detached: false,
                    ahead: None,
                    behind: None,
                    staged: Vec::new(),
                    unstaged: Vec::new(),
                    error: Some(error),
                    key_name: None,
                    in_progress: None,
                    conflicted: Vec::new(),
                    push_mode: None,
                },
            );
        }
        RemoteGitRep::Hello { .. } => {}
    }
}

fn io_thread(session: SshSession, rx: Receiver<ClientMsg>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return,
    };
    rt.block_on(async move {
        let SshSession {
            mut child,
            mut stdin,
            mut stdout,
        } = session;
        let mut reader = FrameReader::new();
        while let Ok(msg) = rx.recv() {
            match msg {
                ClientMsg::Shutdown => break,
                ClientMsg::Req { req, resp } => {
                    let rep = match roundtrip(&mut stdin, &mut stdout, &mut reader, &req).await {
                        Ok(r) => r,
                        Err(e) => RemoteGitRep::Error { error: e },
                    };
                    let _ = resp.send(rep);
                }
            }
        }
        // Best-effort child reap (2s then kill), same contract as chat bridge.
        crate::app::runtime::stdio_bridge::reap_bridge_child(&mut child).await;
    });
}

async fn roundtrip<W, R>(
    stdin: &mut W,
    stdout: &mut R,
    reader: &mut FrameReader,
    req: &RemoteGitReq,
) -> Result<RemoteGitRep, String>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    let bytes = serde_json::to_vec(req).map_err(|e| format!("encode: {e}"))?;
    frame::write_frame_to(stdin, &bytes)
        .await
        .map_err(|e| format!("write: {e}"))?;
    let payload = frame::read_frame_from(stdout, reader)
        .await
        .map_err(|e| format!("read: {e}"))?;
    serde_json::from_slice(&payload).map_err(|e| format!("decode: {e}"))
}

/// Convenience: rebuild auth is unused here (spawn-time only) — silence lint if
/// `RemoteTarget` is only needed for docs.
#[allow(dead_code)]
fn _target_ty(_: &RemoteTarget) {}

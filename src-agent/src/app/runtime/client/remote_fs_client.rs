//! Host-side client for a long-lived `koma remote-fs` SSH child.
//!
//! Owns one SSH process per remote attach. A dedicated IO thread runs a small
//! tokio runtime over the child's stdin/stdout, speaking
//! [`crate::app::runtime::remote_fs_proto`] frames. Callers (push_loop File*
//! arms) block on a request channel and always get a reply so the GUI never
//! hangs.
//!
//! Teardown: [`RemoteFsClient::shutdown`] (or Drop) sends Shutdown, joins the
//! IO thread, and reaps the SSH child — same durability contract as the chat
//! bridge (killing the thin client never deletes a session-daemon).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::app::runtime::remote_fs_proto::{RemoteFsRep, RemoteFsReq};
use crate::ipc::frame::{self, FrameReader};
use crate::remote::ssh::{self, SshSession};
use crate::remote::RemoteTarget;

use super::push_proto::PushEnvelope;
use super::remote_ctl::RemoteCtx;

/// How long a single remote-fs round-trip may block the host worker.
const REQ_TIMEOUT: Duration = Duration::from_secs(30);

enum ClientMsg {
    Req {
        req: RemoteFsReq,
        resp: Sender<RemoteFsRep>,
    },
    Shutdown,
}

/// Long-lived host handle for remote Coding panel File* ops.
pub(super) struct RemoteFsClient {
    tx: Sender<ClientMsg>,
    join: Option<JoinHandle<()>>,
    /// Cached remote workdirs (absolute paths on the remote machine).
    /// Updated from SettingsValues / Snapshot; used as SetRoots + sandbox list.
    roots: std::sync::Mutex<Vec<String>>,
}

impl RemoteFsClient {
    /// Spawn `ssh … koma remote-fs [--cwd <cwd>]` and start the IO thread.
    ///
    /// Returns `Err` if SSH spawn fails. On success the child is owned by the
    /// IO thread until shutdown.
    pub fn start(
        handle: &tokio::runtime::Handle,
        ctx: &RemoteCtx,
        cwd: Option<&str>,
    ) -> anyhow::Result<Self> {
        let auth = ctx.make_auth()?;
        let auth_ref = auth.as_ref();
        let mut argv: Vec<&str> = vec!["remote-fs"];
        let cwd_owned = cwd.map(str::to_string);
        if let Some(ref c) = cwd_owned {
            argv.push("--cwd");
            argv.push(c.as_str());
        }
        let session = {
            let _rt = handle.enter();
            ssh::connect_command(&ctx.target, auth_ref, &ctx.koma_path, &argv)?
        };

        let initial_roots: Vec<String> = cwd_owned.into_iter().collect();
        let (tx, rx) = mpsc::channel::<ClientMsg>();
        let join = std::thread::Builder::new()
            .name("koma-remote-fs-io".into())
            .spawn(move || io_thread(session, rx))
            .map_err(|e| anyhow::anyhow!("failed to spawn remote-fs io thread: {e}"))?;

        let client = Self {
            tx,
            join: Some(join),
            roots: std::sync::Mutex::new(initial_roots.clone()),
        };
        if !initial_roots.is_empty() {
            let _ = client.set_roots(initial_roots);
        }
        // Handshake (best-effort) so a dead child fails fast.
        let _ = client.request(RemoteFsReq::Hello);
        Ok(client)
    }

    /// Replace sandbox roots (absolute remote paths) and push SetRoots to the child.
    pub fn set_roots(&self, roots: Vec<String>) -> Result<(), String> {
        if let Ok(mut g) = self.roots.lock() {
            *g = roots.clone();
        }
        match self.request(RemoteFsReq::SetRoots { roots }) {
            RemoteFsRep::SetRoots { error: Some(e), .. } => Err(e),
            RemoteFsRep::SetRoots { error: None, .. } => Ok(()),
            RemoteFsRep::Error { error, .. } => Err(error),
            other => Err(format!("unexpected remote-fs SetRoots reply: {other:?}")),
        }
    }

    /// Cached roots (may be empty until SettingsValues arrives).
    pub fn roots(&self) -> Vec<String> {
        self.roots
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Cached roots as `PathBuf`s (for API parity with local `workdirs`).
    pub fn root_paths(&self) -> Vec<PathBuf> {
        self.roots().into_iter().map(PathBuf::from).collect()
    }

    /// Round-trip one request. Always returns a `RemoteFsRep` (Error on timeout/IO).
    pub fn request(&self, req: RemoteFsReq) -> RemoteFsRep {
        let (resp_tx, resp_rx) = mpsc::channel();
        if self
            .tx
            .send(ClientMsg::Req {
                req: req.clone(),
                resp: resp_tx,
            })
            .is_err()
        {
            return RemoteFsRep::Error {
                error: "remote-fs client stopped".to_string(),
                request_id: req_id_of(&req),
            };
        }
        match resp_rx.recv_timeout(REQ_TIMEOUT) {
            Ok(rep) => rep,
            Err(_) => RemoteFsRep::Error {
                error: "remote-fs request timed out".to_string(),
                request_id: req_id_of(&req),
            },
        }
    }

    /// Map a File* HostCtl to a remote-fs request, await the reply, push the
    /// matching PushEnvelope. Always pushes so the webview never hangs.
    pub fn handle_file_ctl(&self, ctl: &super::HostCtl, push: &dyn Fn(String)) {
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

impl Drop for RemoteFsClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn req_id_of(req: &RemoteFsReq) -> Option<String> {
    match req {
        RemoteFsReq::Tree { request_id, .. }
        | RemoteFsReq::Read { request_id, .. }
        | RemoteFsReq::Save { request_id, .. }
        | RemoteFsReq::Create { request_id, .. }
        | RemoteFsReq::Rename { request_id, .. }
        | RemoteFsReq::Delete { request_id, .. }
        | RemoteFsReq::WriteBytes { request_id, .. }
        | RemoteFsReq::DownloadBytes { request_id, .. } => Some(request_id.clone()),
        _ => None,
    }
}

fn hostctl_to_req(ctl: &super::HostCtl) -> Option<RemoteFsReq> {
    match ctl {
        super::HostCtl::FileTree {
            root,
            path,
            request_id,
        } => Some(RemoteFsReq::Tree {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileRead {
            root,
            path,
            request_id,
        } => Some(RemoteFsReq::Read {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileSave {
            root,
            path,
            content,
            expected_fingerprint,
            request_id,
        } => Some(RemoteFsReq::Save {
            root: root.clone(),
            path: path.clone(),
            content: content.clone(),
            expected_fingerprint: expected_fingerprint.clone(),
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileCreate {
            root,
            path,
            kind,
            request_id,
        } => Some(RemoteFsReq::Create {
            root: root.clone(),
            path: path.clone(),
            kind: kind.clone(),
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileRename {
            root,
            old_path,
            new_path,
            request_id,
        } => Some(RemoteFsReq::Rename {
            root: root.clone(),
            old_path: old_path.clone(),
            new_path: new_path.clone(),
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileDelete {
            root,
            path,
            request_id,
        } => Some(RemoteFsReq::Delete {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileWriteBytes {
            root,
            path,
            bytes_b64,
            overwrite,
            request_id,
        } => Some(RemoteFsReq::WriteBytes {
            root: root.clone(),
            path: path.clone(),
            bytes_b64: bytes_b64.clone(),
            overwrite: *overwrite,
            request_id: request_id.clone(),
        }),
        super::HostCtl::FileDownloadBytes {
            root,
            path,
            request_id,
        } => Some(RemoteFsReq::DownloadBytes {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
        }),
        _ => None,
    }
}

fn push_rep(push: &dyn Fn(String), rep: RemoteFsRep) {
    let env = match rep {
        RemoteFsRep::Tree(r) => PushEnvelope::FileTree {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            entries: r.entries,
            error: r.error,
        },
        RemoteFsRep::Read(r) => PushEnvelope::FileRead {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            content: r.content,
            fingerprint: r.fingerprint,
            binary: r.binary,
            too_large: r.too_large,
            error: r.error,
        },
        RemoteFsRep::Save(r) => PushEnvelope::FileSave {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            fingerprint: r.fingerprint,
            error: r.error,
        },
        RemoteFsRep::Create(r) => PushEnvelope::FileCreate {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            error: r.error,
        },
        RemoteFsRep::Rename(r) => PushEnvelope::FileRename {
            root: r.root,
            old_path: r.old_path,
            new_path: r.new_path,
            request_id: r.request_id,
            error: r.error,
        },
        RemoteFsRep::Delete(r) => PushEnvelope::FileDelete {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            error: r.error,
        },
        RemoteFsRep::WriteBytes(r) => PushEnvelope::FileWriteBytes {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            error: r.error,
        },
        RemoteFsRep::DownloadBytes(r) => PushEnvelope::FileDownloadBytes {
            root: r.root,
            path: r.path,
            request_id: r.request_id,
            bytes_b64: r.bytes_b64,
            size: r.size,
            too_large: r.too_large,
            error: r.error,
        },
        RemoteFsRep::Error { error, request_id } => {
            // No op context — emit a FileTree-shaped error if we have a request id,
            // otherwise drop (Hello/SetRoots failures are handled by callers).
            if let Some(rid) = request_id {
                PushEnvelope::FileTree {
                    root: String::new(),
                    path: String::new(),
                    request_id: rid,
                    entries: Vec::new(),
                    error: Some(error),
                }
            } else {
                return;
            }
        }
        RemoteFsRep::Hello { .. } | RemoteFsRep::SetRoots { .. } => return,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        push(json);
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
                        Err(e) => RemoteFsRep::Error {
                            error: e,
                            request_id: req_id_of(&req),
                        },
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
    req: &RemoteFsReq,
) -> Result<RemoteFsRep, String>
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

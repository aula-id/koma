//! Host-side client for a long-lived `koma remote-linker` SSH child.
//!
//! Owns one SSH process per remote attach. A dedicated IO thread runs a small
//! tokio runtime over the child's stdin/stdout, speaking
//! [`crate::app::runtime::remote_linker_proto`] frames. Callers (push_loop
//! ImportGraph* arms) block on a request channel and always get a reply so the
//! GUI never hangs.
//!
//! Teardown: [`RemoteLinkerClient::shutdown`] (or Drop) sends Shutdown, joins the
//! IO thread, and reaps the SSH child — same durability contract as remote-fs.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::app::runtime::remote_linker_proto::{RemoteLinkerRep, RemoteLinkerReq};
use crate::ipc::frame::{self, FrameReader};
use crate::remote::ssh::{self, SshSession};

use super::import_graph::{self, ImportGraphResult};
use super::push_proto::{ImportGraphImpactResult, PushEnvelope};
use super::remote_ctl::RemoteCtx;

/// How long a single remote-linker round-trip may block the host worker.
/// Reindex can take longer than File* ops (scan + poll).
const REQ_TIMEOUT: Duration = Duration::from_secs(60);

enum ClientMsg {
    Req {
        req: RemoteLinkerReq,
        resp: Sender<RemoteLinkerRep>,
    },
    Shutdown,
}

/// Long-lived host handle for remote Import Graph panel ops.
pub(super) struct RemoteLinkerClient {
    tx: Sender<ClientMsg>,
    join: Option<JoinHandle<()>>,
    /// Cached remote workdirs (absolute paths on the remote machine).
    /// Updated from SettingsValues / Snapshot; used as SetRoots.
    roots: std::sync::Mutex<Vec<String>>,
}

impl RemoteLinkerClient {
    /// Spawn `ssh … koma remote-linker [--cwd <cwd>]` and start the IO thread.
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
        let mut argv: Vec<&str> = vec!["remote-linker"];
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
            .name("koma-remote-linker-io".into())
            .spawn(move || io_thread(session, rx))
            .map_err(|e| anyhow::anyhow!("failed to spawn remote-linker io thread: {e}"))?;

        let client = Self {
            tx,
            join: Some(join),
            roots: std::sync::Mutex::new(initial_roots.clone()),
        };
        if !initial_roots.is_empty() {
            let _ = client.set_roots(initial_roots);
        }
        // Handshake (best-effort) so a dead child fails fast.
        let _ = client.request(RemoteLinkerReq::Hello);
        Ok(client)
    }

    /// Replace workdir roots (absolute remote paths) and push SetRoots to the child.
    pub fn set_roots(&self, roots: Vec<String>) -> Result<(), String> {
        if let Ok(mut g) = self.roots.lock() {
            *g = roots.clone();
        }
        match self.request(RemoteLinkerReq::SetRoots { roots }) {
            RemoteLinkerRep::SetRoots { error: Some(e), .. } => Err(e),
            RemoteLinkerRep::SetRoots { error: None, .. } => Ok(()),
            RemoteLinkerRep::Error { error, .. } => Err(error),
            other => Err(format!(
                "unexpected remote-linker SetRoots reply: {}",
                rep_label(&other)
            )),
        }
    }

    /// Cached roots (may be empty until SettingsValues arrives).
    #[allow(dead_code)]
    pub fn roots(&self) -> Vec<String> {
        self.roots
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Cached roots as `PathBuf`s (for API parity with local `workdirs`).
    #[allow(dead_code)]
    pub fn root_paths(&self) -> Vec<PathBuf> {
        self.roots().into_iter().map(PathBuf::from).collect()
    }

    /// Round-trip one request. Always returns a `RemoteLinkerRep` (Error on timeout/IO).
    pub fn request(&self, req: RemoteLinkerReq) -> RemoteLinkerRep {
        let (resp_tx, resp_rx) = mpsc::channel();
        if self
            .tx
            .send(ClientMsg::Req {
                req: req.clone(),
                resp: resp_tx,
            })
            .is_err()
        {
            return RemoteLinkerRep::Error {
                error: "remote-linker client stopped".to_string(),
                request_id: req_id_of(&req),
            };
        }
        match resp_rx.recv_timeout(REQ_TIMEOUT) {
            Ok(rep) => rep,
            Err(_) => RemoteLinkerRep::Error {
                error: "remote-linker request timed out".to_string(),
                request_id: req_id_of(&req),
            },
        }
    }

    /// Map an ImportGraph* HostCtl to a remote-linker request, await the reply,
    /// push the matching PushEnvelope. Always pushes so the webview never hangs.
    ///
    /// `current_session` is used for Reindex (HostCtl has no session_id field) and
    /// as a fallback when Graph/Impact omit it.
    pub fn handle_import_ctl(
        &self,
        ctl: &super::HostCtl,
        push: &dyn Fn(String),
        current_session: Option<&str>,
    ) {
        let req = match hostctl_to_req(ctl, current_session) {
            Some(r) => r,
            None => return,
        };
        let rep = self.request(req);
        push_rep(push, rep, ctl);
    }

    /// Stop the IO thread and reap the SSH child.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(ClientMsg::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for RemoteLinkerClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn req_id_of(req: &RemoteLinkerReq) -> Option<String> {
    match req {
        RemoteLinkerReq::Graph { request_id, .. } => request_id.clone(),
        RemoteLinkerReq::Impact { request_id, .. } => Some(request_id.clone()),
        RemoteLinkerReq::Reindex { request_id, .. } => request_id.clone(),
        _ => None,
    }
}

fn rep_label(rep: &RemoteLinkerRep) -> &'static str {
    match rep {
        RemoteLinkerRep::Hello { .. } => "Hello",
        RemoteLinkerRep::SetRoots { .. } => "SetRoots",
        RemoteLinkerRep::Graph(_) => "Graph",
        RemoteLinkerRep::Impact(_) => "Impact",
        RemoteLinkerRep::Error { .. } => "Error",
    }
}

fn hostctl_to_req(ctl: &super::HostCtl, current_session: Option<&str>) -> Option<RemoteLinkerReq> {
    match ctl {
        super::HostCtl::ImportGraph {
            path,
            depth,
            direction,
            filter_roots,
            filter_languages,
            session_id,
            request_id,
        } => Some(RemoteLinkerReq::Graph {
            path: path.clone(),
            depth: *depth,
            direction: *direction,
            filter_roots: filter_roots.clone(),
            filter_languages: filter_languages.clone(),
            session_id: session_id
                .clone()
                .or_else(|| current_session.map(str::to_string)),
            request_id: request_id.clone(),
        }),
        super::HostCtl::ImportGraphImpact {
            path,
            depth,
            request_id,
            session_id,
        } => Some(RemoteLinkerReq::Impact {
            path: path.clone(),
            depth: *depth,
            request_id: request_id.clone(),
            session_id: session_id
                .clone()
                .or_else(|| current_session.map(str::to_string)),
        }),
        super::HostCtl::ImportGraphReindex { request_id } => Some(RemoteLinkerReq::Reindex {
            session_id: current_session.map(str::to_string),
            request_id: request_id.clone(),
            filter_roots: None,
            filter_languages: None,
        }),
        _ => None,
    }
}

fn push_rep(push: &dyn Fn(String), rep: RemoteLinkerRep, ctl: &super::HostCtl) {
    let env = match rep {
        RemoteLinkerRep::Graph(r) => PushEnvelope::ImportGraph(r),
        RemoteLinkerRep::Impact(r) => PushEnvelope::ImportGraphImpact(r),
        RemoteLinkerRep::Error { error, request_id } => {
            // Synthesize an unavailable-shaped result so the GUI can unblock.
            match ctl {
                super::HostCtl::ImportGraphImpact {
                    path,
                    depth,
                    request_id: rid,
                    session_id,
                } => PushEnvelope::ImportGraphImpact(ImportGraphImpactResult {
                    request_id: rid.clone(),
                    session_id: session_id.clone(),
                    path: path.clone(),
                    depth: *depth,
                    paths: vec![],
                    total: 0,
                    error: Some(error),
                }),
                super::HostCtl::ImportGraph {
                    session_id,
                    request_id: rid,
                    ..
                } => {
                    let mut r = import_graph::unavailable_result();
                    r.status = format!("unavailable: {error}");
                    r.request_id = request_id.or_else(|| rid.clone());
                    r.session_id = session_id.clone();
                    PushEnvelope::ImportGraph(r)
                }
                super::HostCtl::ImportGraphReindex { request_id: rid } => {
                    let mut r = import_graph::unavailable_result();
                    r.status = format!("unavailable: {error}");
                    r.request_id = request_id.or_else(|| rid.clone());
                    PushEnvelope::ImportGraph(r)
                }
                _ => return,
            }
        }
        RemoteLinkerRep::Hello { .. } | RemoteLinkerRep::SetRoots { .. } => return,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        push(json);
    }
}

/// Push an unavailable ImportGraph* result when remote attach has no live
/// remote-linker child.
pub(super) fn push_remote_linker_unavailable(ctl: &super::HostCtl, push: &dyn Fn(String)) {
    const ERR: &str = "remote-linker unavailable";
    let env = match ctl {
        super::HostCtl::ImportGraph {
            session_id,
            request_id,
            ..
        } => {
            let mut r = import_graph::unavailable_result();
            r.status = format!("unavailable: {ERR}");
            r.request_id = request_id.clone();
            r.session_id = session_id.clone();
            PushEnvelope::ImportGraph(r)
        }
        super::HostCtl::ImportGraphReindex { request_id } => {
            let mut r = import_graph::unavailable_result();
            r.status = format!("unavailable: {ERR}");
            r.request_id = request_id.clone();
            PushEnvelope::ImportGraph(r)
        }
        super::HostCtl::ImportGraphImpact {
            path,
            depth,
            request_id,
            session_id,
        } => PushEnvelope::ImportGraphImpact(ImportGraphImpactResult {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            path: path.clone(),
            depth: *depth,
            paths: vec![],
            total: 0,
            error: Some(ERR.into()),
        }),
        _ => return,
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
                        Err(e) => RemoteLinkerRep::Error {
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
    req: &RemoteLinkerReq,
) -> Result<RemoteLinkerRep, String>
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

// Silence unused-import warning if ImportGraphResult is only used via type inference.
#[allow(dead_code)]
fn _result_ty(_: &ImportGraphResult) {}

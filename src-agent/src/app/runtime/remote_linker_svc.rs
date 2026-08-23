//! `koma remote-linker` — long-lived stdio thin client for Import Graph panel ops.
//!
//! Runs on the remote machine (spawned over SSH). Reads length-prefixed
//! [`super::remote_linker_proto::RemoteLinkerReq`] frames from stdin, executes
//! them via [`super::client::import_graph`] against the remote linker daemon,
//! writes [`super::remote_linker_proto::RemoteLinkerRep`] frames to stdout.
//!
//! Not a session-daemon bridge — owns linker work directly (same model as
//! remote-fs).

use std::path::PathBuf;

use anyhow::Result;

use crate::ipc::frame::{self, FrameReader};

use super::client::import_graph::{self, ImportGraphJob};
use super::remote_linker_proto::{RemoteLinkerRep, RemoteLinkerReq};

/// Entry point for `koma remote-linker`.
///
/// Optional `--cwd` seeds the initial workdir root list. Further roots arrive via
/// `RemoteLinkerReq::SetRoots` (host pushes session workdirs after Snapshot/Settings).
pub fn run_remote_linker(opts: crate::cli::Opts) -> Result<()> {
    // Ignore SIGPIPE — a broken-pipe write returns EPIPE instead of killing us.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(cwd) = opts.cwd.as_deref() {
        if cwd.is_empty() || cwd.contains('\0') {
            anyhow::bail!("invalid remote working directory");
        }
        let p = PathBuf::from(cwd);
        if p.is_absolute() {
            roots.push(p);
        } else {
            anyhow::bail!("--cwd must be an absolute path");
        }
    }

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
            let req: RemoteLinkerReq = match serde_json::from_slice(&bytes) {
                Ok(r) => r,
                Err(e) => {
                    let rep = RemoteLinkerRep::Error {
                        error: format!("invalid remote-linker request: {e}"),
                        request_id: None,
                    };
                    write_rep(&mut stdout, &rep).await?;
                    continue;
                }
            };
            let rep = handle_req(req, &mut roots);
            write_rep(&mut stdout, &rep).await?;
        }
    })
}

async fn write_rep<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut W,
    rep: &RemoteLinkerRep,
) -> Result<()> {
    let out = serde_json::to_vec(rep)?;
    frame::write_frame_to(stdout, &out).await?;
    Ok(())
}

fn handle_req(req: RemoteLinkerReq, roots: &mut Vec<PathBuf>) -> RemoteLinkerRep {
    match req {
        RemoteLinkerReq::Hello => RemoteLinkerRep::Hello {
            roots: roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        RemoteLinkerReq::SetRoots { roots: new_roots } => {
            let mut parsed = Vec::with_capacity(new_roots.len());
            for r in &new_roots {
                if r.is_empty() || r.contains('\0') {
                    return RemoteLinkerRep::SetRoots {
                        roots: roots
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect(),
                        error: Some("invalid root path".to_string()),
                    };
                }
                let p = PathBuf::from(r);
                if !p.is_absolute() {
                    return RemoteLinkerRep::SetRoots {
                        roots: roots
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect(),
                        error: Some(format!("root must be absolute: {r}")),
                    };
                }
                parsed.push(p);
            }
            *roots = parsed;
            RemoteLinkerRep::SetRoots {
                roots: roots
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                error: None,
            }
        }
        RemoteLinkerReq::Graph {
            path,
            depth,
            direction,
            filter_roots,
            filter_languages,
            session_id,
            request_id,
        } => {
            let configured_roots = crate::linker::client::canonical_roots(roots);
            let configured_root_map = crate::linker::client::configured_root_map(roots);
            RemoteLinkerRep::Graph(import_graph::exec_import_graph(ImportGraphJob {
                path,
                depth,
                direction,
                filter_roots,
                filter_languages,
                configured_roots,
                configured_root_map,
                session_id,
                request_id,
            }))
        }
        RemoteLinkerReq::Impact {
            path,
            depth,
            request_id,
            session_id,
        } => {
            let configured_roots = crate::linker::client::canonical_roots(roots);
            RemoteLinkerRep::Impact(import_graph::exec_import_graph_impact(
                path,
                depth,
                request_id,
                configured_roots,
                session_id,
            ))
        }
        RemoteLinkerReq::Reindex {
            session_id,
            request_id,
            filter_roots,
            filter_languages,
        } => {
            let configured_roots = crate::linker::client::canonical_roots(roots);
            let configured_root_map = crate::linker::client::configured_root_map(roots);
            RemoteLinkerRep::Graph(import_graph::exec_import_graph_reindex(
                session_id,
                request_id,
                configured_roots,
                configured_root_map,
                filter_roots,
                filter_languages,
            ))
        }
    }
}

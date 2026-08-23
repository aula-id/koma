//! `koma remote-fs` — long-lived stdio thin client for Coding panel File* ops.
//!
//! Runs on the remote machine (spawned over SSH). Reads length-prefixed
//! [`super::remote_fs_proto::RemoteFsReq`] frames from stdin, executes them via
//! [`super::client::file_ops`] against a workdir sandbox, writes
//! [`super::remote_fs_proto::RemoteFsRep`] frames to stdout.
//!
//! Not a session-daemon bridge — owns FS work directly (same model as the linker
//! daemon's private request/response protocol).

use std::path::PathBuf;

use anyhow::Result;

use crate::ipc::frame::{self, FrameReader};

use super::client::file_ops;
use super::remote_fs_proto::{RemoteFsRep, RemoteFsReq};

/// Entry point for `koma remote-fs`.
///
/// Optional `--cwd` seeds the initial sandbox root list. Further roots arrive via
/// `RemoteFsReq::SetRoots` (host pushes session workdirs after Snapshot/Settings).
pub fn run_remote_fs(opts: crate::cli::Opts) -> Result<()> {
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
            let req: RemoteFsReq = match serde_json::from_slice(&bytes) {
                Ok(r) => r,
                Err(e) => {
                    let rep = RemoteFsRep::Error {
                        error: format!("invalid remote-fs request: {e}"),
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
    rep: &RemoteFsRep,
) -> Result<()> {
    let out = serde_json::to_vec(rep)?;
    frame::write_frame_to(stdout, &out).await?;
    Ok(())
}

fn handle_req(req: RemoteFsReq, roots: &mut Vec<PathBuf>) -> RemoteFsRep {
    match req {
        RemoteFsReq::Hello => RemoteFsRep::Hello {
            roots: roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        RemoteFsReq::SetRoots { roots: new_roots } => {
            let mut parsed = Vec::with_capacity(new_roots.len());
            for r in &new_roots {
                if r.is_empty() || r.contains('\0') {
                    return RemoteFsRep::SetRoots {
                        roots: roots
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect(),
                        error: Some("invalid root path".to_string()),
                    };
                }
                let p = PathBuf::from(r);
                if !p.is_absolute() {
                    return RemoteFsRep::SetRoots {
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
            RemoteFsRep::SetRoots {
                roots: roots
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                error: None,
            }
        }
        RemoteFsReq::Tree {
            root,
            path,
            request_id,
        } => RemoteFsRep::Tree(file_ops::exec_file_tree(&root, &path, &request_id, roots)),
        RemoteFsReq::Read {
            root,
            path,
            request_id,
        } => RemoteFsRep::Read(file_ops::exec_file_read(&root, &path, &request_id, roots)),
        RemoteFsReq::Save {
            root,
            path,
            content,
            expected_fingerprint,
            request_id,
        } => RemoteFsRep::Save(file_ops::exec_file_save(
            &root,
            &path,
            &content,
            &expected_fingerprint,
            &request_id,
            roots,
        )),
        RemoteFsReq::Create {
            root,
            path,
            kind,
            request_id,
        } => RemoteFsRep::Create(file_ops::exec_file_create(
            &root,
            &path,
            &kind,
            &request_id,
            roots,
        )),
        RemoteFsReq::Rename {
            root,
            old_path,
            new_path,
            request_id,
        } => RemoteFsRep::Rename(file_ops::exec_file_rename(
            &root,
            &old_path,
            &new_path,
            &request_id,
            roots,
        )),
        RemoteFsReq::Delete {
            root,
            path,
            request_id,
        } => RemoteFsRep::Delete(file_ops::exec_file_delete(
            &root,
            &path,
            &request_id,
            roots,
        )),
        RemoteFsReq::WriteBytes {
            root,
            path,
            bytes_b64,
            overwrite,
            request_id,
        } => RemoteFsRep::WriteBytes(file_ops::exec_file_write_bytes(
            &root,
            &path,
            &bytes_b64,
            overwrite,
            &request_id,
            roots,
        )),
        RemoteFsReq::DownloadBytes {
            root,
            path,
            request_id,
        } => RemoteFsRep::DownloadBytes(file_ops::exec_file_download_bytes(
            &root,
            &path,
            &request_id,
            roots,
        )),
    }
}

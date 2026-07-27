//! Thin one-shot IPC client for the linker daemon.
//!
//! Used by L1 (session graph summary injection) and L3 (auto-neighborhood
//! footers on read/edit/write). All calls are best-effort: `None` on any
//! failure (daemon not running, timeout, bad response).

use crate::ipc::frame::FrameReader;
use crate::ipc::linker_proto::{LinkerQuery, LinkerRequest, LinkerResponse};
use crate::ipc::SyncIpcStream;
use std::io::{Read, Write};
use std::time::Duration;

/// Timeout for linker daemon IPC round-trips.
const IO_TIMEOUT: Duration = Duration::from_secs(3);

/// Open a sync Unix socket to the linker daemon, send a request, and read the
/// response frame. Returns `None` if any step fails (daemon not running,
/// timeout, bad frame, decode error).
fn connect_and_send(req: &LinkerRequest) -> Option<LinkerResponse> {
    let sock_path = crate::model::store::linker_daemon_sock_path().ok()?;
    let mut stream = SyncIpcStream::connect(&sock_path).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;

    let payload = serde_json::to_vec(req).ok()?;
    let prefix = (payload.len() as u32).to_be_bytes();
    stream.write_all(&prefix).ok()?;
    stream.write_all(&payload).ok()?;
    stream.flush().ok()?;

    // Read response frame.
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        reader.push(&buf[..n]);
        if let Some(frame) = reader.next_frame().ok()? {
            return serde_json::from_slice(&frame).ok();
        }
    }
}

/// Result of a linker summary query, for L1 injection.
pub struct SummaryResult {
    pub text: String,
    pub generation: u64,
    pub file_count: usize,
    pub edge_count: usize,
    pub languages: Vec<String>,
}

/// One-shot fetch of the linker daemon's graph summary. Returns `None` if the
/// daemon is not running or not ready.
pub fn fetch_summary() -> Option<SummaryResult> {
    let resp = connect_and_send(&LinkerRequest::Summary)?;
    match resp {
        LinkerResponse::Summary {
            text,
            generation,
            file_count,
            edge_count,
            languages,
        } => Some(SummaryResult {
            text,
            generation,
            file_count,
            edge_count,
            languages,
        }),
        _ => None,
    }
}

/// One-shot fetch of the 1-hop neighborhood for a file. Returns
/// `(imports, imported_by)` — two separate lists of paths. Uses the
/// `Neighborhood` query and parses the `(dependency)` / `(dependent)` suffixes
/// the daemon appends, avoiding two round-trips.
pub fn fetch_neighborhood(path: &str) -> Option<(Vec<String>, Vec<String>)> {
    let resp = connect_and_send(&LinkerRequest::Query(LinkerQuery::Neighborhood {
        path: path.to_string(),
    }))?;
    let paths = match resp {
        LinkerResponse::PathList { paths, .. } => paths,
        _ => return None,
    };

    let mut imports = Vec::new();
    let mut imported_by = Vec::new();
    for entry in &paths {
        if let Some(p) = entry.strip_suffix(" (dependency)") {
            imports.push(p.to_string());
        } else if let Some(p) = entry.strip_suffix(" (dependent)") {
            imported_by.push(p.to_string());
        } else {
            // Fallback: treat as dependency if no suffix (shouldn't happen).
            imports.push(entry.clone());
        }
    }
    Some((imports, imported_by))
}



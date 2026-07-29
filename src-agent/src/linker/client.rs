//! Thin one-shot IPC client for the linker daemon.
//!
//! Used by L1 (session graph summary injection) and L3 (auto-neighborhood
//! footers on read/edit/write). All calls are best-effort: `None` on any
//! failure (daemon not running, timeout, bad response).

use crate::ipc::frame::FrameReader;
use crate::ipc::linker_proto::{LinkerQuery, LinkerRequest, LinkerResponse};
use crate::ipc::SyncIpcStream;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// Timeout for linker daemon IPC round-trips.
const IO_TIMEOUT: Duration = Duration::from_secs(3);

/// Open a sync Unix socket to the linker daemon, send a request, and read the
/// response frame. Returns `None` if any step fails (daemon not running,
/// timeout, bad frame, decode error).
pub(crate) fn connect_and_send(req: &LinkerRequest) -> Option<LinkerResponse> {
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
    /// Populated for callers/tools; L1 currently uses `text` only.
    #[allow(dead_code)]
    pub file_count: usize,
    /// Populated for callers/tools; L1 currently uses `text` only.
    #[allow(dead_code)]
    pub edge_count: usize,
    /// Populated for callers/tools; L1 currently uses `text` only.
    #[allow(dead_code)]
    pub languages: Vec<String>,
}

/// One-shot fetch of just the linker daemon's current graph generation. O(1) on
/// the daemon side — no summary text computed, no fan-in/entry-point scan.
/// Returns `None` if the daemon is not running.
pub fn fetch_generation() -> Option<u64> {
    let resp = connect_and_send(&LinkerRequest::Generation)?;
    match resp {
        LinkerResponse::Generation(g) => Some(g),
        _ => None,
    }
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

/// Ensure the linker daemon is running and register the given roots.
/// Returns Ok(()) on success, best-effort error on failure.
pub fn ensure_and_register(roots: &[PathBuf], client_id: &str) -> Result<(), String> {
    crate::app::ensure_linker_daemon_running()
        .map_err(|e| format!("failed to start linker daemon: {e}"))?;

    let root_strs: Vec<String> = roots
        .iter()
        .map(|p| {
            p.canonicalize()
                .unwrap_or_else(|_| p.clone())
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();

    let req = LinkerRequest::RegisterWorkspaces {
        roots: root_strs,
        session_id: client_id.to_string(),
    };

    match connect_and_send(&req) {
        Some(_) => Ok(()),
        None => Err("linker daemon did not respond".into()),
    }
}

/// Unregister a client from the linker daemon.
pub fn unregister_client(client_id: &str) {
    let req = LinkerRequest::Unregister {
        session_id: client_id.to_string(),
    };
    let _ = connect_and_send(&req);
}

/// Fetch summary only if the generation is newer than `min_gen`.
/// Uses a lightweight `Generation` probe first (O(1) on daemon, no summary
/// text computed) — only fetches the full summary when the generation has
/// actually advanced. Returns None if daemon not running or generation unchanged.
pub fn fetch_summary_if_newer(min_gen: u64) -> Option<SummaryResult> {
    // Fast gate: ask for just the generation number (O(1), tiny payload).
    // If unchanged, skip the expensive full-Summary round-trip entirely.
    let cur = fetch_generation()?;
    if cur <= min_gen {
        return None;
    }
    // Generation advanced — fetch the full summary text.
    fetch_summary()
}

/// Normalize a query path: if relative, join against project roots.
/// If absolute and under a root, return as-is.
/// Falls back to suffix matching against known_files from the graph.
pub fn normalize_query_path(path: &str, project_roots: &[PathBuf]) -> String {
    let p = std::path::Path::new(path);

    // Already absolute — return as-is.
    if p.is_absolute() {
        return path.replace('\\', "/");
    }

    // Relative — try each root.
    for root in project_roots {
        let candidate = root.join(path);
        if candidate.exists() {
            return candidate.to_string_lossy().replace('\\', "/");
        }
    }

    // Fallback: return as-is.
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_query_path_absolute_passthrough() {
        let roots = vec![PathBuf::from("/some/root")];
        assert_eq!(
            normalize_query_path("/foo/bar.rs", &roots),
            "/foo/bar.rs"
        );
    }

    #[test]
    fn normalize_query_path_relative_fallback() {
        // When the file doesn't exist on disk, returns as-is.
        let roots = vec![PathBuf::from("/some/nonexistent")];
        assert_eq!(
            normalize_query_path("src/main.rs", &roots),
            "src/main.rs"
        );
    }

    #[test]
    fn normalize_query_path_backslash() {
        // Windows-style backslashes should be normalized.
        let roots = vec![PathBuf::from("/some/root")];
        assert_eq!(
            normalize_query_path("/foo\\bar.rs", &roots),
            "/foo/bar.rs"
        );
    }
}



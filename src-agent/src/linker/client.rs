//! Persistent IPC client for the linker daemon.
//!
//! Uses a process-global connection pool so repeated calls (L1 summary,
//! L3 auto-neighborhood, `graph_query` tool) reuse a single Unix socket
//! instead of connect/teardown per call.  The daemon's `connection_loop`
//! already supports multiple request/response cycles per connection.
//!
//! All calls remain best-effort: `None` on any failure (daemon not running,
//! timeout, bad frame, decode error).  A broken connection is automatically
//! evicted from the pool and re-established on the next call.

use crate::ipc::frame::FrameReader;
use crate::ipc::linker_proto::{
    GraphViewResult, LinkerQuery, LinkerRequest, LinkerResponse, VisualizationRequest,
};
use crate::ipc::SyncIpcStream;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Timeout for linker daemon IPC round-trips.
const IO_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Connection pool — one persistent socket shared across all callers.
// ---------------------------------------------------------------------------

/// A pooled connection: the stream plus its frame reader (buffer state must
/// stay paired with the stream).
struct PooledConn {
    stream: SyncIpcStream,
    reader: FrameReader,
}

/// Process-global pool holding at most one persistent connection.
/// `OnceLock` avoids `lazy_static` / `once_cell` deps; the `Mutex` serialises
/// access (all callers are on `std::thread` workers, contention is negligible).
fn pool() -> &'static Mutex<Option<PooledConn>> {
    static POOL: OnceLock<Mutex<Option<PooledConn>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(None))
}

/// Open a new connection to the linker daemon.
fn open_connection(sock_path: &std::path::Path) -> Option<PooledConn> {
    let stream = SyncIpcStream::connect(sock_path).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    Some(PooledConn {
        stream,
        reader: FrameReader::new(),
    })
}

/// Write a request frame and read one response frame on an existing connection.
///
/// Returns `None` on any I/O or protocol error — the caller should evict the
/// connection from the pool.
fn send_on_conn(conn: &mut PooledConn, req: &LinkerRequest) -> Option<LinkerResponse> {
    let payload = serde_json::to_vec(req).ok()?;
    let prefix = (payload.len() as u32).to_be_bytes();
    conn.stream.write_all(&prefix).ok()?;
    conn.stream.write_all(&payload).ok()?;
    conn.stream.flush().ok()?;

    // Read exactly one response frame, reusing the conn's FrameReader.
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = conn.stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        conn.reader.push(&buf[..n]);
        if let Some(frame) = conn.reader.next_frame().ok()? {
            return serde_json::from_slice(&frame).ok();
        }
    }
}

/// Send a [`LinkerRequest`] to the linker daemon and return the response.
///
/// Tries the pooled persistent connection first; if it fails (daemon
/// restarted, broken pipe, timeout), evicts it and reconnects.  Returns
/// `None` if the daemon is unreachable.
pub(crate) fn connect_and_send(req: &LinkerRequest) -> Option<LinkerResponse> {
    let sock_path = crate::model::store::linker_daemon_sock_path().ok()?;

    let mut guard = pool().lock().ok()?;

    // Try the existing pooled connection.
    if let Some(ref mut conn) = *guard {
        if let Some(resp) = send_on_conn(conn, req) {
            return Some(resp);
        }
        // Connection broken — evict it.
        *guard = None;
    }

    // Open a fresh connection and send.
    let mut conn = open_connection(&sock_path)?;
    let resp = send_on_conn(&mut conn, req);
    if resp.is_some() {
        *guard = Some(conn);
    }
    resp
}

// ---------------------------------------------------------------------------
// High-level convenience wrappers (unchanged public API).
// ---------------------------------------------------------------------------

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

/// Normalize a query path: if relative, join against project roots and
/// canonicalize when the file exists. If absolute, canonicalize or
/// slash-normalize. When the file doesn't exist on disk, still returns
/// a best-effort absolute path under the primary root so the daemon's
/// suffix match can fire.
pub fn normalize_query_path(path: &str, project_roots: &[PathBuf]) -> String {
    let normalized = path.replace('\\', "/");

    // Multi-root [N] prefix: "[1]src/foo.rs" → resolve bare "src/foo.rs"
    // under project_roots[1] (or fallback to primary on OOB).
    if let Some(rest) = normalized.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            if let Ok(idx) = rest[..end].parse::<usize>() {
                let bare = &rest[end + 1..];
                if !bare.is_empty() {
                    let root = project_roots
                        .get(idx)
                        .or(project_roots.first())
                        .cloned()
                        .unwrap_or_default();
                    let candidate = root.join(bare);
                    if let Ok(canon) = std::fs::canonicalize(&candidate) {
                        return canon.to_string_lossy().replace('\\', "/");
                    }
                    return candidate.to_string_lossy().replace('\\', "/");
                }
            }
        }
    }

    let p = std::path::Path::new(&normalized);

    // Already absolute — canonicalize if possible, else slash-normalize.
    if p.is_absolute() {
        return std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/");
    }

    // Relative — try each root; prefer canonicalized path for existing files.
    for root in project_roots {
        let candidate = root.join(p);
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            return canon.to_string_lossy().replace('\\', "/");
        }
    }

    // File doesn't exist on disk: still return an absolute path under the
    // primary root so the daemon suffix-match has a chance.
    if let Some(root) = project_roots.first() {
        return root.join(p).to_string_lossy().replace('\\', "/");
    }

    // No roots at all — return slash-normalized as-is.
    p.to_string_lossy().replace('\\', "/")
}

/// Fetch a bounded subgraph view for GUI visualization.
/// Returns `None` if the linker daemon is unreachable.
pub fn fetch_graph_view(req: &VisualizationRequest) -> Option<GraphViewResult> {
    let resp = connect_and_send(&LinkerRequest::Query(LinkerQuery::Visualization(
        req.clone(),
    )))?;
    match resp {
        LinkerResponse::GraphView(v) => Some(v),
        _ => None,
    }
}

/// Fetch transitive impact analysis for a file.
/// Returns `(paths, total)` on success, or `Err(message)` on any failure.
pub fn fetch_impact(path: &str, depth: u32) -> Result<(Vec<String>, usize), String> {
    let resp = connect_and_send(&LinkerRequest::Query(LinkerQuery::Impact {
        path: path.to_string(),
        depth: Some(depth),
    }))
    .ok_or_else(|| "linker daemon unreachable".to_string())?;
    match resp {
        LinkerResponse::PathList { paths, total } => Ok((paths, total)),
        LinkerResponse::Error(e) => Err(e),
        _ => Err("unexpected linker daemon response".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_query_path_absolute_passthrough() {
        let roots = vec![PathBuf::from("/some/root")];
        assert_eq!(normalize_query_path("/foo/bar.rs", &roots), "/foo/bar.rs");
    }

    #[test]
    fn normalize_query_path_relative_fallback() {
        // When the file doesn't exist on disk, returns primary_root + path.
        let roots = vec![PathBuf::from("/some/nonexistent")];
        assert_eq!(
            normalize_query_path("src/main.rs", &roots),
            "/some/nonexistent/src/main.rs"
        );
    }

    #[test]
    fn normalize_query_path_backslash() {
        // Windows-style backslashes should be normalized.
        let roots = vec![PathBuf::from("/some/root")];
        assert_eq!(normalize_query_path("/foo\\bar.rs", &roots), "/foo/bar.rs");
    }

    #[test]
    fn normalize_query_path_existing_file_canonicalizes() {
        // Create a temp dir + file manually (no tempfile crate).
        let dir = std::env::temp_dir().join(format!("koma_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("hello.rs"), "fn main() {}").unwrap();
        let result = normalize_query_path("hello.rs", std::slice::from_ref(&dir));
        // Should be the canonical absolute path.
        assert!(std::path::Path::new(&result).is_absolute());
        assert!(result.ends_with("hello.rs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_query_path_no_roots_returns_bare() {
        let result = normalize_query_path("src/main.rs", &[]);
        assert_eq!(result, "src/main.rs");
    }

    #[test]
    fn normalize_query_path_ws_prefix_primary() {
        // [0]src/main.rs → resolved under roots[0]
        let root_a = std::env::temp_dir().join(format!("koma_test_{}_a", std::process::id()));
        let _ = std::fs::create_dir_all(root_a.join("src"));
        std::fs::write(root_a.join("src/main.rs"), "fn main() {}").unwrap();
        let result = normalize_query_path("[0]src/main.rs", std::slice::from_ref(&root_a));
        assert!(result.contains("src/main.rs"));
        // On macOS, temp_dir() symlinks /var → /private/var; canonicalize resolves
        // the symlink, so compare against the canonicalized + slash-normalized root.
        let expected_root = std::fs::canonicalize(&root_a)
            .unwrap_or(root_a.clone())
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            result.starts_with(&expected_root),
            "result={result:?} expected_root={expected_root:?}"
        );
        let _ = std::fs::remove_dir_all(&root_a);
    }

    #[test]
    fn normalize_query_path_ws_prefix_secondary() {
        // [1]pkg/README.md → resolved under roots[1]
        let root_a = std::env::temp_dir().join(format!("koma_test_{}_a2", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("koma_test_{}_b2", std::process::id()));
        let _ = std::fs::create_dir_all(&root_a);
        let _ = std::fs::create_dir_all(root_b.join("pkg"));
        std::fs::write(root_b.join("pkg/README.md"), "hello").unwrap();
        let result = normalize_query_path("[1]pkg/README.md", &[root_a.clone(), root_b.clone()]);
        assert!(result.contains("pkg/README.md"));
        // File exists → canonicalize resolves macOS /var → /private/var symlink;
        // compare against the canonicalized + slash-normalized root.
        let expected_root = std::fs::canonicalize(&root_b)
            .unwrap_or(root_b.clone())
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            result.starts_with(&expected_root),
            "result={result:?} expected_root={expected_root:?}"
        );
        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[test]
    fn normalize_query_path_ws_prefix_oob_falls_back() {
        // [9]src/main.rs with only 2 roots → falls back to primary root
        let root_a = std::env::temp_dir().join(format!("koma_test_{}_c", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("koma_test_{}_d", std::process::id()));
        let _ = std::fs::create_dir_all(&root_a);
        let _ = std::fs::create_dir_all(&root_b);
        let result = normalize_query_path("[9]src/main.rs", &[root_a.clone(), root_b.clone()]);
        // OOB index falls back to primary root (no canonicalize — file absent);
        // slash-normalize the expected prefix for Windows compatibility.
        let expected = root_a.to_string_lossy().replace('\\', "/");
        assert!(
            result.starts_with(&expected),
            "result={result:?} expected={expected:?}"
        );
        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[test]
    fn normalize_query_path_ws_prefix_empty_bare() {
        // "[0]" with no bare path → falls through to normal relative logic
        let roots = vec![PathBuf::from("/some/root")];
        let result = normalize_query_path("[0]", &roots);
        // Empty bare falls through, treated as relative path "[0]" → primary root
        assert_eq!(result, "/some/root/[0]");
    }
}

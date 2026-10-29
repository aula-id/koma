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
    EditContextResult, GraphViewResult, LinkerQuery, LinkerRequest, LinkerResponse,
    VisualizationRequest,
};
use crate::ipc::SyncIpcStream;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Timeout for linker daemon IPC round-trips.
const IO_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Per-session registration revision counter (rejects stale registrations).
// ---------------------------------------------------------------------------

/// Monotonically increasing registration revision. The wall-clock component
/// keeps revisions newer after a client-process restart while the in-process
/// last value preserves ordering for concurrent saves in the same timestamp.
fn next_revision(_session_id: &str) -> u64 {
    static LAST_REVISION: OnceLock<Mutex<u64>> = OnceLock::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64;
    let mut last = LAST_REVISION
        .get_or_init(|| Mutex::new(0))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *last = now.max(last.saturating_add(1));
    *last
}

/// Allocate a registration revision synchronously (public).  Call this
/// immediately after an authoritative settings save/change succeeds — **before**
/// spawning the background registration thread — so the revision is captured
/// deterministically.  The returned revision is restart-safe (wall-clock
/// monotonic) and ensures that a delayed older worker is rejected by the daemon.
pub fn next_registration_revision(session_id: &str) -> u64 {
    next_revision(session_id)
}

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
#[allow(dead_code)] // public API: superseded by fetch_edit_context for footer, but useful for callers
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

/// Canonical root normalization: the authoritative form both the linker daemon
/// and the GUI use for workspace root comparison. Canonicalizes via
/// `std::fs::canonicalize` (resolves symlinks) when possible, falls back to
/// making the path absolute via cwd (lexical normalization) when the path
/// doesn't exist on disk, and always slash-normalizes for cross-platform
/// consistency.
pub fn canonical_root(root: &std::path::Path) -> String {
    match std::fs::canonicalize(root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => {
            // Lexical fallback: make relative paths absolute against cwd so
            // the daemon always receives absolute paths for consistent keying.
            let absolute = if root.is_relative() {
                std::env::current_dir().unwrap_or_default().join(root)
            } else {
                root.to_path_buf()
            };
            absolute.to_string_lossy().replace('\\', "/")
        }
    }
}

/// Canonicalize a set of workspace roots into the authoritative string form
/// used by both the linker daemon and the GUI scope comparison.
/// Preserves input order (stable-first dedup): the first occurrence of each
/// unique canonical path wins, so configured root order is not destroyed by
/// sorting.
pub fn canonical_roots(roots: &[PathBuf]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut v: Vec<String> = Vec::with_capacity(roots.len());
    for root in roots {
        let canon = canonical_root(root);
        if seen.insert(canon.clone()) {
            v.push(canon);
        }
    }
    v
}

/// Build a mapping from canonical path → raw configured path for workspace
/// roots where the two differ (e.g. symlinks, relative paths, non-canonical
/// spellings).  Used by the import-graph workers so the GUI root DTO carries
/// both the canonical `root` and the user's original `configured_path`.
///
/// Deduplicates using the same stable-first logic as [`canonical_roots`]:
/// only the first occurrence of each canonical path is recorded.
pub fn configured_root_map(roots: &[PathBuf]) -> std::collections::HashMap<String, String> {
    let mut seen = std::collections::HashSet::new();
    let mut map = std::collections::HashMap::new();
    for root in roots {
        let canon = canonical_root(root);
        if seen.insert(canon.clone()) {
            let raw = root.to_string_lossy().replace('\\', "/");
            if canon != raw {
                map.insert(canon, raw);
            }
        }
    }
    map
}

/// Ensure the linker daemon is running and register the given roots.
/// Returns Ok(()) on success, best-effort error on failure.
/// Automatically stamps a monotonically-increasing per-session revision so
/// the daemon can reject stale out-of-order registrations.
///
/// **Validates** the daemon response: only `Registered` is treated as success;
/// `Error` and unexpected variants produce `Err`.
pub fn ensure_and_register(roots: &[PathBuf], client_id: &str) -> Result<(), String> {
    let revision = next_revision(client_id);
    ensure_and_register_with_revision(roots, client_id, revision)
}

/// Ensure the linker daemon is running and register the given roots with a
/// **pre-allocated revision**.  Use [`next_registration_revision`] to allocate
/// the revision synchronously after an authoritative save, then pass it here
/// from the background thread.  This ensures a delayed older worker is
/// rejected by the daemon's revision gating.
///
/// Only `LinkerResponse::Registered` is treated as success; `Error` and
/// unexpected variants produce `Err`.
pub fn ensure_and_register_with_revision(
    roots: &[PathBuf],
    client_id: &str,
    revision: u64,
) -> Result<(), String> {
    crate::app::ensure_linker_daemon_running()
        .map_err(|e| format!("failed to start linker daemon: {e}"))?;

    let root_strs = canonical_roots(roots);

    let req = LinkerRequest::RegisterWorkspaces {
        roots: root_strs,
        session_id: client_id.to_string(),
        registration_revision: Some(revision),
    };

    match connect_and_send(&req) {
        Some(LinkerResponse::Registered { .. }) => Ok(()),
        Some(LinkerResponse::Error(e)) => Err(e),
        Some(other) => Err(format!("unexpected linker daemon response: {other:?}")),
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

/// Scan coordinator state returned by `fetch_scan_status`.
#[allow(dead_code)]
pub struct ScanCoordinatorState {
    pub desired_revision: u64,
    pub applied_revision: u64,
    pub in_flight: Option<u64>,
    pub generation: u64,
}

/// Fetch scan coordinator state (desired/applied/in-flight revisions).
/// Returns `None` if the daemon is unreachable.
pub fn fetch_scan_status() -> Option<ScanCoordinatorState> {
    let resp = connect_and_send(&LinkerRequest::Query(LinkerQuery::ScanStatus))?;
    match resp {
        LinkerResponse::ScanStatusResponse {
            desired_revision,
            applied_revision,
            in_flight,
            generation,
        } => Some(ScanCoordinatorState {
            desired_revision,
            applied_revision,
            in_flight,
            generation,
        }),
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

/// Fetch rich edit context for a file. Single IPC round-trip.
pub fn fetch_edit_context(path: &str) -> Option<EditContextResult> {
    let resp = connect_and_send(&LinkerRequest::Query(LinkerQuery::EditContext {
        path: path.to_string(),
    }))?;
    match resp {
        LinkerResponse::EditContext(ctx) => Some(ctx),
        _ => None,
    }
}

/// Result of a blast-radius query.
#[allow(dead_code)] // public API: not yet consumed by tools (blast_radius uses connect_and_send directly)
pub struct BlastRadiusResult {
    pub affected_count: usize,
    pub entry_point_count: usize,
    pub paths: Vec<String>,
}

/// Fetch blast radius (impact) for a file at a given depth.
#[allow(dead_code)] // public API: not yet consumed by tools (blast_radius uses connect_and_send directly)
pub fn fetch_blast_radius(path: &str, depth: u32) -> Option<BlastRadiusResult> {
    let resp = connect_and_send(&LinkerRequest::Query(LinkerQuery::Impact {
        path: path.to_string(),
        depth: Some(depth.min(3)),
    }))?;
    match resp {
        LinkerResponse::PathList { paths, total } => Some(BlastRadiusResult {
            affected_count: total,
            entry_point_count: 0,
            paths,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod tests;

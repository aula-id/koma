//! Off-thread worker for the import-graph visualization `HostCtl` handler.
//!
//! Calls `linker::client::fetch_graph_view()` on a std::thread (blocking IPC),
//! sends the result back over an `mpsc` channel to be drained by `push_loop`.
//!
//! All visualization queries are **foreground-session scoped**: the configured
//! workdirs (from the attached session's Settings) define the *allow-set*.
//! UI `All`/`null` means all configured roots (never the daemon-global graph).
//! Explicit root selections are intersected with the configured set.  Stale or
//! foreign selections fall back to the full configured allow-set.  No configured
//! roots returns an empty scoped result without querying the daemon.
//!
//! After the daemon returns, nodes/edges outside the allow-set are removed and
//! configured roots missing from the graph are synthesised with zero metadata and
//! an explicit not-indexed/scanning state.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc::Sender;

use crate::ipc::linker_proto::{GraphDirection, GraphViewNode, GraphViewResult, WorkspaceRootInfo};

// ─── Reindex polling constants ──────────────────────────────────────────────

/// Maximum time (seconds) to poll for scan completion after Rescan.
const REINDEX_POLL_TIMEOUT_SECS: u64 = 10;
/// Interval between ScanStatus polls (milliseconds).
const REINDEX_POLL_INTERVAL_MS: u64 = 200;

// ─── Public DTOs (GUI push contract) ────────────────────────────────────────

/// Workspace-relative import-graph view result for the GUI.
/// Matches the fields of `GraphViewResult` but is self-contained (no linker
/// daemon types leak into the GUI push contract).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphResult {
    /// Status of the graph response: "ok" (successful graph data),
    /// "unavailable" (linker daemon unreachable/error),
    /// "scanning" (generation 0 with no files — scan in progress),
    /// "not-indexed" (configured root not yet scanned).
    pub status: String,
    pub nodes: Vec<ImportGraphNode>,
    pub edges: Vec<ImportGraphEdge>,
    pub focus: Option<String>,
    pub generation: u64,
    pub file_count: usize,
    pub edge_count: usize,
    pub languages: Vec<String>,
    pub nodes_truncated: bool,
    pub edges_truncated: bool,
    pub total_nodes_available: usize,
    pub total_edges_available: usize,
    pub available_roots: Vec<ImportGraphRootInfo>,
    /// Correlation id for matching this reply to its originating request.
    /// `None` for normal graph queries; `Some(id)` for reindex results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Session id this result belongs to.  The frontend can compare this to the
    /// current foreground session and drop stale results (e.g. a reindex started
    /// before a session switch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphNode {
    pub path: String,
    pub language: String,
    pub out_degree: usize,
    pub in_degree: usize,
    pub role: String,
    pub depth_from_focus: Option<u32>,
    pub workspace_root: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportGraphEdge {
    pub from: String,
    pub to: String,
}

/// Per-root workspace metadata for filter pickers.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphRootInfo {
    /// Canonical identity path used for requests, security, and matching.
    pub root: String,
    /// The configured (user-input) path — may be a symlink or relative
    /// spelling.  Identical to `root` when the configured path already
    /// canonicalises to `root`.  Omitted when it equals `root`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_path: Option<String>,
    /// Human-friendly display label for the UI (typically basename).
    /// Kept distinct from `root` so the frontend can show a short name
    /// while keeping the canonical path in the title/sublabel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_path: Option<String>,
    pub file_count: usize,
    pub languages: Vec<ImportGraphLangCount>,
    /// Per-root indexing state: "indexed" (files present), "scanning" (daemon
    /// knows about this root but scan is in progress), "not-indexed"
    /// (configured root missing from the daemon graph entirely), or
    /// "unavailable" (terminal scan failure or daemon unreachable).
    pub indexed_state: String,
}

/// Language with count for per-root breakdown.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphLangCount {
    pub name: String,
    pub count: usize,
}

// ─── Session-scoped query helpers ───────────────────────────────────────────

/// Compute the effective `filter_roots` for the daemon's `VisualizationRequest`
/// given the UI selection and the foreground session's configured (canonical)
/// workdirs.
///
/// Rules:
/// - **No configured roots** → return `None` (caller short-circuits to empty).
/// - **`All` / `None` UI selection** → all configured roots.
/// - **Explicit UI selection** → intersect with configured; if intersection is
///   empty (stale/foreign), fall back to the full configured allow-set.
pub fn compute_effective_filter(
    ui_filter_roots: Option<Vec<String>>,
    configured_roots: &[String],
) -> Option<Vec<String>> {
    if configured_roots.is_empty() {
        return None; // No configured roots → caller returns empty scoped result.
    }
    match ui_filter_roots {
        None => Some(configured_roots.to_vec()), // All → all configured.
        Some(selected) if selected.is_empty() => Some(configured_roots.to_vec()),
        Some(selected) => {
            let configured_set: HashSet<&str> =
                configured_roots.iter().map(|s| s.as_str()).collect();
            let intersection: Vec<String> = selected
                .iter()
                .filter(|r| configured_set.contains(r.as_str()))
                .cloned()
                .collect();
            if intersection.is_empty() {
                // Stale/foreign — fall back to full configured allow-set.
                Some(configured_roots.to_vec())
            } else {
                Some(intersection)
            }
        }
    }
}

/// Determine the `indexed_state` for a configured root given daemon context.
///
/// - If the root is present in the daemon's `available_roots` and has files →
///   `"indexed"`.
/// - If present but zero files and generation is 0 → `"scanning"`.
/// - If present but zero files and generation > 0 → `"indexed"` (genuinely
///   empty workspace).
/// - If absent from daemon's `available_roots` → `"not-indexed"` (or
///   `"scanning"` when a scan is known to be in-flight).
/// - If `scan_failed` is true → `"unavailable"`.
fn compute_root_indexed_state(
    root: &str,
    daemon_root_set: &HashSet<&str>,
    generation: u64,
    daemon_root_file_counts: &std::collections::HashMap<&str, usize>,
    scan_in_flight: bool,
    scan_failed: bool,
) -> String {
    if scan_failed {
        return "unavailable".to_string();
    }
    if !daemon_root_set.contains(root) {
        // Missing from daemon graph: if a scan is in-flight, report "scanning"
        // so the UI shows an active state rather than a stale "not-indexed".
        return if scan_in_flight {
            "scanning".to_string()
        } else {
            "not-indexed".to_string()
        };
    }
    let files = daemon_root_file_counts.get(root).copied().unwrap_or(0);
    if files == 0 && generation == 0 {
        "scanning".to_string()
    } else {
        "indexed".to_string()
    }
}

/// Compute the overall status from per-root states.
///
/// - If ANY root is `"unavailable"` → `"unavailable"`.
/// - If ANY root is `"not-indexed"` → `"not-indexed"`.
/// - If ANY root is `"scanning"` → `"scanning"`.
/// - Otherwise → `"ok"`.
fn compute_overall_status(available_roots: &[ImportGraphRootInfo]) -> String {
    if available_roots.is_empty() {
        return "not-indexed".to_string();
    }
    let any_unavailable = available_roots
        .iter()
        .any(|r| r.indexed_state == "unavailable");
    if any_unavailable {
        return "unavailable".to_string();
    }
    let any_not_indexed = available_roots
        .iter()
        .any(|r| r.indexed_state == "not-indexed");
    if any_not_indexed {
        return "not-indexed".to_string();
    }
    let any_scanning = available_roots
        .iter()
        .any(|r| r.indexed_state == "scanning");
    if any_scanning {
        return "scanning".to_string();
    }
    "ok".to_string()
}

/// Scope a raw daemon `GraphViewResult` to the given allow-set (canonical
/// configured roots).  Nodes and edges outside the allow-set are removed;
/// `available_roots` is restricted to configured roots, and configured roots
/// not present in the daemon graph are synthesised with zero metadata and an
/// explicit `indexed_state`.
///
/// `scan_in_flight` is true when the daemon's scan coordinator reports an
/// active scan thread (drives "scanning" state for missing roots).
/// `scan_failed` is true when the daemon's last scan terminated with an error
/// (drives "unavailable" state).
pub fn scope_result(
    result: GraphViewResult,
    allowed_roots: &[String],
    configured_root_map: &HashMap<String, String>,
    scan_in_flight: bool,
    scan_failed: bool,
) -> ImportGraphResult {
    if allowed_roots.is_empty() {
        return empty_scoped_result();
    }

    let allowed_set: HashSet<&str> = allowed_roots.iter().map(|s| s.as_str()).collect();

    // ── snapshot counts before consuming the vectors ──
    let daemon_node_count = result.nodes.len();
    let daemon_edge_count = result.edges.len();
    let daemon_total_nodes = result.total_nodes_available;
    let daemon_total_edges = result.total_edges_available;
    let daemon_nodes_truncated = result.nodes_truncated;
    let daemon_edges_truncated = result.edges_truncated;
    let generation = result.generation;
    let focus = result.focus;

    // ── nodes: keep only those whose workspace_root is in the allow-set ──
    let scoped_nodes: Vec<ImportGraphNode> = result
        .nodes
        .into_iter()
        .filter(|n| {
            n.workspace_root
                .as_deref()
                .is_some_and(|r| allowed_set.contains(r))
        })
        .map(|n| ImportGraphNode {
            path: n.path,
            language: n.language,
            out_degree: n.out_degree,
            in_degree: n.in_degree,
            role: format!("{:?}", n.role),
            depth_from_focus: n.depth_from_focus,
            workspace_root: n.workspace_root,
        })
        .collect();

    // ── edges: keep only those whose BOTH endpoints are scoped nodes ─────
    let node_paths: HashSet<&str> = scoped_nodes.iter().map(|n| n.path.as_str()).collect();
    let scoped_edges: Vec<ImportGraphEdge> = result
        .edges
        .into_iter()
        .filter(|e| node_paths.contains(e.from.as_str()) && node_paths.contains(e.to.as_str()))
        .map(|e| ImportGraphEdge {
            from: e.from,
            to: e.to,
        })
        .collect();

    // ── languages: recompute from scoped nodes ──
    let mut lang_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for n in &scoped_nodes {
        *lang_counts.entry(n.language.clone()).or_default() += 1;
    }
    let mut languages: Vec<String> = lang_counts.keys().cloned().collect();
    languages.sort();

    // ── available_roots: restrict to configured, then synthesise missing ──
    let all_daemon_roots = result.available_roots.clone();
    let daemon_root_set: HashSet<&str> = all_daemon_roots.iter().map(|r| r.root.as_str()).collect();

    // Build a map of daemon root → file_count for indexed_state computation.
    let daemon_file_counts: std::collections::HashMap<&str, usize> = all_daemon_roots
        .iter()
        .map(|r| (r.root.as_str(), r.file_count))
        .collect();

    // Helper: derive a short display label from a canonical root path.
    fn display_label(root: &str) -> String {
        std::path::Path::new(root)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(root)
            .to_string()
    }

    let mut available_roots: Vec<ImportGraphRootInfo> = result
        .available_roots
        .into_iter()
        .filter(|r| allowed_set.contains(r.root.as_str()))
        .map(|r| {
            let configured_path = configured_root_map.get(r.root.as_str()).cloned();
            let display = configured_path
                .as_ref()
                .map(|cp| display_label(cp))
                .unwrap_or_else(|| display_label(&r.root));
            ImportGraphRootInfo {
                indexed_state: compute_root_indexed_state(
                    &r.root,
                    &daemon_root_set,
                    generation,
                    &daemon_file_counts,
                    scan_in_flight,
                    scan_failed,
                ),
                root: r.root.clone(),
                configured_path,
                display_path: Some(display),
                file_count: r.file_count,
                languages: r
                    .languages
                    .into_iter()
                    .map(|l| ImportGraphLangCount {
                        name: l.name,
                        count: l.count,
                    })
                    .collect(),
            }
        })
        .collect();

    // Synthesise configured roots not yet present in the daemon graph.
    for root in allowed_roots {
        if !daemon_root_set.contains(root.as_str()) {
            let configured_path = configured_root_map.get(root.as_str()).cloned();
            let display = configured_path
                .as_ref()
                .map(|cp| display_label(cp))
                .unwrap_or_else(|| display_label(root));
            available_roots.push(ImportGraphRootInfo {
                root: root.clone(),
                configured_path,
                display_path: Some(display),
                file_count: 0,
                languages: Vec::new(),
                indexed_state: compute_root_indexed_state(
                    root,
                    &daemon_root_set,
                    generation,
                    &daemon_file_counts,
                    scan_in_flight,
                    scan_failed,
                ),
            });
        }
    }

    // Deterministic ordering: configured roots first (already ordered), then
    // any extra daemon roots that happen to be in the allow-set.
    available_roots.sort_by(|a, b| {
        let ai = allowed_roots.iter().position(|r| r == &a.root);
        let bi = allowed_roots.iter().position(|r| r == &b.root);
        ai.cmp(&bi)
    });

    // ── aggregate totals ──
    let total_files: usize = available_roots.iter().map(|r| r.file_count).sum();

    // After client-side scoping, the scoped node count is the best lower
    // bound on total available.  When the daemon already filtered by root
    // (via filter_roots) and no client-side filtering removed nodes, the
    // daemon's pre-cap totals are authoritative.  We detect client-side
    // filtering by comparing scoped vs original daemon node counts.
    let scoped_node_count = scoped_nodes.len();
    let client_filtered = daemon_node_count != scoped_node_count;
    let total_nodes_available = if client_filtered {
        scoped_node_count
    } else {
        daemon_total_nodes
    };

    // Edges: preserve daemon aggregate totals when the client post-filter
    // did not change the scope (no nodes removed).  Only recompute from
    // the scoped edge set when the client actually narrowed the view.
    let total_edges_available = if client_filtered {
        scoped_edges.len()
    } else {
        daemon_total_edges
    };

    // ── truncation: coherent after scoping ──
    // If client-side filtering removed nodes, some of the daemon's
    // truncated edges may have been removed too.  The scoped edge count is
    // the ground truth.  For nodes, if we didn't filter, trust the daemon;
    // if we did filter, the scoped count is below the cap so truncation
    // from our perspective is false (we have all the scoped nodes the
    // daemon returned; we just can't know about nodes beyond the cap).
    let nodes_truncated = if client_filtered {
        false
    } else {
        daemon_nodes_truncated
    };
    let edges_truncated = if client_filtered {
        // Client removed nodes; scoped edge count is ground truth.
        scoped_edges.len() < daemon_edge_count
    } else {
        daemon_edges_truncated
    };

    // ── status: derive from per-root indexed_state ──
    let status = compute_overall_status(&available_roots);

    ImportGraphResult {
        status: status.to_string(),
        nodes: scoped_nodes,
        edges: scoped_edges,
        focus,
        generation,
        file_count: total_files,
        edge_count: total_edges_available,
        languages,
        nodes_truncated,
        edges_truncated,
        total_nodes_available,
        total_edges_available,
        available_roots,
        request_id: None,
        session_id: None,
    }
}

/// An empty scoped result when no configured workdirs are available.
pub fn empty_scoped_result() -> ImportGraphResult {
    ImportGraphResult {
        status: "not-indexed".to_string(),
        nodes: Vec::new(),
        edges: Vec::new(),
        focus: None,
        generation: 0,
        file_count: 0,
        edge_count: 0,
        languages: Vec::new(),
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 0,
        total_edges_available: 0,
        available_roots: Vec::new(),
        request_id: None,
        session_id: None,
    }
}

/// An unavailable result for when the linker daemon is unreachable.
pub fn unavailable_result() -> ImportGraphResult {
    ImportGraphResult {
        status: "unavailable".to_string(),
        nodes: Vec::new(),
        edges: Vec::new(),
        focus: None,
        generation: 0,
        file_count: 0,
        edge_count: 0,
        languages: Vec::new(),
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 0,
        total_edges_available: 0,
        available_roots: Vec::new(),
        request_id: None,
        session_id: None,
    }
}

/// An unavailable result tagged with correlation ids for the frontend to
/// match and reject stale replies.
fn unavailable_with_ids(
    request_id: Option<String>,
    session_id: Option<String>,
) -> ImportGraphResult {
    let mut r = unavailable_result();
    r.request_id = request_id;
    r.session_id = session_id;
    r
}

// ─── Off-thread visualization workers ───────────────────────────────────────

/// Parameters shared by the attached / detached import-graph workers.
pub struct ImportGraphJob {
    pub path: Option<String>,
    pub depth: u32,
    pub direction: GraphDirection,
    pub filter_roots: Option<Vec<String>>,
    pub filter_languages: Option<Vec<String>>,
    pub configured_roots: Vec<String>,
    pub configured_root_map: HashMap<String, String>,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
}

/// Synchronous import-graph fetch + scope (shared by local spawners and
/// `koma remote-linker`).
pub(crate) fn exec_import_graph(job: ImportGraphJob) -> ImportGraphResult {
    let mut result = scoped_fetch_and_convert(
        job.path,
        job.depth,
        job.direction,
        job.filter_roots,
        job.filter_languages,
        &job.configured_roots,
        &job.configured_root_map,
    );
    result.request_id = job.request_id;
    result.session_id = job.session_id;
    result
}

/// Spawn an off-thread `HostCtl::ImportGraph` worker (attached mode).
/// Resolves the effective filter from the foreground session's configured
/// workdirs, calls `fetch_graph_view` on a std::thread, scopes the result,
/// and sends it over the channel.
pub fn spawn_import_graph_attached(tx: Sender<ImportGraphResult>, job: ImportGraphJob) {
    std::thread::spawn(move || {
        let _ = tx.send(exec_import_graph(job));
    });
}

/// Spawn an off-thread `HostCtl::ImportGraph` worker (detached mode).
/// Same scoping logic but pushes the result directly instead of via channel.
pub fn spawn_import_graph(push: impl Fn(String) + Send + 'static, job: ImportGraphJob) {
    std::thread::spawn(move || {
        let result = exec_import_graph(job);
        let env = super::push_proto::PushEnvelope::ImportGraph(result);
        super::render::emit(&push, &env);
    });
}

/// Core worker body: compute effective filter → fetch → scope → convert.
fn scoped_fetch_and_convert(
    path: Option<String>,
    depth: u32,
    direction: GraphDirection,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
    configured_roots: &[String],
    configured_root_map: &HashMap<String, String>,
) -> ImportGraphResult {
    let effective_roots = match compute_effective_filter(filter_roots, configured_roots) {
        Some(r) => r,
        None => return empty_scoped_result(), // No configured roots.
    };

    let req = crate::ipc::linker_proto::VisualizationRequest {
        path,
        depth,
        direction,
        max_nodes: 200,
        max_edges: 400,
        filter_roots: Some(effective_roots.clone()),
        filter_languages,
    };

    // Query scan state for per-root status derivation.
    let (scan_in_flight, scan_failed) = match crate::linker::client::fetch_scan_status() {
        Some(status) => (status.in_flight.is_some(), false),
        None => (false, false), // daemon unreachable handled below
    };

    match crate::linker::client::fetch_graph_view(&req) {
        Some(raw) => scope_result(
            raw,
            &effective_roots,
            configured_root_map,
            scan_in_flight,
            scan_failed,
        ),
        None => {
            // Daemon unreachable — still scope available_roots to configured
            // roots so the GUI shows the configured roots with scan state.
            let mut result = unavailable_result();
            result.available_roots = configured_roots
                .iter()
                .map(|r| {
                    let display = std::path::Path::new(r)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(r)
                        .to_string();
                    ImportGraphRootInfo {
                        root: r.clone(),
                        configured_path: None,
                        display_path: Some(display),
                        file_count: 0,
                        languages: Vec::new(),
                        indexed_state: "unavailable".to_string(),
                    }
                })
                .collect();
            result
        }
    }
}

// ─── Manual reindex workers ─────────────────────────────────────────────────

/// Synchronous reindex + scoped fetch (shared by local spawners and
/// `koma remote-linker`).
pub(crate) fn exec_import_graph_reindex(
    session_id: Option<String>,
    request_id: Option<String>,
    configured_roots: Vec<String>,
    configured_root_map: HashMap<String, String>,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
) -> ImportGraphResult {
    reindex_and_fetch(
        session_id.as_deref(),
        request_id.as_deref(),
        &configured_roots,
        &configured_root_map,
        filter_roots,
        filter_languages,
    )
}

/// Spawn an off-thread reindex worker (attached mode): reconcile/register the
/// foreground session's workdirs, issue Rescan, poll until the scan
/// completes, then fetch + scope + send.  Every failure yields a terminal
/// unavailable/error result so the UI can never stay busy.
pub fn spawn_import_graph_reindex_attached(
    tx: Sender<ImportGraphResult>,
    session_id: String,
    configured_roots: Vec<String>,
    configured_root_map: HashMap<String, String>,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
    request_id: Option<String>,
) {
    std::thread::spawn(move || {
        let result = exec_import_graph_reindex(
            Some(session_id),
            request_id,
            configured_roots,
            configured_root_map,
            filter_roots,
            filter_languages,
        );
        let _ = tx.send(result);
    });
}

/// Spawn an off-thread reindex worker (detached mode): same as above but
/// pushes the result directly.
pub fn spawn_import_graph_reindex(
    push: impl Fn(String) + Send + 'static,
    session_id: String,
    configured_roots: Vec<String>,
    configured_root_map: HashMap<String, String>,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
    request_id: Option<String>,
) {
    std::thread::spawn(move || {
        let result = exec_import_graph_reindex(
            Some(session_id),
            request_id,
            configured_roots,
            configured_root_map,
            filter_roots,
            filter_languages,
        );
        let env = super::push_proto::PushEnvelope::ImportGraph(result);
        super::render::emit(&push, &env);
    });
}

/// Reindex flow with full error handling:
///
/// 1. Validate registration response → terminal error on failure.
/// 2. Issue Rescan → obtain ScanRevision (the exact revision accepted).
/// 3. Poll `ScanStatus` until `applied_revision >= requested` and
///    `in_flight` no longer holds that revision.
/// 4. Scoped visualization fetch with confirmed scan completion.
///
/// Every failure produces a terminal unavailable/error `ImportGraphResult`
/// so the UI never stays in a busy/spinning state.
fn reindex_and_fetch(
    session_id: Option<&str>,
    request_id: Option<&str>,
    configured_roots: &[String],
    configured_root_map: &HashMap<String, String>,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
) -> ImportGraphResult {
    use crate::ipc::linker_proto::{LinkerQuery, LinkerRequest};

    let rid = request_id.map(|s| s.to_string());
    let sid = session_id.map(|s| s.to_string());

    if configured_roots.is_empty() {
        let mut r = empty_scoped_result();
        r.request_id = rid;
        r.session_id = sid;
        return r;
    }

    // Step 1: Register workspace roots with the linker daemon.
    let path_bufs: Vec<std::path::PathBuf> = configured_roots
        .iter()
        .map(std::path::PathBuf::from)
        .collect();

    match crate::linker::client::ensure_and_register(&path_bufs, session_id.unwrap_or("")) {
        Ok(()) => { /* registration succeeded, continue */ }
        Err(e) => {
            let mut r = unavailable_with_ids(rid, sid);
            r.status = format!("unavailable: registration failed: {e}");
            return r;
        }
    }

    // Step 2: Issue Rescan → obtain the exact accepted ScanRevision.
    let requested_revision = match crate::linker::client::connect_and_send(&LinkerRequest::Query(
        LinkerQuery::Rescan,
    )) {
        Some(crate::ipc::linker_proto::LinkerResponse::ScanRevision { revision }) => revision,
        Some(crate::ipc::linker_proto::LinkerResponse::Ack) => {
            // Daemon returned Ack without a revision — this indicates an
            // outdated daemon that does not support exact scan revision
            // tracking.  Treat as a terminal error; the client must not
            // poll on an unspecified revision.
            let mut r = unavailable_with_ids(rid, sid);
            r.status = "unavailable: daemon returned Ack instead of ScanRevision — requires linker daemon with revision support".to_string();
            return r;
        }
        Some(crate::ipc::linker_proto::LinkerResponse::Error(e)) => {
            let mut r = unavailable_with_ids(rid, sid);
            r.status = format!("unavailable: rescan rejected: {e}");
            return r;
        }
        _ => {
            let mut r = unavailable_with_ids(rid, sid);
            r.status = "unavailable: unexpected linker daemon rescan response".to_string();
            return r;
        }
    };

    // Step 3: Poll ScanStatus until the requested revision is applied and
    // no in-flight scan holds it (or a newer one is running).
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(REINDEX_POLL_TIMEOUT_SECS);
    let poll_interval = std::time::Duration::from_millis(REINDEX_POLL_INTERVAL_MS);

    loop {
        if std::time::Instant::now() >= deadline {
            let mut r = unavailable_with_ids(rid, sid);
            r.status = "unavailable: reindex timed out waiting for scan to complete".to_string();
            return r;
        }
        match crate::linker::client::fetch_scan_status() {
            Some(status) => {
                if status.applied_revision >= requested_revision
                    && status.in_flight != Some(requested_revision)
                {
                    // Scan complete — the requested revision (or newer) has
                    // been applied and is no longer the in-flight scan.
                    break;
                }
                // Still in progress — keep polling.
                std::thread::sleep(poll_interval);
            }
            None => {
                let mut r = unavailable_with_ids(rid, sid);
                r.status = "unavailable: linker daemon lost during reindex poll".to_string();
                return r;
            }
        }
    }

    // Step 4: Scoped visualization fetch with confirmed scan completion.
    let mut result = scoped_fetch_and_convert(
        None, // overview mode for reindex
        1,    // depth 1 is sufficient for a refresh
        GraphDirection::Both,
        filter_roots,
        filter_languages,
        configured_roots,
        configured_root_map,
    );
    result.request_id = rid;
    result.session_id = sid;
    result
}

// ─── Impact analysis workers ───────────────────────────────────────────────

/// Build an [`ImportGraphImpactResult`] scoped to the configured workspace
/// roots.  The focal `path` must lie within one of the configured roots; paths
/// outside the allow-set are never disclosed to the frontend.
fn build_scoped_impact_result(
    request_id: String,
    path: String,
    depth: u32,
    configured_roots: &[String],
    session_id: Option<String>,
) -> super::push_proto::ImportGraphImpactResult {
    if configured_roots.is_empty() {
        return super::push_proto::ImportGraphImpactResult {
            request_id,
            session_id,
            path,
            depth,
            paths: vec![],
            total: 0,
            error: Some("no configured workspace roots".to_string()),
        };
    }

    // Reject out-of-scope focal path: the path must lie within at least one
    // configured root.  Use `std::path::Path::starts_with` for component-safe
    // comparison (no prefix-of-prefix false positives like `/workspace/app`
    // matching `/workspace/application-secret`).
    let path_obj = std::path::Path::new(&path);
    let normalised = path.replace('\\', "/");
    let in_scope = configured_roots.iter().any(|root| {
        let nr = root.replace('\\', "/");
        // Prefer Path::starts_with when both are absolute.
        let root_obj = std::path::Path::new(&nr);
        if path_obj.is_absolute() && root_obj.is_absolute() {
            path_obj.starts_with(root_obj)
        } else {
            // Fallback to slash-normalised prefix for edge cases.
            normalised.starts_with(&nr)
        }
    });
    if !in_scope {
        return super::push_proto::ImportGraphImpactResult {
            request_id,
            session_id,
            path,
            depth,
            paths: vec![],
            total: 0,
            error: Some("focal path is outside configured workspace roots".to_string()),
        };
    }

    match crate::linker::client::fetch_impact(&path, depth) {
        Ok((paths, _total)) => {
            // Filter returned paths to configured roots — never disclose
            // foreign paths from the daemon-global graph.  Use component-safe
            // `Path::starts_with` to avoid prefix-of-prefix collisions.
            let allowed_roots: Vec<std::path::PathBuf> = configured_roots
                .iter()
                .map(|r| std::path::PathBuf::from(r.replace('\\', "/")))
                .collect();
            let filtered: Vec<String> = paths
                .into_iter()
                .filter(|p| {
                    let np = p.replace('\\', "/");
                    let pobj = std::path::Path::new(&np);
                    allowed_roots.iter().any(|root| {
                        if pobj.is_absolute() && root.is_absolute() {
                            pobj.starts_with(root)
                        } else {
                            np.starts_with(&*root.to_string_lossy())
                        }
                    })
                })
                .collect();
            let total = filtered.len();
            super::push_proto::ImportGraphImpactResult {
                request_id,
                session_id,
                path,
                depth,
                paths: filtered,
                total,
                error: None,
            }
        }
        Err(e) => super::push_proto::ImportGraphImpactResult {
            request_id,
            session_id,
            path,
            depth,
            paths: vec![],
            total: 0,
            error: Some(e),
        },
    }
}

/// Synchronous impact analysis (shared by local spawners and
/// `koma remote-linker`).
pub(crate) fn exec_import_graph_impact(
    path: String,
    depth: u32,
    request_id: String,
    configured_roots: Vec<String>,
    session_id: Option<String>,
) -> super::push_proto::ImportGraphImpactResult {
    build_scoped_impact_result(request_id, path, depth, &configured_roots, session_id)
}

/// Spawn an off-thread `HostCtl::ImportGraphImpact` worker (attached mode).
/// Calls `fetch_impact` on a std::thread (blocking IPC), sends the result
/// over the channel so `push_loop` can drain + emit without blocking its
/// 16ms fold cadence.
pub fn spawn_import_graph_impact_attached(
    tx: Sender<super::push_proto::ImportGraphImpactResult>,
    path: String,
    depth: u32,
    request_id: String,
    configured_roots: Vec<String>,
    session_id: Option<String>,
) {
    std::thread::spawn(move || {
        let result =
            exec_import_graph_impact(path, depth, request_id, configured_roots, session_id);
        let _ = tx.send(result);
    });
}

/// Spawn an off-thread `HostCtl::ImportGraphImpact` worker (detached mode).
/// Calls `fetch_impact` on a std::thread (blocking IPC), pushes the result
/// directly — no channel needed since there is no fold loop to drain.
pub fn spawn_import_graph_impact(
    push: impl Fn(String) + Send + 'static,
    path: String,
    depth: u32,
    request_id: String,
    configured_roots: Vec<String>,
    session_id: Option<String>,
) {
    std::thread::spawn(move || {
        let result =
            exec_import_graph_impact(path, depth, request_id, configured_roots, session_id);
        let env = super::push_proto::PushEnvelope::ImportGraphImpact(result);
        super::render::emit(&push, &env);
    });
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "import_graph_test.rs"]
mod tests;

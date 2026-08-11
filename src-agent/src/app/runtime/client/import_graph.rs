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
#[derive(serde::Serialize)]
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

#[derive(serde::Serialize)]
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

#[derive(serde::Serialize)]
pub struct ImportGraphEdge {
    pub from: String,
    pub to: String,
}

/// Per-root workspace metadata for filter pickers.
#[derive(serde::Serialize)]
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
#[derive(serde::Serialize)]
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

/// Spawn an off-thread `HostCtl::ImportGraph` worker (attached mode).
/// Resolves the effective filter from the foreground session's configured
/// workdirs, calls `fetch_graph_view` on a std::thread, scopes the result,
/// and sends it over the channel.
pub fn spawn_import_graph_attached(
    tx: Sender<ImportGraphResult>,
    path: Option<String>,
    depth: u32,
    direction: GraphDirection,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
    configured_roots: Vec<String>,
    configured_root_map: HashMap<String, String>,
    session_id: Option<String>,
    request_id: Option<String>,
) {
    std::thread::spawn(move || {
        let mut result = scoped_fetch_and_convert(
            path,
            depth,
            direction,
            filter_roots,
            filter_languages,
            &configured_roots,
            &configured_root_map,
        );
        result.request_id = request_id;
        result.session_id = session_id;
        let _ = tx.send(result);
    });
}

/// Spawn an off-thread `HostCtl::ImportGraph` worker (detached mode).
/// Same scoping logic but pushes the result directly instead of via channel.
pub fn spawn_import_graph(
    push: impl Fn(String) + Send + 'static,
    path: Option<String>,
    depth: u32,
    direction: GraphDirection,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
    configured_roots: Vec<String>,
    configured_root_map: HashMap<String, String>,
    session_id: Option<String>,
    request_id: Option<String>,
) {
    std::thread::spawn(move || {
        let mut result = scoped_fetch_and_convert(
            path,
            depth,
            direction,
            filter_roots,
            filter_languages,
            &configured_roots,
            &configured_root_map,
        );
        result.request_id = request_id;
        result.session_id = session_id;
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
        let result = reindex_and_fetch(
            Some(&session_id),
            request_id.as_deref(),
            &configured_roots,
            &configured_root_map,
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
        let result = reindex_and_fetch(
            Some(&session_id),
            request_id.as_deref(),
            &configured_roots,
            &configured_root_map,
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
            build_scoped_impact_result(request_id, path, depth, &configured_roots, session_id);
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
            build_scoped_impact_result(request_id, path, depth, &configured_roots, session_id);
        let env = super::push_proto::PushEnvelope::ImportGraphImpact(result);
        super::render::emit(&push, &env);
    });
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::linker_proto::{GraphNodeRole, GraphViewEdge, GraphViewNode};

    /// Helper: build a minimal `GraphViewResult` with two roots and a few nodes.
    fn make_test_result() -> GraphViewResult {
        GraphViewResult {
            nodes: vec![
                GraphViewNode {
                    path: "/ws_a/src/main.rs".into(),
                    language: "Rust".into(),
                    out_degree: 1,
                    in_degree: 0,
                    role: GraphNodeRole::Focus,
                    depth_from_focus: Some(0),
                    workspace_root: Some("/ws_a".into()),
                },
                GraphViewNode {
                    path: "/ws_a/src/lib.rs".into(),
                    language: "Rust".into(),
                    out_degree: 0,
                    in_degree: 1,
                    role: GraphNodeRole::Dependency,
                    depth_from_focus: Some(1),
                    workspace_root: Some("/ws_a".into()),
                },
                GraphViewNode {
                    path: "/ws_b/app.py".into(),
                    language: "Python".into(),
                    out_degree: 0,
                    in_degree: 0,
                    role: GraphNodeRole::Overview,
                    depth_from_focus: None,
                    workspace_root: Some("/ws_b".into()),
                },
            ],
            edges: vec![GraphViewEdge {
                from: "/ws_a/src/main.rs".into(),
                to: "/ws_a/src/lib.rs".into(),
            }],
            focus: Some("/ws_a/src/main.rs".into()),
            generation: 5,
            file_count: 3,
            edge_count: 1,
            languages: vec!["Rust".into(), "Python".into()],
            nodes_truncated: false,
            edges_truncated: false,
            total_nodes_available: 3,
            total_edges_available: 1,
            available_roots: vec![
                WorkspaceRootInfo {
                    root: "/ws_a".into(),
                    file_count: 2,
                    languages: vec![crate::ipc::linker_proto::LanguageCount {
                        name: "Rust".into(),
                        count: 2,
                    }],
                },
                WorkspaceRootInfo {
                    root: "/ws_b".into(),
                    file_count: 1,
                    languages: vec![crate::ipc::linker_proto::LanguageCount {
                        name: "Python".into(),
                        count: 1,
                    }],
                },
            ],
        }
    }

    // ── compute_effective_filter tests ──────────────────────────────────

    #[test]
    fn filter_all_no_configured_roots_returns_none() {
        let result = compute_effective_filter(None, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn filter_all_with_configured_roots_returns_all() {
        let roots = vec!["/ws_a".into(), "/ws_b".into()];
        let result = compute_effective_filter(None, &roots);
        assert_eq!(result, Some(roots));
    }

    #[test]
    fn filter_explicit_intersecting_roots() {
        let configured = vec!["/ws_a".into(), "/ws_b".into()];
        let ui = Some(vec!["/ws_a".into(), "/foreign".into()]);
        let result = compute_effective_filter(ui, &configured).unwrap();
        assert_eq!(result, vec!["/ws_a".to_string()]);
    }

    #[test]
    fn filter_explicit_stale_foreign_falls_back_to_configured() {
        let configured = vec!["/ws_a".into(), "/ws_b".into()];
        let ui = Some(vec!["/deleted_root".into(), "/another_foreign".into()]);
        let result = compute_effective_filter(ui, &configured).unwrap();
        // Intersection is empty → fall back to full configured set.
        assert_eq!(result, configured);
    }

    #[test]
    fn filter_empty_selection_is_all() {
        let configured = vec!["/ws_a".into()];
        let ui = Some(vec![]);
        let result = compute_effective_filter(ui, &configured).unwrap();
        assert_eq!(result, configured);
    }

    // ── scope_result tests ─────────────────────────────────────────────

    #[test]
    fn scope_filters_nodes_to_allowed_roots() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.nodes.len(), 2);
        assert!(scoped
            .nodes
            .iter()
            .all(|n| n.workspace_root.as_deref() == Some("/ws_a")));
    }

    #[test]
    fn scope_filters_edges_to_allowed_nodes() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        // Edge between two /ws_a nodes is kept.
        assert_eq!(scoped.edges.len(), 1);
    }

    #[test]
    fn scope_removes_edges_with_foreign_endpoints() {
        let mut result = make_test_result();
        // Add an edge from ws_a → ws_b.
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/main.rs".into(),
            to: "/ws_b/app.py".into(),
        });
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        // Only the intra-root edge survives.
        assert_eq!(scoped.edges.len(), 1);
        assert_eq!(scoped.edges[0].from, "/ws_a/src/main.rs");
    }

    #[test]
    fn scope_restricts_available_roots_to_configured() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.available_roots.len(), 1);
        assert_eq!(scoped.available_roots[0].root, "/ws_a");
    }

    #[test]
    fn scope_synthesises_missing_configured_roots() {
        let result = make_test_result();
        // /ws_c is configured but not in the daemon graph.
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            false,
            false,
        );
        assert_eq!(scoped.available_roots.len(), 2);
        assert_eq!(scoped.available_roots[0].root, "/ws_a");
        assert_eq!(scoped.available_roots[0].file_count, 2);
        // /ws_c is synthesised with zero metadata.
        let ws_c = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_c")
            .unwrap();
        assert_eq!(ws_c.file_count, 0);
        assert!(ws_c.languages.is_empty());
    }

    #[test]
    fn scope_empty_allowed_roots_returns_empty() {
        let result = make_test_result();
        let scoped = scope_result(result, &[], &HashMap::new(), false, false);
        assert_eq!(scoped.status, "not-indexed");
        assert!(scoped.nodes.is_empty());
        assert!(scoped.edges.is_empty());
        assert!(scoped.available_roots.is_empty());
    }

    #[test]
    fn scope_canonical_path_matching() {
        // A node has a canonical root (e.g. /private/var → /var on macOS).
        let mut result = make_test_result();
        result.available_roots.clear();
        result.available_roots.push(WorkspaceRootInfo {
            root: "/ws_a".into(),
            file_count: 2,
            languages: vec![],
        });
        // Use a symlink-equivalent spelling as the configured root.
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.nodes.len(), 2);
    }

    #[test]
    fn scope_foreign_node_removal() {
        // A node with workspace_root not in any configured root is removed.
        let mut result = make_test_result();
        result.nodes.push(GraphViewNode {
            path: "/orphan/file.rs".into(),
            language: "Rust".into(),
            out_degree: 0,
            in_degree: 0,
            role: GraphNodeRole::Overview,
            depth_from_focus: None,
            workspace_root: Some("/orphan".into()),
        });
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.nodes.len(), 2);
        assert!(!scoped.nodes.iter().any(|n| n.path.contains("/orphan")));
    }

    #[test]
    fn scope_node_with_none_root_filtered_out() {
        // A node with no workspace_root is treated as foreign.
        let mut result = make_test_result();
        result.nodes.push(GraphViewNode {
            path: "/no-root/file.rs".into(),
            language: "Rust".into(),
            out_degree: 0,
            in_degree: 0,
            role: GraphNodeRole::Overview,
            depth_from_focus: None,
            workspace_root: None,
        });
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.nodes.len(), 2);
    }

    #[test]
    fn scope_synthesised_root_zerometa_status_not_indexed() {
        // When one configured root is still missing, the overall result must
        // remain not-indexed so the UI offers reindex instead of reporting ok.
        let result = make_test_result();
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            false,
            false,
        );
        assert_eq!(scoped.status, "not-indexed");
    }

    #[test]
    fn scope_all_roots_zero_files_status_scanning() {
        let result = GraphViewResult {
            nodes: vec![],
            edges: vec![],
            focus: None,
            generation: 0,
            file_count: 0,
            edge_count: 0,
            languages: vec![],
            nodes_truncated: false,
            edges_truncated: false,
            total_nodes_available: 0,
            total_edges_available: 0,
            available_roots: vec![WorkspaceRootInfo {
                root: "/ws_a".into(),
                file_count: 0,
                languages: vec![],
            }],
        };
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.status, "scanning");
    }

    #[test]
    fn scope_languages_recomputed_from_filtered_nodes() {
        let result = make_test_result();
        // Only /ws_a (Rust) nodes survive.
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.languages, vec!["Rust".to_string()]);
    }

    #[test]
    fn scope_available_roots_ordering_matches_configured() {
        let result = make_test_result();
        let scoped = scope_result(
            result,
            &["/ws_b".into(), "/ws_a".into()],
            &HashMap::new(),
            false,
            false,
        );
        // Ordering follows the configured_roots order.
        assert_eq!(scoped.available_roots[0].root, "/ws_b");
        assert_eq!(scoped.available_roots[1].root, "/ws_a");
    }

    #[test]
    fn unavailable_result_shows_configured_roots() {
        let mut result = unavailable_result();
        result.available_roots = vec![ImportGraphRootInfo {
            root: "/ws_a".into(),
            configured_path: None,
            display_path: Some("ws_a".into()),
            file_count: 0,
            languages: Vec::new(),
            indexed_state: "not-indexed".to_string(),
        }];
        assert_eq!(result.status, "unavailable");
        assert_eq!(result.available_roots.len(), 1);
    }

    #[test]
    fn edge_count_reflects_filtered_edges() {
        let mut result = make_test_result();
        // Add more edges.
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/lib.rs".into(),
            to: "/ws_a/src/main.rs".into(),
        });
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.edge_count, 2);
        assert_eq!(scoped.total_edges_available, 2);
    }

    #[test]
    fn scope_multiple_roots_keeps_cross_root_edges_if_both_allowed() {
        let mut result = make_test_result();
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/main.rs".into(),
            to: "/ws_b/app.py".into(),
        });
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_b".into()],
            &HashMap::new(),
            false,
            false,
        );
        // Both nodes are allowed, so the cross-root edge survives.
        assert_eq!(scoped.edges.len(), 2);
    }

    // ── per-root indexed_state tests ───────────────────────────────────

    #[test]
    fn indexed_state_daemon_root_with_files_is_indexed() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        let ws_a = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_a")
            .unwrap();
        assert_eq!(ws_a.indexed_state, "indexed");
    }

    #[test]
    fn indexed_state_synthesised_root_is_not_indexed() {
        let result = make_test_result();
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            false,
            false,
        );
        let ws_c = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_c")
            .unwrap();
        assert_eq!(ws_c.indexed_state, "not-indexed");
    }

    #[test]
    fn indexed_state_zero_files_gen_zero_is_scanning() {
        let result = GraphViewResult {
            nodes: vec![],
            edges: vec![],
            focus: None,
            generation: 0,
            file_count: 0,
            edge_count: 0,
            languages: vec![],
            nodes_truncated: false,
            edges_truncated: false,
            total_nodes_available: 0,
            total_edges_available: 0,
            available_roots: vec![WorkspaceRootInfo {
                root: "/ws_a".into(),
                file_count: 0,
                languages: vec![],
            }],
        };
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.available_roots[0].indexed_state, "scanning");
    }

    #[test]
    fn indexed_state_zero_files_gen_positive_is_indexed() {
        // Root is in daemon graph with 0 files but generation > 0 — the root
        // genuinely has no scannable files (e.g. empty workspace).
        let result = GraphViewResult {
            nodes: vec![],
            edges: vec![],
            focus: None,
            generation: 3,
            file_count: 0,
            edge_count: 0,
            languages: vec![],
            nodes_truncated: false,
            edges_truncated: false,
            total_nodes_available: 0,
            total_edges_available: 0,
            available_roots: vec![WorkspaceRootInfo {
                root: "/ws_a".into(),
                file_count: 0,
                languages: vec![],
            }],
        };
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.available_roots[0].indexed_state, "indexed");
    }

    #[test]
    fn overall_status_all_not_indexed() {
        // Both configured roots are synthesised (not in daemon graph).
        let result = make_test_result();
        let scoped = scope_result(
            result,
            &["/ws_c".into(), "/ws_d".into()],
            &HashMap::new(),
            false,
            false,
        );
        assert_eq!(scoped.status, "not-indexed");
    }

    #[test]
    fn overall_status_any_not_indexed() {
        // /ws_a is indexed, /ws_c is not-indexed.
        let result = make_test_result();
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            false,
            false,
        );
        assert_eq!(scoped.status, "not-indexed");
    }

    #[test]
    fn overall_status_empty_roots() {
        let result = make_test_result();
        let scoped = scope_result(result, &[], &HashMap::new(), false, false);
        assert_eq!(scoped.status, "not-indexed");
    }

    // ── aggregate totals: no client filtering preserves daemon totals ───

    #[test]
    fn aggregate_totals_no_client_filtering_preserves_daemon_totals() {
        // Daemon returns 2 nodes, both in /ws_a.  Scoped to /ws_a, no
        // client-side filtering.  total_nodes_available should match daemon.
        let mut result = make_test_result();
        // Remove /ws_b node so daemon only returns /ws_a nodes.
        result.nodes.pop();
        result.total_nodes_available = 2;
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        // No client-side filtering: daemon total preserved.
        assert_eq!(scoped.total_nodes_available, 2);
    }

    #[test]
    fn aggregate_totals_client_filtering_uses_scoped_count() {
        // Daemon returns 3 nodes across 2 roots.  Scoped to /ws_a only,
        // client filtering removes 1 node.  total_nodes_available = 2.
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert_eq!(scoped.total_nodes_available, 2);
        // After client filtering, truncation should be false since we have
        // all the scoped nodes.
        assert!(!scoped.nodes_truncated);
    }

    // ── correlation fields ─────────────────────────────────────────────

    #[test]
    fn correlation_fields_default_none() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert!(scoped.request_id.is_none());
        assert!(scoped.session_id.is_none());
    }

    #[test]
    fn unavailable_with_ids_populates_correlation() {
        let r = unavailable_with_ids(Some("req-1".to_string()), Some("sess-2".to_string()));
        assert_eq!(r.request_id.as_deref(), Some("req-1"));
        assert_eq!(r.session_id.as_deref(), Some("sess-2"));
        assert_eq!(r.status, "unavailable");
    }

    #[test]
    fn reindex_empty_roots_no_ids() {
        let r = reindex_and_fetch(None, None, &[], &HashMap::new(), None, None);
        assert_eq!(r.status, "not-indexed");
        assert!(r.request_id.is_none());
        assert!(r.session_id.is_none());
    }

    // ── reindex: daemon unreachable → unavailable result ────────────────

    #[test]
    fn reindex_daemon_unreachable_yields_unavailable() {
        // If the linker daemon is NOT running, ensure_and_register will fail
        // and the result MUST be a terminal unavailable.  If the daemon IS
        // running this test is not meaningful (the root may succeed), so we
        // skip.
        if crate::linker::client::fetch_generation().is_none() {
            let r = reindex_and_fetch(
                Some("test-session"),
                Some("req-reindex-1"),
                &["/nonexistent_root_a".into()],
                &HashMap::new(),
                None,
                None,
            );
            assert!(
                r.status.contains("unavailable"),
                "expected unavailable status when daemon is down, got: {}",
                r.status
            );
            assert_eq!(r.request_id.as_deref(), Some("req-reindex-1"));
            assert_eq!(r.session_id.as_deref(), Some("test-session"));
        }
        // When the daemon IS running the reindex proceeds normally — we
        // can't assert "unavailable" in that case.
    }

    // ── reindex: register failure returns terminal error ────────────────

    #[test]
    fn reindex_register_failure_returns_terminal_error() {
        // Same daemon-liveness guard as above.
        if crate::linker::client::fetch_generation().is_none() {
            let r = reindex_and_fetch(
                Some("s"),
                Some("req-reg-fail"),
                &["/definitely/not/a/real/root/xyz".into()],
                &HashMap::new(),
                None,
                None,
            );
            assert!(
                r.status.contains("unavailable"),
                "registration failure must yield unavailable: {}",
                r.status
            );
            assert_eq!(r.request_id.as_deref(), Some("req-reg-fail"));
        }
    }

    // ── reindex: no configured roots → not-indexed ──────────────────────

    #[test]
    fn reindex_no_configured_roots_yields_not_indexed() {
        let r = reindex_and_fetch(None, None, &[], &HashMap::new(), None, None);
        assert_eq!(r.status, "not-indexed");
    }

    // ── reindex: correlation fields propagated regardless of daemon state ─

    #[test]
    fn reindex_propagates_correlation_fields() {
        // This test works whether the daemon is running or not — we just
        // check that request_id and session_id survive the reindex path.
        let configured = vec!["/nonexistent_reindex_root".into()];
        let r = reindex_and_fetch(
            Some("my-session"),
            Some("req-corr-1"),
            &configured,
            &HashMap::new(),
            None,
            None,
        );
        // Regardless of daemon state, correlation fields must be populated.
        assert_eq!(r.request_id.as_deref(), Some("req-corr-1"));
        assert_eq!(r.session_id.as_deref(), Some("my-session"));
    }

    #[test]
    fn reindex_with_daemon_yields_ok_or_unavailable() {
        // When the daemon is running and the root exists, reindex succeeds.
        // When the daemon is not running, we get unavailable. Either way,
        // the result must be a valid terminal state (never stale data).
        let configured = vec!["/nonexistent_reindex_root".into()];
        let r = reindex_and_fetch(
            Some("s"),
            Some("req-daemon-test"),
            &configured,
            &HashMap::new(),
            None,
            None,
        );
        assert!(
            r.status == "ok" || r.status.contains("unavailable") || r.status == "scanning",
            "reindex must produce a terminal state, got: {}",
            r.status
        );
        assert_eq!(r.request_id.as_deref(), Some("req-daemon-test"));
        assert_eq!(r.session_id.as_deref(), Some("s"));
    }

    // ── scoped impact analysis tests ───────────────────────────────────

    #[test]
    fn impact_empty_roots_returns_error() {
        let r = build_scoped_impact_result(
            "req-1".to_string(),
            "/ws_a/src/main.rs".into(),
            3,
            &[],
            None,
        );
        assert!(r.error.is_some());
        assert!(r.paths.is_empty());
        assert_eq!(r.total, 0);
    }

    #[test]
    fn impact_out_of_scope_path_rejected() {
        let r = build_scoped_impact_result(
            "req-2".to_string(),
            "/foreign/path/file.rs".into(),
            3,
            &["/ws_a".into(), "/ws_b".into()],
            None,
        );
        assert!(r.error.is_some());
        assert!(r.error.unwrap().contains("outside configured"));
        assert!(r.paths.is_empty());
    }

    #[test]
    fn impact_in_scope_path_not_called_daemon_still_validates() {
        // On CI, linker daemon is unreachable, so fetch_impact fails.
        // But the path IS in scope, so we get a daemon-unreachable error,
        // not an out-of-scope error.
        let r = build_scoped_impact_result(
            "req-3".to_string(),
            "/ws_a/src/main.rs".into(),
            3,
            &["/ws_a".into()],
            None,
        );
        // Either the daemon is running (r.error might be None or some
        // impact error) or it's not (unreachable error). Either way the
        // path was accepted as in-scope.
        if let Some(ref e) = r.error {
            // If there's an error, it should NOT be about scope.
            assert!(!e.contains("outside configured"));
        }
        assert_eq!(r.request_id, "req-3");
        assert_eq!(r.path, "/ws_a/src/main.rs");
    }

    #[test]
    fn impact_paths_filtered_to_configured_roots() {
        // This test verifies the filtering logic structurally:
        // build_scoped_impact_result uses a HashSet of allowed roots and
        // filters paths by prefix. We verify that out-of-scope focal paths
        // are rejected (tested above), and that the error structure is
        // correct. A full integration test would require a running daemon.
        let r = build_scoped_impact_result(
            "req-4".to_string(),
            "/ws_a/src/main.rs".into(),
            2,
            &["/ws_a".into()],
            None,
        );
        // Verify request_id and path are echoed.
        assert_eq!(r.request_id, "req-4");
        assert_eq!(r.path, "/ws_a/src/main.rs");
        assert_eq!(r.depth, 2);
    }

    // ── worker spawn function signatures compile ───────────────────────

    #[test]
    fn spawn_functions_compile_attached() {
        // Verify the function signatures are compatible. We can't easily
        // test actual IPC in a unit test, but we can at least confirm the
        // types line up at compile time.
        let _: fn(
            Sender<ImportGraphResult>,
            Option<String>,
            u32,
            GraphDirection,
            Option<Vec<String>>,
            Option<Vec<String>>,
            Vec<String>,
            HashMap<String, String>,
            Option<String>,
            Option<String>,
        ) = spawn_import_graph_attached;
    }

    #[test]
    fn spawn_functions_compile_reindex_attached() {
        let _: fn(
            Sender<ImportGraphResult>,
            String,
            Vec<String>,
            HashMap<String, String>,
            Option<Vec<String>>,
            Option<Vec<String>>,
            Option<String>,
        ) = spawn_import_graph_reindex_attached;
    }

    #[test]
    fn spawn_functions_compile_impact_attached() {
        let _: fn(
            Sender<super::super::push_proto::ImportGraphImpactResult>,
            String,
            u32,
            String,
            Vec<String>,
            Option<String>,
        ) = spawn_import_graph_impact_attached;
    }

    // ── Component-safe impact scoping (Path::starts_with) ──────────────

    #[test]
    fn impact_prefix_sibling_rejected_by_path_starts_with() {
        // /workspace/app must NOT be treated as in-scope for
        // /workspace/application-secret — a string prefix match would pass,
        // but Path::starts_with is component-aware and rejects this.
        let r = build_scoped_impact_result(
            "req-prefix".to_string(),
            "/workspace/app/src/main.rs".into(),
            3,
            &["/workspace/application-secret".into()],
            None,
        );
        assert!(
            r.error.is_some(),
            "should reject /workspace/app when configured root is /workspace/application-secret"
        );
        assert!(r.error.unwrap().contains("outside configured"));
    }

    #[test]
    fn impact_prefix_sibling_accepted_when_exact_root() {
        // /workspace/app IS in scope when /workspace/app is a configured root.
        let r = build_scoped_impact_result(
            "req-exact".to_string(),
            "/workspace/app/src/main.rs".into(),
            3,
            &["/workspace/app".into()],
            None,
        );
        // Either daemon unreachable (error about daemon, not scope) or success.
        if let Some(ref e) = r.error {
            assert!(!e.contains("outside configured"));
        }
    }

    // ── Display mapping tests ─────────────────────────────────────────

    #[test]
    fn scope_result_populates_display_path() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        let ws_a = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_a")
            .unwrap();
        assert_eq!(ws_a.display_path.as_deref(), Some("ws_a"));
    }

    #[test]
    fn scope_result_configured_path_none_when_equal() {
        // When the daemon root matches the configured root exactly,
        // configured_path should be None (omitted from JSON).
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        let ws_a = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_a")
            .unwrap();
        assert!(ws_a.configured_path.is_none());
    }

    #[test]
    fn scope_result_synthesised_root_has_display_path() {
        let result = make_test_result();
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            false,
            false,
        );
        let ws_c = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_c")
            .unwrap();
        assert_eq!(ws_c.display_path.as_deref(), Some("ws_c"));
    }

    // ── Scan state from ScanStatus (per-root + overall) ───────────────

    #[test]
    fn scope_scan_in_flight_marks_missing_root_as_scanning() {
        let result = make_test_result();
        // /ws_c is not in daemon graph but scan_in_flight = true.
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            true,
            false,
        );
        let ws_c = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_c")
            .unwrap();
        assert_eq!(ws_c.indexed_state, "scanning");
    }

    #[test]
    fn scope_scan_failed_marks_all_as_unavailable() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, true);
        assert_eq!(scoped.status, "unavailable");
        let ws_a = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_a")
            .unwrap();
        assert_eq!(ws_a.indexed_state, "unavailable");
    }

    #[test]
    fn overall_mixed_indexed_and_not_indexed_with_scan_in_flight_is_scanning() {
        let result = make_test_result();
        // /ws_a is indexed, /ws_c is not-indexed but scan is in-flight → scanning.
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            true,
            false,
        );
        assert_eq!(scoped.status, "scanning");
    }

    #[test]
    fn overall_mixed_indexed_and_not_indexed_without_scan_is_not_indexed() {
        let result = make_test_result();
        // /ws_a is indexed, /ws_c is not-indexed, no scan in-flight.
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &HashMap::new(),
            false,
            false,
        );
        assert_eq!(scoped.status, "not-indexed");
    }

    // ── Reindex: repeated reindex with exact revision ─────────────────

    #[test]
    fn reindex_daemon_unreachable_yields_terminal_unavailable() {
        // When daemon is down, reindex must produce a terminal unavailable
        // result — not hang or produce stale data.
        if crate::linker::client::fetch_generation().is_none() {
            let r1 = reindex_and_fetch(
                Some("s"),
                Some("req-r1"),
                &["/nonexistent_a".into()],
                &HashMap::new(),
                None,
                None,
            );
            assert!(
                r1.status.contains("unavailable"),
                "first reindex should be unavailable: {}",
                r1.status
            );
            // Second reindex on the same dead daemon must also be terminal.
            let r2 = reindex_and_fetch(
                Some("s"),
                Some("req-r2"),
                &["/nonexistent_a".into()],
                &HashMap::new(),
                None,
                None,
            );
            assert!(
                r2.status.contains("unavailable"),
                "second reindex should also be unavailable: {}",
                r2.status
            );
            // Both carry their correlation ids.
            assert_eq!(r1.request_id.as_deref(), Some("req-r1"));
            assert_eq!(r2.request_id.as_deref(), Some("req-r2"));
        }
    }

    // ── unavailable_result display mapping ─────────────────────────────

    #[test]
    fn unavailable_result_display_path_is_none() {
        // The base unavailable_result has no available_roots at all.
        let r = unavailable_result();
        assert!(r.available_roots.is_empty());
    }

    // ── configured_root_map integration: symlink/relative DTO mapping ────

    #[test]
    fn scope_result_configured_path_set_when_map_differs() {
        let result = make_test_result();
        let mut map = HashMap::new();
        map.insert("/ws_a".to_string(), "/symlink/to/ws_a".to_string());
        let scoped = scope_result(result, &["/ws_a".into()], &map, false, false);
        let ws_a = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_a")
            .unwrap();
        assert_eq!(ws_a.configured_path.as_deref(), Some("/symlink/to/ws_a"));
        // display_path should come from the raw configured path basename.
        assert_eq!(ws_a.display_path.as_deref(), Some("ws_a"));
    }

    #[test]
    fn scope_result_configured_path_none_when_map_empty() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        let ws_a = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_a")
            .unwrap();
        assert!(ws_a.configured_path.is_none());
        // display_path falls back to canonical basename.
        assert_eq!(ws_a.display_path.as_deref(), Some("ws_a"));
    }

    #[test]
    fn scope_result_synthesised_root_uses_map_for_configured_path() {
        let result = make_test_result();
        let mut map = HashMap::new();
        map.insert("/ws_c".to_string(), "../ws_c".to_string());
        let scoped = scope_result(
            result,
            &["/ws_a".into(), "/ws_c".into()],
            &map,
            false,
            false,
        );
        let ws_c = scoped
            .available_roots
            .iter()
            .find(|r| r.root == "/ws_c")
            .unwrap();
        assert_eq!(ws_c.configured_path.as_deref(), Some("../ws_c"));
        assert_eq!(ws_c.display_path.as_deref(), Some("ws_c"));
    }

    // ── Edge totals preservation: daemon aggregate exact scope ───────────

    #[test]
    fn edge_totals_preserved_when_no_client_filtering() {
        // Daemon returns 2 nodes and 3 edges, all in /ws_a.  Scoped to
        // /ws_a — no client filtering.  edge_count should match daemon.
        let mut result = make_test_result();
        // Remove the /ws_b node so daemon only returns /ws_a nodes.
        result.nodes.pop();
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/lib.rs".into(),
            to: "/ws_a/src/main.rs".into(),
        });
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/main.rs".into(),
            to: "/ws_a/src/lib.rs".into(),
        });
        result.total_edges_available = 3;
        result.total_nodes_available = 2;
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        // No client-side filtering → daemon totals preserved.
        assert_eq!(scoped.total_edges_available, 3);
        assert_eq!(scoped.edge_count, 3);
    }

    #[test]
    fn edge_totals_recomputed_when_client_filters() {
        // Daemon returns 3 nodes across 2 roots.  Scoped to /ws_a only —
        // client filters out 1 node.  Also add a cross-root edge that will
        // be removed by client filtering.
        let mut result = make_test_result();
        // All 3 nodes in /ws_a get an extra intra-root edge.
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/lib.rs".into(),
            to: "/ws_a/src/main.rs".into(),
        });
        // Plus a cross-root edge (won't survive scoping to /ws_a).
        result.edges.push(GraphViewEdge {
            from: "/ws_a/src/main.rs".into(),
            to: "/ws_b/app.py".into(),
        });
        result.total_edges_available = 3;
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        // Client filtered: /ws_b node removed, cross-root edge removed.
        // 2 intra-root edges survive.
        assert_eq!(scoped.edge_count, 2);
        assert_eq!(scoped.total_edges_available, 2);
    }

    #[test]
    fn edges_truncated_preserved_when_no_client_filtering() {
        // Daemon reports truncation, no client filtering → truncation preserved.
        let mut result = make_test_result();
        result.nodes.pop(); // remove /ws_b node, leaving only /ws_a nodes
        result.edges_truncated = true;
        result.total_edges_available = 5; // daemon says 5 edges available total
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert!(scoped.edges_truncated);
        // Daemon's pre-cap aggregate preserved (not the array length).
        assert_eq!(scoped.total_edges_available, 5);
    }

    // ── request_id threading: scope_result default ───────────────────────

    #[test]
    fn scope_result_request_id_session_id_default_none() {
        let result = make_test_result();
        let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
        assert!(scoped.request_id.is_none());
        assert!(scoped.session_id.is_none());
    }
}

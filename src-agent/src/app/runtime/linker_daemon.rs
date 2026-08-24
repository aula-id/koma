//! The GLOBAL linker daemon (`koma --linker-daemon`).
//!
//! A singleton headless process that owns an in-memory import graph for every
//! registered project. Session clients query it via IPC for dependency info.
//!
//! Lifecycle: similar to the OAuth daemon — bind socket, serve requests, idle
//! self-reap when no registered projects / no clients.
//!
//! A file watcher monitors workspace roots for source-file changes and
//! incrementally updates the import graph (create/modify → re-scan, delete →
//! remove node).

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::linker_proto::{LinkerQuery, LinkerRequest, LinkerResponse};
use crate::linker::graph::ImportGraph;
use crate::model::store;

use super::signals::install_daemon_signals;

/// How long a single `accept` waits before we re-check the `shutting_down` flag.
const ACCEPT_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Maximum number of results returned per query to avoid huge payloads.
const QUERY_RESULT_CAP: usize = 200;

/// Initial grace period before the reaper starts checking.
const REAPER_INITIAL_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
/// Poll interval for the reaper.
const REAPER_POLL: std::time::Duration = std::time::Duration::from_secs(15);
/// Number of consecutive empty scans before exit.
const REAPER_EMPTY_STREAK_TO_EXIT: u32 = 2;

/// Newest-wins full-scan request coalesced by [`request_scan`].
struct ScanRequest {
    roots: Vec<PathBuf>,
}

/// Coordinates single-flight full scans: at most one worker, cooperative cancel,
/// and newest-wins pending coalesce.
///
/// `desired_revision` cancels in-flight workers. `applied_revision` tracks the
/// last published full-scan revision. Client-visible graph generation lives in
/// `published_generation` / `ImportGraph::generation`.
struct ScanCoordinator {
    /// Monotonically-increasing counter, bumped on each desired scan.
    desired_revision: u64,
    /// The revision whose results are currently published in the graph.
    applied_revision: u64,
    /// `Some(rev)` while a scan worker for `rev` is running.
    in_flight: Option<u64>,
    /// Cooperative cancel flag for the current worker (shared with scan).
    cancel: Arc<AtomicBool>,
    /// Newest pending full-scan request (at most one).
    pending: Option<ScanRequest>,
    /// How many scan worker threads have been spawned (test observability).
    spawn_count: u64,
}

/// Shared daemon state: the import graph plus per-client root tracking, the
/// file watcher, and the project index.
struct DaemonState {
    graph: RwLock<ImportGraph>,
    /// client_id → set of registered workspace root paths.
    clients: RwLock<HashMap<String, HashSet<PathBuf>>>,
    /// root → refcount (how many clients hold this root).
    root_refs: RwLock<HashMap<PathBuf, u32>>,
    /// True while a scan is running on a background thread.
    scanning: std::sync::atomic::AtomicBool,
    /// The debounced file watcher (kept alive while the daemon runs).
    /// Dropped on shutdown or when all roots are unregistered.
    watcher: Mutex<Option<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>>>,
    /// Channel receiver for debounced file-change batches from the watcher.
    /// Wrapped in Mutex so it can be taken out once and polled in a dedicated
    /// thread.
    watcher_rx: Mutex<Option<std::sync::mpsc::Receiver<Vec<PathBuf>>>>,
    /// The set of roots currently being watched (so we can detect changes).
    watched_roots: RwLock<Vec<PathBuf>>,
    /// Phase 2: the project index tracking known files and workspace ownership.
    project_index: RwLock<crate::linker::project::ProjectIndex>,

    // ── Atomic reconciliation ──────────────────────────────────────────
    /// Serializes entire RegisterWorkspaces / Unregister operations so that
    /// clients, refcounts, watcher updates, and scan scheduling are always
    /// consistent.  Without this, two concurrent requests from different
    /// sessions could interleave between the clients write and the
    /// root_refs write, leaving refcounts corrupt.
    operation_lock: Mutex<()>,

    /// Serializes graph + project_index pair mutations from the scan-thread
    /// publication and the watcher event-processing loop.  Ensures that the
    /// pair is always swapped/mutated atomically relative to each other and
    /// to new scan scheduling.  Ordering: scan_coordinator → publication_lock
    /// (scan thread) or publication_lock alone (watcher).  Never held by
    /// RegisterWorkspaces (which schedules scans, not data mutation).
    publication_lock: Mutex<()>,

    /// Scan versioning: prevents stale slow scans from overwriting a
    /// graph that was already updated by a newer scan.
    scan_coordinator: Mutex<ScanCoordinator>,

    /// Per-session monotonic registration revision.  Registrations whose
    /// revision is older than the last accepted one are silently ignored,
    /// preventing quick successive settings saves from registering stale
    /// roots out of order.
    session_revisions: RwLock<HashMap<String, u64>>,

    /// Monotonically-increasing graph generation counter, owned exclusively
    /// by the daemon.  Set at publication time (scan or watcher batch) so
    /// the published generation never moves backward.
    published_generation: std::sync::atomic::AtomicU64,
}

impl DaemonState {
    fn new() -> Self {
        Self {
            graph: RwLock::new(ImportGraph::new()),
            clients: RwLock::new(HashMap::new()),
            root_refs: RwLock::new(HashMap::new()),
            scanning: std::sync::atomic::AtomicBool::new(false),
            watcher: Mutex::new(None),
            watcher_rx: Mutex::new(None),
            watched_roots: RwLock::new(Vec::new()),
            project_index: RwLock::new(crate::linker::project::ProjectIndex::new()),
            operation_lock: Mutex::new(()),
            publication_lock: Mutex::new(()),
            scan_coordinator: Mutex::new(ScanCoordinator {
                desired_revision: 0,
                applied_revision: 0,
                in_flight: None,
                cancel: Arc::new(AtomicBool::new(false)),
                pending: None,
                spawn_count: 0,
            }),
            session_revisions: RwLock::new(HashMap::new()),
            published_generation: AtomicU64::new(0),
        }
    }
}

/// Headless entry point: run the GLOBAL linker daemon event loop.
pub fn run_linker_daemon(_opts: crate::cli::Opts) -> Result<()> {
    // Ignore SIGPIPE.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Ensure config dirs exist.
    store::ensure_dirs()?;

    // Own tokio runtime.
    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Install signal handling.
    let shutting_down = install_daemon_signals(&handle);

    // Bind the singleton socket (bind = liveness oracle).
    let sock_path = store::linker_daemon_sock_path()?;
    let listener = {
        let _enter = handle.enter();
        crate::ipc::server::bind(&sock_path)?
    };

    // Write advisory pidfile.
    let pid_path = store::linker_daemon_pid_path()?;
    let _ = store::write_linker_daemon_pid();

    // Shared state.
    let state = Arc::new(DaemonState::new());

    // Spawn the idle reaper.
    {
        let flag = Arc::clone(&shutting_down);
        let state = Arc::clone(&state);
        handle.spawn(reaper_loop(flag, state));
    }

    // Accept loop.
    handle.block_on(accept_loop(listener, &shutting_down, &state));

    // Teardown — explicitly stop watcher.
    if let Ok(mut w) = state.watcher.lock() {
        *w = None;
    }
    drop(rt);
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(pid_path);

    Ok(())
}

/// Accept connections until `shutting_down` is set, spawning a per-connection
/// task for each. Runs on the tokio runtime (async socket I/O).
async fn accept_loop(
    listener: crate::ipc::IpcListener,
    shutting_down: &Arc<std::sync::atomic::AtomicBool>,
    state: &Arc<DaemonState>,
) {
    use std::sync::atomic::Ordering;

    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return;
        }
        match tokio::time::timeout(ACCEPT_POLL, listener.accept()).await {
            Ok(Ok((stream, _addr))) => {
                let flag = Arc::clone(shutting_down);
                let state = Arc::clone(state);
                tokio::spawn(async move {
                    connection_loop(stream, flag, &state).await;
                });
            }
            Err(_elapsed) => {}
            Ok(Err(_e)) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

/// Serve one client connection: read a [`LinkerRequest`] frame, produce its
/// [`LinkerResponse`], write it back, and repeat until the peer closes or a
/// read/decode/write error ends the connection.
async fn connection_loop(
    mut stream: crate::ipc::IpcStream,
    shutting_down: Arc<std::sync::atomic::AtomicBool>,
    state: &Arc<DaemonState>,
) {
    let mut reader = FrameReader::new();
    loop {
        let bytes = match read_frame_from(&mut stream, &mut reader).await {
            Ok(b) => b,
            Err(_) => return,
        };
        let req: LinkerRequest = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                let _ = respond(
                    &mut stream,
                    &LinkerResponse::Error(format!("bad request: {e}")),
                )
                .await;
                return;
            }
        };

        let resp = handle_request(req, &shutting_down, state);
        if respond(&mut stream, &resp).await.is_err() {
            return;
        }
    }
}

/// Serialise + frame-write one [`LinkerResponse`].
async fn respond(stream: &mut crate::ipc::IpcStream, resp: &LinkerResponse) -> std::io::Result<()> {
    let bytes = match serde_json::to_vec(resp) {
        Ok(b) => b,
        Err(e) => serde_json::to_vec(&LinkerResponse::Error(format!("encode failed: {e}")))
            .unwrap_or_else(|_| b"{\"Error\":\"encode failed\"}".to_vec()),
    };
    write_frame_to(stream, &bytes).await
}

/// Produce the [`LinkerResponse`] for one [`LinkerRequest`].
fn handle_request(
    req: LinkerRequest,
    shutting_down: &std::sync::atomic::AtomicBool,
    state: &Arc<DaemonState>,
) -> LinkerResponse {
    match req {
        LinkerRequest::Fingerprint => LinkerResponse::Fingerprint(store::build_fingerprint()),
        LinkerRequest::Shutdown => {
            shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
            LinkerResponse::Ack
        }
        LinkerRequest::RegisterWorkspaces {
            roots,
            session_id,
            registration_revision,
        } => {
            // ── Serialize the entire reconciliation under operation_lock ──
            let _op = state
                .operation_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            // ── Reject stale registration ────────────────────────────────
            if let Some(rev) = registration_revision {
                let mut revs = state
                    .session_revisions
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(&known) = revs.get(&session_id) {
                    if rev < known {
                        let gen = state
                            .graph
                            .read()
                            .unwrap_or_else(|e| e.into_inner())
                            .generation;
                        return LinkerResponse::Registered {
                            status: if state.scanning.load(Ordering::SeqCst) {
                                crate::ipc::linker_proto::ScanStatus::Scanning
                            } else {
                                crate::ipc::linker_proto::ScanStatus::Ready
                            },
                            generation: gen,
                        };
                    }
                }
                revs.insert(session_id.clone(), rev);
            }

            let new_set: HashSet<PathBuf> = roots.iter().map(PathBuf::from).collect();

            // ── Reconcile this session's root set ──────────────────────────
            let mut roots_added = Vec::new();
            let mut roots_removed = Vec::new();
            {
                let mut clients = state.clients.write().unwrap_or_else(|e| e.into_inner());
                let old_set = clients.entry(session_id.clone()).or_default();
                // Roots in new_set but not in old_set → increment.
                for root in &new_set {
                    if !old_set.contains(root) {
                        roots_added.push(root.clone());
                    }
                }
                // Roots in old_set but not in new_set → decrement.
                for root in old_set.iter() {
                    if !new_set.contains(root) {
                        roots_removed.push(root.clone());
                    }
                }
                // Replace with the new set.  Empty registration removes the
                // client key entirely so the idle reaper can reap it.
                if new_set.is_empty() {
                    clients.remove(&session_id);
                } else {
                    *old_set = new_set;
                }
            }

            // Bump / decrement refcounts.
            {
                let mut refs = state.root_refs.write().unwrap_or_else(|e| e.into_inner());
                for root in &roots_added {
                    let count = refs.entry(root.clone()).or_insert(0);
                    *count += 1;
                }
                for root in &roots_removed {
                    if let Some(count) = refs.get_mut(root) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            refs.remove(root);
                        }
                    }
                }
            }

            // Snapshot the old watcher roots so we can detect union changes.
            let old_watched: HashSet<PathBuf> = {
                let wr = state
                    .watched_roots
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                wr.iter().cloned().collect()
            };

            // Collect all roots for the watcher.
            let all_roots = collect_all_roots(state);
            let new_watched: HashSet<PathBuf> = all_roots.iter().cloned().collect();
            maybe_update_watcher(state, &all_roots);

            // Determine if we need to scan: new globally-introduced roots or
            // roots whose last reference was removed (need to evict them from
            // the graph), or a first-time scan on a non-empty root set.
            let globally_new_roots: Vec<PathBuf> = roots_added
                .iter()
                .filter(|r| !old_watched.contains(*r))
                .cloned()
                .collect();
            let finally_dropped_roots: Vec<PathBuf> = roots_removed
                .iter()
                .filter(|r| !new_watched.contains(*r))
                .cloned()
                .collect();
            let needs_scan = !globally_new_roots.is_empty() || !finally_dropped_roots.is_empty();
            let first_scan_needed = {
                let graph = state.graph.read().unwrap_or_else(|e| e.into_inner());
                graph.generation == 0
            } && !all_roots.is_empty();

            if needs_scan || first_scan_needed {
                request_scan(state, all_roots);
            }

            let status = if state.scanning.load(Ordering::SeqCst) {
                crate::ipc::linker_proto::ScanStatus::Scanning
            } else {
                crate::ipc::linker_proto::ScanStatus::Ready
            };
            let gen = state
                .graph
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .generation;

            LinkerResponse::Registered {
                status,
                generation: gen,
            }
        }
        LinkerRequest::Unregister { session_id } => {
            // ── Serialize under operation_lock (same lock as Register) ────
            let _op = state
                .operation_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            let removed_roots;
            {
                let mut clients = state.clients.write().unwrap_or_else(|e| e.into_inner());
                removed_roots = clients.remove(&session_id);
            }

            if let Some(roots) = removed_roots {
                let mut roots_to_drop = Vec::new();
                {
                    let mut refs = state.root_refs.write().unwrap_or_else(|e| e.into_inner());
                    for root in &roots {
                        if let Some(count) = refs.get_mut(root) {
                            *count = count.saturating_sub(1);
                            if *count == 0 {
                                refs.remove(root);
                                roots_to_drop.push(root.clone());
                            }
                        }
                    }
                }

                // If all roots are empty, stop the watcher.
                let all_roots = collect_all_roots(state);
                if all_roots.is_empty() {
                    stop_watcher(state);
                } else {
                    maybe_update_watcher(state, &all_roots);
                }

                if all_roots.is_empty() {
                    // Empty union is an applied state transition: invalidate any
                    // in-flight scan before clearing so its late completion can
                    // never republish removed workspace data.
                    let mut coord = state
                        .scan_coordinator
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    coord.desired_revision = coord.desired_revision.saturating_add(1);
                    let empty_revision = coord.desired_revision;
                    coord.in_flight = None;
                    coord.pending = None;
                    coord.cancel.store(true, Ordering::SeqCst);
                    let _pub = state
                        .publication_lock
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let new_gen = state
                        .published_generation
                        .fetch_add(1, Ordering::SeqCst)
                        .saturating_add(1);
                    if let Ok(mut g) = state.graph.write() {
                        g.clear();
                        g.generation = new_gen;
                    }
                    if let Ok(mut idx) = state.project_index.write() {
                        *idx = crate::linker::project::ProjectIndex::new();
                    }
                    coord.applied_revision = empty_revision;
                    state.scanning.store(false, Ordering::SeqCst);
                } else if !roots_to_drop.is_empty() {
                    // Rescan remaining roots (dropped roots are gone from all_roots).
                    request_scan(state, all_roots);
                }
            }

            LinkerResponse::Ack
        }
        LinkerRequest::Summary => {
            let graph = state.graph.read().unwrap_or_else(|e| e.into_inner());
            let languages = graph.languages();
            let file_count = graph.file_count;
            let edge_count = graph.edge_count;
            let generation = graph.generation;

            let (ext_refs, unres_refs, amb_refs, dyn_refs) = graph.aggregate_ref_counts();

            let top_fan_in = graph.top_fan_in(5);
            let entry_points = graph.entry_points(10);

            let mut text = format!(
                "Import graph: {file_count} files, {edge_count} edges (gen {generation})\n\
                 Languages: {}\nRefs: {} external, {} unresolved, {} ambiguous, {} dynamic\n",
                languages.join(", "),
                ext_refs,
                unres_refs,
                amb_refs,
                dyn_refs,
            );

            if !top_fan_in.is_empty() {
                text.push_str("Most depended-on:\n");
                for (path, count) in &top_fan_in {
                    text.push_str(&format!("  {path} ({count} dependents)\n"));
                }
            }

            if !entry_points.is_empty() {
                text.push_str("Entry points:\n");
                for ep in &entry_points {
                    text.push_str(&format!("  {ep}\n"));
                }
            }

            // Cap summary text at 600 chars.
            if text.len() > 600 {
                text.truncate(600);
                text.push('…');
            }

            LinkerResponse::Summary {
                text,
                generation,
                file_count,
                edge_count,
                languages,
            }
        }
        LinkerRequest::Generation => {
            let gen = state
                .graph
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .generation;
            LinkerResponse::Generation(gen)
        }
        LinkerRequest::Query(query) => {
            let graph = state.graph.read().unwrap_or_else(|e| e.into_inner());
            handle_query(query, &graph, state)
        }
    }
}

/// Collect all workspace roots across all registered clients.
/// Returns a sorted, deduplicated list (deterministic ordering for scan/watcher).
fn collect_all_roots(state: &Arc<DaemonState>) -> Vec<PathBuf> {
    let clients = state.clients.read().unwrap_or_else(|e| e.into_inner());
    let mut all_roots: Vec<PathBuf> = clients.values().flatten().cloned().collect();
    all_roots.sort();
    all_roots.dedup();
    all_roots
}

/// Request a full root scan through the single-flight scheduler.
///
/// - Merges into `pending` (newest roots win).
/// - If a worker is running, sets cancel and does **not** spawn another thread.
/// - If idle, starts one worker that drains pending requests until empty.
///
/// Returns the accepted scan revision (`desired_revision` after the request).
fn request_scan(state: &Arc<DaemonState>, roots: Vec<PathBuf>) -> u64 {
    let (scan_rev, should_spawn) = {
        let mut coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        coord.desired_revision = coord.desired_revision.saturating_add(1);
        let scan_rev = coord.desired_revision;
        // Newest-wins coalesce: replace any pending request.
        coord.pending = Some(ScanRequest { roots });
        let should_spawn = coord.in_flight.is_none();
        if !should_spawn {
            // Cancel the running worker so it exits promptly; the pending
            // request will run when it finishes.
            coord.cancel.store(true, Ordering::SeqCst);
        }
        (scan_rev, should_spawn)
    };

    if should_spawn {
        start_scan_worker(state);
    } else {
        // Worker is running and will pick up pending after cancel/finish.
        state.scanning.store(true, Ordering::SeqCst);
    }

    scan_rev
}

/// Start the single scan worker if one is not already running. The worker
/// drains `pending` until empty (loop), so callers never stack threads.
fn start_scan_worker(state: &Arc<DaemonState>) {
    {
        let mut coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if coord.in_flight.is_some() {
            return;
        }
        // Reserve the in_flight slot with the current desired revision; the
        // actual work roots come from pending inside the worker loop.
        if coord.pending.is_none() {
            return;
        }
        coord.in_flight = Some(coord.desired_revision);
        coord.spawn_count = coord.spawn_count.saturating_add(1);
        state.scanning.store(true, Ordering::SeqCst);
    }

    let state_clone = Arc::clone(state);
    if std::thread::Builder::new()
        .name("linker-scan".to_string())
        .spawn(move || scan_worker_loop(state_clone))
        .is_err()
    {
        // Thread spawn failure — restore coordinator and scanning flag.
        let mut coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        coord.in_flight = None;
        state
            .scanning
            .store(coord.in_flight.is_some(), Ordering::SeqCst);
    }
}

/// Single-flight worker: take pending → scan → publish or discard → repeat.
fn scan_worker_loop(state: Arc<DaemonState>) {
    loop {
        // Take the newest pending request and bind it to a revision.
        let (scan_rev, roots, cancel) = {
            let mut coord = state
                .scan_coordinator
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let Some(req) = coord.pending.take() else {
                coord.in_flight = None;
                state.scanning.store(false, Ordering::SeqCst);
                return;
            };
            // Bind this attempt to the current desired revision so a cancel
            // that races after take still discards the result.
            let scan_rev = coord.desired_revision;
            coord.in_flight = Some(scan_rev);
            coord.cancel.store(false, Ordering::SeqCst);
            let cancel = Arc::clone(&coord.cancel);
            state.scanning.store(true, Ordering::SeqCst);
            (scan_rev, req.roots, cancel)
        };

        let outcome = crate::linker::scan::scan_roots_cancellable(&roots, Some(&cancel));

        {
            let mut coord = state
                .scan_coordinator
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            let still_current =
                coord.in_flight == Some(scan_rev) && coord.desired_revision == scan_rev;
            let cancelled = cancel.load(Ordering::SeqCst) || matches!(outcome, None);

            if still_current {
                if let Some((graph, pi)) = outcome {
                    if !cancelled {
                        let _pub = state
                            .publication_lock
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        let new_gen = state.published_generation.fetch_add(1, Ordering::SeqCst) + 1;
                        if let Ok(mut g) = state.graph.write() {
                            *g = graph;
                            g.generation = new_gen;
                        }
                        if let Ok(mut idx) = state.project_index.write() {
                            *idx = pi;
                        }
                        coord.applied_revision = scan_rev;
                    }
                }
                // Either published or cancelled-while-still-current: clear
                // in_flight only if no newer pending was queued. The loop head
                // will re-check pending.
                if coord.pending.is_none() {
                    coord.in_flight = None;
                    state.scanning.store(false, Ordering::SeqCst);
                    return;
                }
                // Keep in_flight occupied while we drain pending.
                coord.in_flight = Some(coord.desired_revision);
            } else if coord.in_flight == Some(scan_rev) {
                // Superseded (e.g. empty unregister invalidated us).
                if coord.pending.is_none() {
                    coord.in_flight = None;
                    state.scanning.store(false, Ordering::SeqCst);
                    return;
                }
                coord.in_flight = Some(coord.desired_revision);
            } else if coord.pending.is_none() {
                // Another path cleared in_flight (empty unregister). Exit.
                state.scanning.store(false, Ordering::SeqCst);
                return;
            }
            // else: pending set and/or newer in_flight — loop and drain.
        }
    }
}

/// Stop the current file watcher (if running), dropping the debouncer.
fn stop_watcher(state: &DaemonState) {
    if let Ok(mut w) = state.watcher.lock() {
        *w = None;
    }
    if let Ok(mut rx) = state.watcher_rx.lock() {
        *rx = None;
    }
    if let Ok(mut wr) = state.watched_roots.write() {
        wr.clear();
    }
}

/// Re-create the file watcher if the set of watched roots has changed.
///
/// This stops the old watcher (if any), creates a new one for the given roots,
/// and spawns a background thread to process file-change events from the
/// debounced receiver.
fn maybe_update_watcher(state: &Arc<DaemonState>, new_roots: &[PathBuf]) {
    // Check whether roots actually changed.
    let current_roots = {
        let wr = state
            .watched_roots
            .read()
            .unwrap_or_else(|e| e.into_inner());
        wr.clone()
    };

    if current_roots == new_roots {
        return; // Roots unchanged — keep existing watcher.
    }

    // Stop old watcher.
    stop_watcher(state);

    if new_roots.is_empty() {
        return;
    }

    // Create new watcher.
    match crate::linker::watch::create_watcher(new_roots) {
        Ok((debouncer, rx)) => {
            *state.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(debouncer);
            *state.watcher_rx.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);
            *state
                .watched_roots
                .write()
                .unwrap_or_else(|e| e.into_inner()) = new_roots.to_vec();

            // Spawn the watcher event-processing thread.
            let state_clone = Arc::clone(state);
            std::thread::Builder::new()
                .name("linker-watcher".to_string())
                .spawn(move || watcher_loop(state_clone))
                .ok(); // Thread spawn failure is non-fatal.
        }
        Err(e) => {
            // Watcher creation failed — log and continue without watching.
            // (The full rescan on RegisterWorkspaces still runs.)
            eprintln!("[linker-daemon] watcher setup failed: {e}");
        }
    }
}

/// Ceiling for storm coalesce: force-process a batch even if events keep
/// arriving. Quiet debounce is handled by notify-debouncer-mini (400ms).
const WATCHER_STORM_CEILING: Duration = Duration::from_millis(1500);
/// Short drain window after each recv to batch path-identity duplicates.
const WATCHER_DRAIN_IDLE: Duration = Duration::from_millis(50);

/// Background thread: read debounced file-change events and update the graph.
///
/// Runs until the receiver is disconnected (watcher dropped).
///
/// **Coordination:** Acquires `publication_lock` during graph + project_index
/// mutation so the pair is always updated atomically relative to scan-thread
/// publications. Content-only batches apply incrementally. Full rebuilds
/// (config / repair) go through [`request_scan`] single-flight — the watcher
/// never stacks scan threads.
fn watcher_loop(state: Arc<DaemonState>) {
    // Take the receiver out of the Mutex — this thread owns it exclusively.
    let rx = {
        let mut slot = state.watcher_rx.lock().unwrap_or_else(|e| e.into_inner());
        match slot.take() {
            Some(r) => r,
            None => return,
        }
    };

    // Read events in a loop. The channel closes when the debouncer is dropped.
    while let Ok(first) = rx.recv() {
        if first.is_empty() {
            continue;
        }

        // Coalesce: path-identity dedupe + storm ceiling flush.
        let mut batch: HashSet<PathBuf> = first.into_iter().collect();
        let storm_started = Instant::now();
        loop {
            // Drain anything already queued without blocking.
            while let Ok(more) = rx.try_recv() {
                batch.extend(more);
            }
            if storm_started.elapsed() >= WATCHER_STORM_CEILING {
                break;
            }
            // Brief idle wait for a quiet window after the mini debounce.
            match rx.recv_timeout(WATCHER_DRAIN_IDLE) {
                Ok(more) => {
                    batch.extend(more);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Process what we have, then exit outer loop on next recv.
                    break;
                }
            }
        }

        if batch.is_empty() {
            continue;
        }
        let paths: Vec<PathBuf> = batch.into_iter().collect();

        // Cancel any in-flight full scan so it won't overwrite incremental
        // mutations — but do NOT spawn a replacement scan on every batch.
        // Content-only apply stays incremental; full rebuilds are requested
        // only when handle_events reports a root rebuild is required.
        {
            let mut coord = state
                .scan_coordinator
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if coord.in_flight.is_some() {
                coord.desired_revision = coord.desired_revision.saturating_add(1);
                coord.cancel.store(true, Ordering::SeqCst);
            }
        }

        // Hold publication_lock during the graph + project_index mutation
        // so it is atomic with respect to scan-thread publications.
        let mut full_rebuild_roots = Vec::new();
        let mut new_dirs = Vec::new();
        {
            let _pub = state
                .publication_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let (Ok(mut graph), Ok(mut pi)) = (state.graph.write(), state.project_index.write())
            {
                let outcome = crate::linker::watch::handle_events(&paths, &mut graph, &mut pi);
                full_rebuild_roots = outcome.full_rebuild_roots;
                new_dirs = outcome.new_dirs;
            }
        }

        // Dynamically attach NonRecursive watches for newly created dirs.
        if !new_dirs.is_empty() {
            if let Ok(mut w) = state.watcher.lock() {
                if let Some(debouncer) = w.as_mut() {
                    for dir in &new_dirs {
                        let _ = crate::linker::watch::watch_new_dir(debouncer.watcher(), dir);
                    }
                }
            }
        }

        // Config / repair paths request a single-flight full scan (coalesced).
        if !full_rebuild_roots.is_empty() {
            let all_roots = collect_all_roots(&state);
            if !all_roots.is_empty() {
                // Prefer the specific roots when they are still registered;
                // otherwise fall back to the full union.
                let scoped: Vec<PathBuf> = full_rebuild_roots
                    .into_iter()
                    .filter(|r| all_roots.iter().any(|a| a == r))
                    .collect();
                let roots = if scoped.is_empty() {
                    all_roots
                } else {
                    scoped
                };
                request_scan(&state, roots);
            }
        }
    }
}

/// Dispatch a graph query and produce a response.
fn handle_query(
    query: LinkerQuery,
    graph: &ImportGraph,
    state: &Arc<DaemonState>,
) -> LinkerResponse {
    match query {
        LinkerQuery::Dependencies { path } => {
            let key = match graph.resolve_key(&path) {
                Some(k) => k,
                None => {
                    return LinkerResponse::PathList {
                        paths: vec![],
                        total: 0,
                    }
                }
            };
            let deps = graph.dependencies(key);
            let total = deps.len();
            let paths: Vec<String> = deps
                .into_iter()
                .take(QUERY_RESULT_CAP)
                .map(String::from)
                .collect();
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Dependents { path } => {
            let key = match graph.resolve_key(&path) {
                Some(k) => k,
                None => {
                    return LinkerResponse::PathList {
                        paths: vec![],
                        total: 0,
                    }
                }
            };
            let deps = graph.dependents(key);
            let total = deps.len();
            let paths: Vec<String> = deps
                .into_iter()
                .take(QUERY_RESULT_CAP)
                .map(String::from)
                .collect();
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Impact { path, depth } => {
            let key = match graph.resolve_key(&path) {
                Some(k) => k,
                None => {
                    return LinkerResponse::PathList {
                        paths: vec![],
                        total: 0,
                    }
                }
            };
            let max_depth = depth.unwrap_or(10);
            let impact = graph.impact(key, max_depth);
            let total = impact.len();
            let paths: Vec<String> = impact
                .into_iter()
                .take(QUERY_RESULT_CAP)
                .map(String::from)
                .collect();
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Neighborhood { path } => {
            let key = match graph.resolve_key(&path) {
                Some(k) => k,
                None => {
                    return LinkerResponse::PathList {
                        paths: vec![],
                        total: 0,
                    }
                }
            };
            let (deps, dependents) = graph.neighborhood(key);
            let mut paths: Vec<String> = Vec::new();
            for d in &deps {
                paths.push(format!("{d} (dependency)"));
            }
            for d in &dependents {
                paths.push(format!("{d} (dependent)"));
            }
            let total = paths.len();
            paths.truncate(QUERY_RESULT_CAP);
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Status => {
            let languages = graph.languages();
            let top_fan_in = graph.top_fan_in(10);
            let entry_points = graph.entry_points(10);

            let mut text = format!(
                "Files: {}, Edges: {}, Gen: {}\nLanguages: {}\n",
                graph.file_count,
                graph.edge_count,
                graph.generation,
                languages.join(", ")
            );

            if !top_fan_in.is_empty() {
                text.push_str("Top fan-in:\n");
                for (path, count) in &top_fan_in {
                    text.push_str(&format!("  {path} ({count})\n"));
                }
            }

            if !entry_points.is_empty() {
                text.push_str("Entry points:\n");
                for ep in &entry_points {
                    text.push_str(&format!("  {ep}\n"));
                }
            }

            LinkerResponse::PathList {
                paths: vec![text],
                total: 1,
            }
        }
        LinkerQuery::Rescan => {
            let all_roots = collect_all_roots(state);
            if all_roots.is_empty() {
                return LinkerResponse::Ack;
            }
            let scan_rev = request_scan(state, all_roots);
            LinkerResponse::ScanRevision { revision: scan_rev }
        }
        LinkerQuery::ScanStatus => {
            let coord = state
                .scan_coordinator
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let gen = state.published_generation.load(Ordering::SeqCst);
            LinkerResponse::ScanStatusResponse {
                desired_revision: coord.desired_revision,
                applied_revision: coord.applied_revision,
                in_flight: coord.in_flight,
                generation: gen,
            }
        }
        LinkerQuery::Visualization(req) => {
            let result = graph.visualization_view(&req);
            LinkerResponse::GraphView(result)
        }
        LinkerQuery::WorkspaceInfo => {
            let info = graph.workspace_info();
            LinkerResponse::WorkspaceInfo(info)
        }
        LinkerQuery::EditContext { path } => {
            let key = graph.resolve_key(&path).unwrap_or(&path);
            let ctx = graph.edit_context(key);
            LinkerResponse::EditContext(ctx)
        }
    }
}

/// Idle reaper: exit when no registered clients and no session sockets.
async fn reaper_loop(shutting_down: Arc<std::sync::atomic::AtomicBool>, state: Arc<DaemonState>) {
    use std::sync::atomic::Ordering;

    tokio::time::sleep(REAPER_INITIAL_GRACE).await;

    let mut empty_streak: u32 = 0;
    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return;
        }

        let has_clients = {
            let clients = state.clients.read().unwrap_or_else(|e| e.into_inner());
            !clients.is_empty()
        };
        let has_session_socket = run_dir_has_socket();

        if has_clients || has_session_socket {
            empty_streak = 0;
        } else {
            empty_streak = empty_streak.saturating_add(1);
            if empty_streak >= REAPER_EMPTY_STREAK_TO_EXIT {
                shutting_down.store(true, Ordering::Relaxed);
                return;
            }
        }

        tokio::time::sleep(REAPER_POLL).await;
    }
}

/// Whether the run dir currently contains ANY `.sock` file.
#[cfg(unix)]
fn run_dir_has_socket() -> bool {
    let dir = match crate::model::store::run_dir() {
        Ok(d) => d,
        Err(_) => return true,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return true,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sock") {
            return true;
        }
    }
    false
}

/// Whether the run dir currently contains ANY `.sock` file (Windows).
/// On Windows, named pipes use `.sock` advisory extension for detection.
#[cfg(not(unix))]
fn run_dir_has_socket() -> bool {
    let dir = match crate::model::store::run_dir() {
        Ok(d) => d,
        Err(_) => return true,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return true,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sock") {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "linker_daemon_test.rs"]
mod tests;

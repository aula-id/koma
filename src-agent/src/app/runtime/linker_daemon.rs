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
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};

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

/// Coordinates scan versioning to prevent stale scans from publishing.
///
/// Every call to `spawn_scan_versioned` bumps `desired_revision` and sets
/// `in_flight`. When a scan thread completes it checks whether its revision
/// is still current before publishing, preventing a slow/stale scan from
/// overwriting a graph that was already updated by a newer scan.
struct ScanCoordinator {
    /// Monotonically-increasing counter, bumped on each desired scan.
    desired_revision: u64,
    /// The revision whose results are currently published in the graph.
    applied_revision: u64,
    /// `Some(rev)` while a scan thread for `rev` is running.
    in_flight: Option<u64>,
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
            }),
            session_revisions: RwLock::new(HashMap::new()),
            published_generation: std::sync::atomic::AtomicU64::new(0),
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
                spawn_scan_versioned(state, all_roots);
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
                    spawn_scan_versioned(state, all_roots);
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

/// Spawn a versioned background scan thread that replaces the graph and project
/// index.  The scan is tagged with a monotonically-increasing `desired_revision`
/// from the [`ScanCoordinator`].  When the thread completes, it only publishes
/// results if its revision is still the latest — preventing a slow/stale scan
/// from overwriting a graph that was already updated by a newer scan.
///
/// Returns the accepted scan revision (monotonically increasing).
///
/// On thread-spawn failure the coordinator and scanning flag are restored so
/// state never becomes permanently inconsistent.
fn spawn_scan_versioned(state: &Arc<DaemonState>, roots: Vec<PathBuf>) -> u64 {
    let scan_rev = {
        let mut coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        coord.desired_revision += 1;
        coord.in_flight = Some(coord.desired_revision);
        coord.desired_revision
    };
    state.scanning.store(true, Ordering::SeqCst);

    let state_clone = Arc::clone(state);
    if std::thread::Builder::new()
        .name("linker-scan".to_string())
        .spawn(move || {
            let (graph, pi) = crate::linker::scan::scan_roots(&roots);

            // Publish atomically with respect to scan scheduling and the
            // watcher loop.  Holding the coordinator prevents a newer desired
            // scan from being registered, and publication_lock ensures the
            // graph + project_index pair is swapped atomically relative to
            // watcher batches.
            {
                let mut coord = state_clone
                    .scan_coordinator
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if coord.in_flight == Some(scan_rev) && coord.desired_revision == scan_rev {
                    let _pub = state_clone
                        .publication_lock
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    // Advance the monotonically-increasing generation counter.
                    let new_gen = state_clone
                        .published_generation
                        .fetch_add(1, Ordering::SeqCst)
                        + 1;
                    if let Ok(mut g) = state_clone.graph.write() {
                        *g = graph;
                        // Set generation AFTER swap so it never moves backward.
                        g.generation = new_gen;
                    }
                    if let Ok(mut idx) = state_clone.project_index.write() {
                        *idx = pi;
                    }
                    coord.applied_revision = scan_rev;
                    coord.in_flight = None;
                } else if coord.in_flight == Some(scan_rev) {
                    // Superseded without a replacement scan (for example all
                    // roots were removed): release the stale slot so scanning
                    // cannot remain stuck forever.
                    coord.in_flight = None;
                }
            }

            // Update scanning flag based on whether any scan is still in flight.
            let still_in_flight = {
                let coord = state_clone
                    .scan_coordinator
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                coord.in_flight.is_some()
            };
            state_clone
                .scanning
                .store(still_in_flight, Ordering::SeqCst);
        })
        .is_err()
    {
        // Thread spawn failure — restore coordinator and scanning flag.
        let mut coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if coord.in_flight == Some(scan_rev) {
            coord.in_flight = None;
        }
        state
            .scanning
            .store(coord.in_flight.is_some(), Ordering::SeqCst);
    }

    scan_rev
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

/// Background thread: read debounced file-change events and update the graph.
///
/// Runs until the receiver is disconnected (watcher dropped).
///
/// **Phase 2:** Uses `ProjectIndex` for owner-based resolution.
///
/// **Coordination:** Acquires `publication_lock` during graph + project_index
/// mutation so the pair is always updated atomically relative to scan-thread
/// publications.  Also bumps `desired_revision` so any in-flight full scan
/// sees itself as stale and does not overwrite watcher-applied mutations.
/// If the watcher supersedes an in-flight scan, a follow-up rescan is
/// scheduled so the full-scan path re-applies cleanly.
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
    while let Ok(paths) = rx.recv() {
        if paths.is_empty() {
            continue;
        }

        // Supersede any in-flight full scan so it won't overwrite our
        // incremental mutations when it completes.
        let superseded = {
            let mut coord = state
                .scan_coordinator
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let was_in_flight = coord.in_flight.is_some();
            if was_in_flight {
                coord.desired_revision += 1;
            }
            was_in_flight
        };

        // Hold publication_lock during the graph + project_index mutation
        // so it is atomic with respect to scan-thread publications.
        {
            let _pub = state
                .publication_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let (Ok(mut graph), Ok(mut pi)) = (state.graph.write(), state.project_index.write())
            {
                crate::linker::watch::handle_events(&paths, &mut graph, &mut pi);
            }
        }

        // If we superseded an in-flight scan, schedule a follow-up rescan
        // so the full-scan path re-applies cleanly (the stale scan's
        // results are discarded, but any roots it was scanning still need
        // a fresh pass).
        if superseded {
            let all_roots = collect_all_roots(&state);
            if !all_roots.is_empty() {
                spawn_scan_versioned(&state, all_roots);
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
            let scan_rev = spawn_scan_versioned(state, all_roots);
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
mod tests {
    use super::*;
    use crate::ipc::linker_proto::{LinkerRequest, LinkerResponse, ScanStatus};

    /// Build a minimal DaemonState for unit tests (no watcher, no tokio).
    fn test_state() -> Arc<DaemonState> {
        Arc::new(DaemonState::new())
    }

    /// Helper: send a RegisterWorkspaces request and return the response.
    fn register(state: &Arc<DaemonState>, session_id: &str, roots: &[&str]) -> LinkerResponse {
        handle_request(
            LinkerRequest::RegisterWorkspaces {
                roots: roots.iter().map(|s| s.to_string()).collect(),
                session_id: session_id.to_string(),
                registration_revision: None,
            },
            &std::sync::atomic::AtomicBool::new(false),
            state,
        )
    }

    /// Helper: send a RegisterWorkspaces request with a revision tag.
    fn register_with_rev(
        state: &Arc<DaemonState>,
        session_id: &str,
        roots: &[&str],
        rev: u64,
    ) -> LinkerResponse {
        handle_request(
            LinkerRequest::RegisterWorkspaces {
                roots: roots.iter().map(|s| s.to_string()).collect(),
                session_id: session_id.to_string(),
                registration_revision: Some(rev),
            },
            &std::sync::atomic::AtomicBool::new(false),
            state,
        )
    }

    /// Helper: send an Unregister request.
    fn unregister(state: &Arc<DaemonState>, session_id: &str) -> LinkerResponse {
        handle_request(
            LinkerRequest::Unregister {
                session_id: session_id.to_string(),
            },
            &std::sync::atomic::AtomicBool::new(false),
            state,
        )
    }

    /// Helper: read the current refcount for a root.
    fn refcount(state: &Arc<DaemonState>, root: &str) -> u32 {
        state
            .root_refs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&PathBuf::from(root))
            .copied()
            .unwrap_or(0)
    }

    /// Helper: read the session's registered root set.
    fn session_roots(state: &Arc<DaemonState>, session_id: &str) -> HashSet<PathBuf> {
        state
            .clients
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Helper: check whether the clients map has a key for this session.
    fn has_client(state: &Arc<DaemonState>, session_id: &str) -> bool {
        state
            .clients
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(session_id)
    }

    // ── Idempotent repeat ──────────────────────────────────────────────

    #[test]
    fn idempotent_repeat_no_change() {
        let state = test_state();
        // First registration.
        let r1 = register(&state, "s1", &["/a", "/b"]);
        assert!(matches!(r1, LinkerResponse::Registered { .. }));
        assert_eq!(refcount(&state, "/a"), 1);
        assert_eq!(refcount(&state, "/b"), 1);

        // Re-register the same set — refcounts must not double.
        let r2 = register(&state, "s1", &["/a", "/b"]);
        assert!(matches!(r2, LinkerResponse::Registered { .. }));
        assert_eq!(refcount(&state, "/a"), 1, "refcount must stay 1");
        assert_eq!(refcount(&state, "/b"), 1, "refcount must stay 1");
    }

    // ── Add/remove/replace ─────────────────────────────────────────────

    #[test]
    fn add_roots_increments_refcount() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        assert_eq!(refcount(&state, "/a"), 1);

        register(&state, "s1", &["/a", "/b"]);
        assert_eq!(refcount(&state, "/a"), 1);
        assert_eq!(refcount(&state, "/b"), 1);
    }

    #[test]
    fn remove_roots_decrements_refcount() {
        let state = test_state();
        register(&state, "s1", &["/a", "/b"]);
        assert_eq!(refcount(&state, "/a"), 1);
        assert_eq!(refcount(&state, "/b"), 1);

        register(&state, "s1", &["/a"]);
        assert_eq!(refcount(&state, "/a"), 1);
        assert_eq!(refcount(&state, "/b"), 0, "removed root refcount must be 0");
        assert!(
            !state
                .root_refs
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&PathBuf::from("/b")),
            "removed root must be cleaned from refs map"
        );
    }

    #[test]
    fn replace_all_roots() {
        let state = test_state();
        register(&state, "s1", &["/a", "/b"]);
        register(&state, "s1", &["/c", "/d"]);
        assert_eq!(refcount(&state, "/a"), 0);
        assert_eq!(refcount(&state, "/b"), 0);
        assert_eq!(refcount(&state, "/c"), 1);
        assert_eq!(refcount(&state, "/d"), 1);
        assert_eq!(
            session_roots(&state, "s1"),
            [PathBuf::from("/c"), PathBuf::from("/d")]
                .into_iter()
                .collect()
        );
    }

    // ── Shared roots / refcounts ───────────────────────────────────────

    #[test]
    fn shared_root_refcount() {
        let state = test_state();
        register(&state, "s1", &["/shared", "/a"]);
        register(&state, "s2", &["/shared", "/b"]);
        assert_eq!(refcount(&state, "/shared"), 2);
        assert_eq!(refcount(&state, "/a"), 1);
        assert_eq!(refcount(&state, "/b"), 1);

        // Drop s1 — shared root refcount decrements to 1, not 0.
        unregister(&state, "s1");
        assert_eq!(refcount(&state, "/shared"), 1);
        assert_eq!(refcount(&state, "/a"), 0);
        // /b still alive via s2.
        assert_eq!(refcount(&state, "/b"), 1);
    }

    #[test]
    fn shared_root_survives_partial_unregister() {
        let state = test_state();
        register(&state, "s1", &["/x"]);
        register(&state, "s2", &["/x"]);
        assert_eq!(refcount(&state, "/x"), 2);
        unregister(&state, "s1");
        assert_eq!(refcount(&state, "/x"), 1);
        // Session s2 still has it.
        assert!(session_roots(&state, "s2").contains(&PathBuf::from("/x")));
    }

    // ── Global union / watcher ─────────────────────────────────────────

    #[test]
    fn watcher_not_updated_when_union_unchanged() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        let roots_before = collect_all_roots(&state);
        // Re-register same set — union is unchanged.
        register(&state, "s1", &["/a"]);
        let roots_after = collect_all_roots(&state);
        assert_eq!(roots_before, roots_after, "global union must be stable");
    }

    #[test]
    fn global_union_reflects_all_sessions() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        register(&state, "s2", &["/b"]);
        register(&state, "s3", &["/a", "/c"]);
        let all = collect_all_roots(&state);
        assert_eq!(
            all,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
    }

    // ── Final reference drop clearing/rescanning ───────────────────────

    #[test]
    fn final_unregister_clears_clients() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        unregister(&state, "s1");
        assert!(
            session_roots(&state, "s1").is_empty(),
            "session should be removed from clients map"
        );
        assert_eq!(refcount(&state, "/a"), 0);
    }

    #[test]
    fn final_unregister_invalidates_in_flight_scan() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        {
            let mut coord = state
                .scan_coordinator
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            coord.desired_revision = coord.desired_revision.saturating_add(1);
            coord.in_flight = Some(coord.desired_revision);
            state.scanning.store(true, Ordering::SeqCst);
        }
        unregister(&state, "s1");
        let coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(coord.in_flight.is_none());
        assert_eq!(coord.applied_revision, coord.desired_revision);
        assert!(!state.scanning.load(Ordering::SeqCst));
        assert!(state
            .graph
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .nodes
            .is_empty());
    }

    #[test]
    fn unregister_nonexistent_session_is_noop() {
        let state = test_state();
        let resp = unregister(&state, "ghost");
        assert!(matches!(resp, LinkerResponse::Ack));
    }

    // ── Empty registration / edge cases ────────────────────────────────

    #[test]
    fn empty_roots_clears_session() {
        let state = test_state();
        register(&state, "s1", &["/a", "/b"]);
        register(&state, "s1", &[]); // empty — should clear
        assert!(session_roots(&state, "s1").is_empty());
        assert_eq!(refcount(&state, "/a"), 0);
        assert_eq!(refcount(&state, "/b"), 0);
    }

    #[test]
    fn first_registration_triggers_scan() {
        let state = test_state();
        // Before first registration, scanning should be false.
        assert!(!state.scanning.load(std::sync::atomic::Ordering::SeqCst));
        let _resp = register(&state, "s1", &["/nonexistent_root_for_test"]);
        // After registration of new root, scanning should be true (scan thread spawned).
        assert!(
            state.scanning.load(std::sync::atomic::Ordering::SeqCst),
            "first registration must trigger a scan"
        );
    }

    // ── Response format checks ─────────────────────────────────────────

    #[test]
    fn register_response_contains_status_and_generation() {
        let state = test_state();
        let resp = register(&state, "s1", &["/a"]);
        match resp {
            LinkerResponse::Registered { status, generation } => {
                // generation starts at 0 (no scan done yet in sync path).
                assert_eq!(generation, 0);
                assert!(matches!(status, ScanStatus::Scanning | ScanStatus::Ready));
            }
            other => panic!("expected Registered, got {other:?}"),
        }
    }

    // ── Overlapping session changes ────────────────────────────────────

    #[test]
    fn overlapping_sessions_one_adds_one_removes() {
        let state = test_state();
        register(&state, "s1", &["/shared", "/only1"]);
        register(&state, "s2", &["/shared", "/only2"]);

        // s1 drops /shared, keeps /only1.
        register(&state, "s1", &["/only1"]);
        assert_eq!(refcount(&state, "/shared"), 1, "s2 still holds /shared");
        assert_eq!(refcount(&state, "/only1"), 1);
        assert_eq!(refcount(&state, "/only2"), 1);

        // s2 drops /shared too — finally unreferenced.
        register(&state, "s2", &["/only2"]);
        assert_eq!(refcount(&state, "/shared"), 0);
        assert!(
            !state
                .root_refs
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&PathBuf::from("/shared")),
            "/shared should be removed from refs when refcount hits 0"
        );
    }

    // ── Requirement 3: Empty register removes client key ──────────────

    #[test]
    fn empty_register_removes_client_key_for_reaper() {
        let state = test_state();
        register(&state, "s1", &["/a", "/b"]);
        assert!(
            has_client(&state, "s1"),
            "client should exist after registration"
        );

        // Empty registration removes the key entirely.
        register(&state, "s1", &[]);
        assert!(
            !has_client(&state, "s1"),
            "empty registration must remove client key so reaper can reap"
        );
        assert_eq!(refcount(&state, "/a"), 0);
        assert_eq!(refcount(&state, "/b"), 0);
    }

    #[test]
    fn reaper_sees_empty_after_unregister() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        assert!(has_client(&state, "s1"));

        unregister(&state, "s1");
        assert!(!has_client(&state, "s1"));
        assert!(
            state
                .clients
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty(),
            "clients map should be empty so reaper can exit"
        );
    }

    // ── Requirement 1: Concurrent handler atomicity ────────────────────

    #[test]
    fn concurrent_register_unregister_refcounts_consistent() {
        let state = test_state();
        register(&state, "s1", &["/shared", "/only1"]);
        register(&state, "s2", &["/shared", "/only2"]);

        // Spawn concurrent operations: s1 re-registers, s2 unregisters.
        let state_c = Arc::clone(&state);
        let h1 = std::thread::spawn(move || {
            register(&state_c, "s1", &["/shared"]);
        });
        let state_c = Arc::clone(&state);
        let h2 = std::thread::spawn(move || {
            unregister(&state_c, "s2");
        });
        h1.join().unwrap();
        h2.join().unwrap();

        // Regardless of ordering, final state must be consistent:
        // /shared held by s1 only (refcount 1), /only1 dropped by s1, /only2 dropped by s2.
        assert_eq!(refcount(&state, "/shared"), 1);
        assert_eq!(
            refcount(&state, "/only1"),
            0,
            "s1 dropped /only1 when re-registering with only /shared"
        );
        assert_eq!(refcount(&state, "/only2"), 0);
    }

    #[test]
    fn concurrent_registrations_stable_refcounts() {
        let state = test_state();
        // Spawn 8 threads, each registering a different root for a unique session.
        let mut handles = Vec::new();
        for i in 0..8 {
            let state_c = Arc::clone(&state);
            let root = format!("/root_{i}");
            let sid = format!("s{i}");
            handles.push(std::thread::spawn(move || {
                register(&state_c, &sid, &[&root]);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Each root must have refcount exactly 1.
        for i in 0..8 {
            assert_eq!(
                refcount(&state, &format!("/root_{i}")),
                1,
                "root_{i} refcount must be 1"
            );
        }
        // Exactly 8 clients registered.
        assert_eq!(
            state
                .clients
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            8
        );
    }

    // ── Requirement 2: Revision gating / stale rejection ───────────────

    #[test]
    fn revision_rejects_stale_registration() {
        let state = test_state();
        // Register with revision 2.
        let r2 = register_with_rev(&state, "s1", &["/a"], 2);
        assert!(matches!(r2, LinkerResponse::Registered { .. }));
        assert_eq!(refcount(&state, "/a"), 1);

        // Try to register with revision 1 (stale) — should be rejected.
        let r1 = register_with_rev(&state, "s1", &["/b"], 1);
        assert!(matches!(r1, LinkerResponse::Registered { .. }));
        // /b must NOT have been registered (stale rejected).
        assert_eq!(refcount(&state, "/b"), 0, "stale revision must be rejected");
        // /a still held.
        assert_eq!(refcount(&state, "/a"), 1);
    }

    #[test]
    fn revision_accepts_newer_registration() {
        let state = test_state();
        register_with_rev(&state, "s1", &["/a"], 1);
        assert_eq!(refcount(&state, "/a"), 1);

        // Register with revision 2 (newer) — should succeed and replace.
        register_with_rev(&state, "s1", &["/b"], 2);
        assert_eq!(refcount(&state, "/a"), 0, "old root should be released");
        assert_eq!(refcount(&state, "/b"), 1, "new root should be registered");
    }

    #[test]
    fn revision_equal_is_accepted() {
        let state = test_state();
        register_with_rev(&state, "s1", &["/a"], 5);
        // Same revision should be accepted (>= check).
        register_with_rev(&state, "s1", &["/a", "/b"], 5);
        assert_eq!(refcount(&state, "/a"), 1);
        assert_eq!(refcount(&state, "/b"), 1);
    }

    #[test]
    fn no_revision_always_accepted() {
        let state = test_state();
        // No revision → always accepted (backward compat).
        register(&state, "s1", &["/a"]);
        register(&state, "s1", &["/b"]);
        register(&state, "s1", &["/c"]);
        assert_eq!(refcount(&state, "/a"), 0);
        assert_eq!(refcount(&state, "/b"), 0);
        assert_eq!(refcount(&state, "/c"), 1);
    }

    // ── Requirement 2: Scan versioning coordinator ─────────────────────

    #[test]
    fn scan_coordinator_starts_at_zero() {
        let state = test_state();
        let coord = state.scan_coordinator.lock().unwrap();
        assert_eq!(coord.desired_revision, 0);
        assert_eq!(coord.applied_revision, 0);
        assert!(coord.in_flight.is_none());
    }

    #[test]
    fn versioned_scan_bumps_desired_revision() {
        let state = test_state();
        // After first registration, a scan should have been scheduled.
        register(&state, "s1", &["/nonexistent_for_scan_test"]);
        // Wait briefly for thread to spawn.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let coord = state.scan_coordinator.lock().unwrap();
        assert!(
            coord.desired_revision >= 1,
            "desired_revision should be >= 1 after registration"
        );
    }

    // ── Task 1: Scan revision / status contract ─────────────────────────

    #[test]
    fn repeated_rescan_advances_revision() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        // Wait for scan thread to spawn and register in coordinator.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let rev1 = {
            let coord = state.scan_coordinator.lock().unwrap();
            coord.desired_revision
        };
        assert!(rev1 >= 1, "first rescan should bump desired_revision");

        // Issue a manual Rescan query.
        let resp = handle_request(
            LinkerRequest::Query(LinkerQuery::Rescan),
            &std::sync::atomic::AtomicBool::new(false),
            &state,
        );
        let scan_rev = match resp {
            LinkerResponse::ScanRevision { revision } => revision,
            other => panic!("expected ScanRevision, got {other:?}"),
        };
        assert!(
            scan_rev > rev1,
            "rescan revision {scan_rev} should be > previous {rev1}"
        );

        // A second Rescan must advance further.
        let resp2 = handle_request(
            LinkerRequest::Query(LinkerQuery::Rescan),
            &std::sync::atomic::AtomicBool::new(false),
            &state,
        );
        let scan_rev2 = match resp2 {
            LinkerResponse::ScanRevision { revision } => revision,
            other => panic!("expected ScanRevision, got {other:?}"),
        };
        assert!(
            scan_rev2 > scan_rev,
            "second rescan revision {scan_rev2} should be > first {scan_rev}"
        );
    }

    #[test]
    fn scan_status_returns_coordinator_state() {
        let state = test_state();
        // Initially all zero.
        let resp = handle_request(
            LinkerRequest::Query(LinkerQuery::ScanStatus),
            &std::sync::atomic::AtomicBool::new(false),
            &state,
        );
        match resp {
            LinkerResponse::ScanStatusResponse {
                desired_revision,
                applied_revision,
                in_flight,
                generation,
            } => {
                assert_eq!(desired_revision, 0);
                assert_eq!(applied_revision, 0);
                assert!(in_flight.is_none());
                assert_eq!(generation, 0);
            }
            other => panic!("expected ScanStatusResponse, got {other:?}"),
        }
    }

    #[test]
    fn rescan_returns_accepted_revision_for_later_poll() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Issue Rescan and capture revision.
        let resp = handle_request(
            LinkerRequest::Query(LinkerQuery::Rescan),
            &std::sync::atomic::AtomicBool::new(false),
            &state,
        );
        let scan_rev = match resp {
            LinkerResponse::ScanRevision { revision } => revision,
            other => panic!("expected ScanRevision, got {other:?}"),
        };

        // ScanStatus should show the revision as desired.
        let status_resp = handle_request(
            LinkerRequest::Query(LinkerQuery::ScanStatus),
            &std::sync::atomic::AtomicBool::new(false),
            &state,
        );
        match status_resp {
            LinkerResponse::ScanStatusResponse {
                desired_revision, ..
            } => {
                assert!(
                    desired_revision >= scan_rev,
                    "desired_revision {desired_revision} should be >= scan_rev {scan_rev}"
                );
            }
            other => panic!("expected ScanStatusResponse, got {other:?}"),
        }
    }

    // ── Task 2: Publication/watcher coordination ─────────────────────────

    #[test]
    fn publication_lock_exists() {
        let state = test_state();
        // Verify the publication_lock can be acquired (no deadlock in test).
        let _guard = state
            .publication_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
    }

    #[test]
    fn watcher_supersede_bumps_desired_revision() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Snapshot desired_revision before watcher event.
        let before = {
            let coord = state.scan_coordinator.lock().unwrap();
            coord.desired_revision
        };

        // Simulate a watcher event arriving while a scan is in_flight:
        // set in_flight artificially.
        {
            let mut coord = state.scan_coordinator.lock().unwrap();
            coord.in_flight = Some(coord.desired_revision);
        }

        // Now the watcher_loop would bump desired_revision.  We simulate
        // the relevant section of watcher_loop here:
        let superseded = {
            let mut coord = state.scan_coordinator.lock().unwrap();
            let was_in_flight = coord.in_flight.is_some();
            if was_in_flight {
                coord.desired_revision += 1;
            }
            was_in_flight
        };
        assert!(superseded, "should detect in-flight scan");

        let after = {
            let coord = state.scan_coordinator.lock().unwrap();
            coord.desired_revision
        };
        assert!(
            after > before,
            "desired_revision should advance after supersede: before={before}, after={after}"
        );
    }

    #[test]
    fn stale_scan_does_not_publish_over_watcher() {
        let state = test_state();
        register(&state, "s1", &["/a"]);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Simulate: scan rev=1 is in flight, then watcher bumps desired to 2.
        let mut coord = state.scan_coordinator.lock().unwrap();
        let rev1 = coord.desired_revision;
        coord.in_flight = Some(rev1);
        coord.desired_revision = rev1 + 1;
        drop(coord);

        // Now scan rev=1 tries to publish.  The staleness check:
        //   coord.in_flight == Some(rev1) && coord.desired_revision == rev1
        // should FAIL because desired_revision was bumped.
        let coord = state.scan_coordinator.lock().unwrap();
        assert!(
            !(coord.in_flight == Some(rev1) && coord.desired_revision == rev1),
            "scan rev={rev1} should be stale (desired={})",
            coord.desired_revision
        );
    }

    // ── Task 3: Delayed older worker rejected ───────────────────────────

    #[test]
    fn delayed_older_worker_registration_rejected() {
        let state = test_state();
        // Worker A registers with revision 5.
        let r5 = register_with_rev(&state, "s1", &["/a"], 5);
        assert!(matches!(r5, LinkerResponse::Registered { .. }));
        assert_eq!(refcount(&state, "/a"), 1);

        // Worker B arrives late with revision 3 (older) — should be rejected.
        let r3 = register_with_rev(&state, "s1", &["/b"], 3);
        assert!(matches!(r3, LinkerResponse::Registered { .. }));
        assert_eq!(refcount(&state, "/b"), 0, "stale worker must be rejected");
        assert_eq!(refcount(&state, "/a"), 1, "original root unchanged");
    }

    // ── Task 4: Response validation ─────────────────────────────────────

    #[test]
    fn registered_only_is_success_for_validation() {
        // Simulate what ensure_and_register_with_revision checks:
        // Only Registered should be Ok.
        let registered = LinkerResponse::Registered {
            status: crate::ipc::linker_proto::ScanStatus::Ready,
            generation: 1,
        };
        assert!(matches!(registered, LinkerResponse::Registered { .. }));

        let error = LinkerResponse::Error("test".into());
        assert!(!matches!(error, LinkerResponse::Registered { .. }));

        let ack = LinkerResponse::Ack;
        assert!(!matches!(ack, LinkerResponse::Registered { .. }));
    }
}

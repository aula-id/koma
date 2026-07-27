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

/// Shared daemon state: the import graph plus per-session root tracking and
/// the file watcher.
struct DaemonState {
    graph: RwLock<ImportGraph>,
    /// session_id → set of registered workspace root paths.
    sessions: RwLock<HashMap<String, HashSet<PathBuf>>>,
    /// The debounced file watcher (kept alive while the daemon runs).
    /// Dropped on shutdown or when all roots are unregistered.
    watcher: Mutex<Option<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>>>,
    /// Channel receiver for debounced file-change batches from the watcher.
    /// Wrapped in Mutex so it can be taken out once and polled in a dedicated
    /// thread.
    watcher_rx: Mutex<Option<std::sync::mpsc::Receiver<Vec<PathBuf>>>>,
    /// The set of roots currently being watched (so we can detect changes).
    watched_roots: RwLock<Vec<PathBuf>>,
}

impl DaemonState {
    fn new() -> Self {
        Self {
            graph: RwLock::new(ImportGraph::new()),
            sessions: RwLock::new(HashMap::new()),
            watcher: Mutex::new(None),
            watcher_rx: Mutex::new(None),
            watched_roots: RwLock::new(Vec::new()),
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
async fn respond(
    stream: &mut crate::ipc::IpcStream,
    resp: &LinkerResponse,
) -> std::io::Result<()> {
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
        LinkerRequest::Fingerprint => {
            LinkerResponse::Fingerprint(store::build_fingerprint())
        }
        LinkerRequest::Shutdown => {
            shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
            LinkerResponse::Ack
        }
        LinkerRequest::RegisterWorkspaces { roots, session_id } => {
            let paths: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();

            // Register the session's roots.
            {
                let mut sessions = state.sessions.write().unwrap_or_else(|e| e.into_inner());
                sessions.insert(session_id, paths.iter().cloned().collect());
            }

            // Collect all roots across all sessions.
            let all_roots = collect_all_roots(state);

            // (Re-)create the watcher if roots changed.
            maybe_update_watcher(state, &all_roots);

            // Spawn scan on a blocking thread (tree-sitter parsing is CPU-bound).
            let state_clone = Arc::clone(state);
            let scan_roots = all_roots.clone();
            std::thread::spawn(move || {
                let graph = crate::linker::scan::scan_roots(&scan_roots);
                if let Ok(mut g) = state_clone.graph.write() {
                    *g = graph;
                }
            });

            LinkerResponse::Ready
        }
        LinkerRequest::Unregister { session_id } => {
            let removed;
            {
                let mut sessions = state.sessions.write().unwrap_or_else(|e| e.into_inner());
                removed = sessions.remove(&session_id).is_some();
            }

            if removed {
                // Rescan remaining roots.
                let all_roots = collect_all_roots(state);

                if all_roots.is_empty() {
                    // No roots left — stop the watcher.
                    stop_watcher(state);
                } else {
                    // Roots may have changed — update watcher.
                    maybe_update_watcher(state, &all_roots);
                }

                let state_clone = Arc::clone(state);
                let scan_roots = all_roots;
                std::thread::spawn(move || {
                    let graph = crate::linker::scan::scan_roots(&scan_roots);
                    if let Ok(mut g) = state_clone.graph.write() {
                        *g = graph;
                    }
                });
            }

            LinkerResponse::Ack
        }
        LinkerRequest::Summary => {
            let graph = state.graph.read().unwrap_or_else(|e| e.into_inner());
            let languages = graph.languages();
            let file_count = graph.file_count;
            let edge_count = graph.edge_count;
            let generation = graph.generation;

            let top_fan_in = graph.top_fan_in(5);
            let entry_points = graph.entry_points(10);

            let mut text = format!(
                "Import graph: {file_count} files, {edge_count} edges (gen {generation})\n\
                 Languages: {}\n",
                languages.join(", ")
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

            LinkerResponse::Summary {
                text,
                generation,
                file_count,
                edge_count,
                languages,
            }
        }
        LinkerRequest::Query(query) => {
            let graph = state.graph.read().unwrap_or_else(|e| e.into_inner());
            handle_query(query, &graph)
        }
    }
}

/// Collect all workspace roots across all registered sessions.
fn collect_all_roots(state: &Arc<DaemonState>) -> Vec<PathBuf> {
    let sessions = state.sessions.read().unwrap_or_else(|e| e.into_inner());
    let mut all_roots: Vec<PathBuf> = sessions.values().flatten().cloned().collect();
    all_roots.sort();
    all_roots.dedup();
    all_roots
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
        let wr = state.watched_roots.read().unwrap_or_else(|e| e.into_inner());
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
            *state.watcher.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(debouncer);
            *state.watcher_rx.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(rx);
            *state.watched_roots.write().unwrap_or_else(|e| e.into_inner()) =
                new_roots.to_vec();

            // Spawn the watcher event-processing thread.
            let state_clone = Arc::clone(state);
            let watched = new_roots.to_vec();
            std::thread::Builder::new()
                .name("linker-watcher".to_string())
                .spawn(move || watcher_loop(state_clone, watched))
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
fn watcher_loop(state: Arc<DaemonState>, workspace_roots: Vec<PathBuf>) {
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

        if let Ok(mut graph) = state.graph.write() {
            crate::linker::watch::handle_events(&paths, &mut graph, &workspace_roots);
        }
    }
}

/// Dispatch a graph query and produce a response.
fn handle_query(query: LinkerQuery, graph: &ImportGraph) -> LinkerResponse {
    match query {
        LinkerQuery::Dependencies { path } => {
            let deps = graph.dependencies(&path);
            let total = deps.len();
            let paths: Vec<String> = deps.into_iter().take(QUERY_RESULT_CAP).map(String::from).collect();
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Dependents { path } => {
            let deps = graph.dependents(&path);
            let total = deps.len();
            let paths: Vec<String> = deps.into_iter().take(QUERY_RESULT_CAP).map(String::from).collect();
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Impact { path, depth } => {
            let max_depth = depth.unwrap_or(10);
            let impact = graph.impact(&path, max_depth);
            let total = impact.len();
            let paths: Vec<String> = impact.into_iter().take(QUERY_RESULT_CAP).map(String::from).collect();
            LinkerResponse::PathList { paths, total }
        }
        LinkerQuery::Neighborhood { path } => {
            let (deps, dependents) = graph.neighborhood(&path);
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
            // Trigger an asynchronous rescan (same as RegisterWorkspaces).
            // For now, just return Ack; the caller should re-register to trigger rescan.
            LinkerResponse::Ack
        }
    }
}

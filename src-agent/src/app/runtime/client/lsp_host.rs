//! Host-side language-server status / install / uninstall for the GUI Settings
//! "Language servers" section, plus the live [`crate::lsp::LspManager`] dispatch
//! for Monaco doc-sync and language features.
//!
//! Entirely host-local (never the daemon), mirroring [`super::keys`]: a
//! `std::thread::spawn` worker does the blocking work (`reqwest`, npm, pip, go).
//!
//! Two flavors:
//! - **detached** (`host_swapper`): push replies straight through a cloned sink
//! - **attached** (`push_loop`): send replies over an `mpsc` channel drained by
//!   the fold loop (the fold's `push` is `&dyn Fn(String)` and cannot be cloned)
//!
//! Language-client ops use a process-wide ordered notify worker (didOpen/Change/
//! Save/Close, with didChange coalescing) plus a bounded request pool so a burst
//! of Monaco features never spawns unbounded OS threads.

use std::sync::mpsc::Sender;

use super::push_proto::{push_lsp_install, push_lsp_status};

/// One frame the attached worker may send back to `push_loop`.
pub(super) enum LspReply {
    Status(Vec<crate::lsp::ServerStatus>),
    Install {
        id: String,
        pct: u8,
        error: Option<String>,
    },
}

// ─── DETACHED (host_swapper): push straight through the cloned sink ──────────

/// `HostCtl::LspStatus` while detached.
pub(super) fn spawn_lsp_status(push: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        let servers = crate::lsp::status_all();
        push_lsp_status(&push, servers);
    });
}

/// `HostCtl::LspInstall` while detached.
pub(super) fn spawn_lsp_install(
    push: impl Fn(String) + Clone + Send + 'static,
    id: Option<String>,
    all: bool,
    force: bool,
) {
    std::thread::spawn(move || {
        if all {
            for spec in crate::lsp::CATALOG {
                if !is_managed_installable(spec.id) {
                    continue;
                }
                install_one_cloned(&push, spec.id, force);
            }
        } else if let Some(ref id) = id {
            install_one_cloned(&push, id, force);
        } else {
            push_lsp_install(&push, "", 0, Some("lsp install requires id or all".into()));
        }
        let servers = crate::lsp::status_all();
        push_lsp_status(&push, servers);
    });
}

fn install_one_cloned(push: &(impl Fn(String) + Clone), id: &str, force: bool) {
    // Detached path: push start + end only. Intermediate pct needs a 'static
    // Send sink (ProgressFn); the host_swapper push is Clone+Send+'static when
    // owned, but here we only hold a borrow. Attached path streams live pct
    // via mpsc (install_one_tx).
    push_lsp_install(push, id, 0, None);
    match crate::lsp::install_one(id, force, None) {
        Ok(()) => push_lsp_install(push, id, 100, None),
        Err(e) => push_lsp_install(push, id, 0, Some(format!("{e:#}"))),
    }
}

/// `HostCtl::LspUninstall` while detached.
pub(super) fn spawn_lsp_uninstall(push: impl Fn(String) + Send + 'static, id: String) {
    std::thread::spawn(move || {
        match crate::lsp::uninstall_one(&id) {
            Ok(()) => push_lsp_install(&push, &id, 100, None),
            Err(e) => push_lsp_install(&push, &id, 0, Some(format!("{e:#}"))),
        }
        let servers = crate::lsp::status_all();
        push_lsp_status(&push, servers);
    });
}

// ─── ATTACHED (push_loop): reply over mpsc, drained by the fold loop ─────────

/// `HostCtl::LspStatus` while attached.
pub(super) fn spawn_lsp_status_attached(tx: Sender<LspReply>) {
    std::thread::spawn(move || {
        let servers = crate::lsp::status_all();
        let _ = tx.send(LspReply::Status(servers));
    });
}

/// `HostCtl::LspInstall` while attached.
pub(super) fn spawn_lsp_install_attached(
    tx: Sender<LspReply>,
    id: Option<String>,
    all: bool,
    force: bool,
) {
    std::thread::spawn(move || {
        if all {
            for spec in crate::lsp::CATALOG {
                if !is_managed_installable(spec.id) {
                    continue;
                }
                install_one_tx(&tx, spec.id, force);
            }
        } else if let Some(ref id) = id {
            install_one_tx(&tx, id, force);
        } else {
            let _ = tx.send(LspReply::Install {
                id: String::new(),
                pct: 0,
                error: Some("lsp install requires id or all".into()),
            });
        }
        let servers = crate::lsp::status_all();
        let _ = tx.send(LspReply::Status(servers));
    });
}

fn install_one_tx(tx: &Sender<LspReply>, id: &str, force: bool) {
    let tx2 = tx.clone();
    let progress: crate::lsp::ProgressFn = Box::new(move |sid, pct, err| {
        let _ = tx2.send(LspReply::Install {
            id: sid.to_string(),
            pct,
            error: err.map(|s| s.to_string()),
        });
    });
    if let Err(e) = crate::lsp::install_one(id, force, Some(progress)) {
        let _ = tx.send(LspReply::Install {
            id: id.to_string(),
            pct: 0,
            error: Some(format!("{e:#}")),
        });
    }
}

/// `HostCtl::LspUninstall` while attached.
pub(super) fn spawn_lsp_uninstall_attached(tx: Sender<LspReply>, id: String) {
    std::thread::spawn(move || {
        match crate::lsp::uninstall_one(&id) {
            Ok(()) => {
                let _ = tx.send(LspReply::Install {
                    id: id.clone(),
                    pct: 100,
                    error: None,
                });
            }
            Err(e) => {
                let _ = tx.send(LspReply::Install {
                    id: id.clone(),
                    pct: 0,
                    error: Some(format!("{e:#}")),
                });
            }
        }
        let servers = crate::lsp::status_all();
        let _ = tx.send(LspReply::Status(servers));
    });
}

/// Drain attached LSP replies into the GUI push sink. Called once per fold frame.
pub(super) fn drain_lsp_replies(rx: &std::sync::mpsc::Receiver<LspReply>, push: &dyn Fn(String)) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
            LspReply::Status(servers) => push_lsp_status(push, servers),
            LspReply::Install { id, pct, error } => push_lsp_install(push, &id, pct, error),
        }
    }
}

fn is_managed_installable(id: &str) -> bool {
    matches!(
        id,
        "rust-analyzer"
            | "taplo"
            | "clangd"
            | "vtsls"
            | "basedpyright"
            | "gopls"
            | "vscode-langservers"
            | "bash-language-server"
            | "intelephense"
    )
}

// ─── Language client (LspManager) ────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::push_proto::{
    push_lsp_completion, push_lsp_completion_resolve, push_lsp_definition,
    push_lsp_document_symbol, push_lsp_hover, push_lsp_references,
};
use super::HostCtl;
use crate::lsp::LspManager;

/// Max concurrent language-feature request threads (completion/hover/…).
const LSP_REQUEST_POOL: usize = 6;

/// Jobs for the single ordered notify worker (didOpen/Change/Save/Close).
enum NotifyJob {
    Open {
        mgr: Arc<Mutex<LspManager>>,
        root: String,
        path: String,
        language_id: String,
        text: String,
    },
    /// Coalesced by (root, path) — latest text wins before flush.
    Change {
        mgr: Arc<Mutex<LspManager>>,
        root: String,
        path: String,
        text: String,
    },
    Save {
        mgr: Arc<Mutex<LspManager>>,
        root: String,
        path: String,
        text: Option<String>,
    },
    Close {
        mgr: Arc<Mutex<LspManager>>,
        root: String,
        path: String,
    },
}

struct LspWorkers {
    notify_tx: Mutex<Sender<NotifyJob>>,
    request_tx: Mutex<Sender<(HostCtl, Arc<Mutex<LspManager>>)>>,
}

fn lsp_workers() -> &'static LspWorkers {
    static WORKERS: OnceLock<LspWorkers> = OnceLock::new();
    WORKERS.get_or_init(|| {
        let (notify_tx, notify_rx) = mpsc::channel::<NotifyJob>();
        std::thread::Builder::new()
            .name("lsp-notify".into())
            .spawn(move || notify_worker_loop(notify_rx))
            .expect("spawn lsp-notify");

        let (request_tx, request_rx) =
            mpsc::channel::<(HostCtl, Arc<Mutex<LspManager>>)>();
        let request_rx = Arc::new(Mutex::new(request_rx));
        for i in 0..LSP_REQUEST_POOL {
            let rx = Arc::clone(&request_rx);
            std::thread::Builder::new()
                .name(format!("lsp-req-{i}"))
                .spawn(move || request_worker_loop(rx))
                .expect("spawn lsp-req");
        }

        LspWorkers {
            notify_tx: Mutex::new(notify_tx),
            request_tx: Mutex::new(request_tx),
        }
    })
}

fn notify_worker_loop(rx: mpsc::Receiver<NotifyJob>) {
    // Latest didChange text per (root, path); flushed before non-change jobs and
    // after a short quiet window so bursts collapse to one notify.
    let mut pending_change: HashMap<(String, String), (Arc<Mutex<LspManager>>, String)> =
        HashMap::new();
    let coalesce = Duration::from_millis(16);

    loop {
        let first = if pending_change.is_empty() {
            match rx.recv() {
                Ok(j) => j,
                Err(_) => break,
            }
        } else {
            match rx.recv_timeout(coalesce) {
                Ok(j) => j,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    flush_pending_changes(&mut pending_change);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        match first {
            NotifyJob::Change {
                mgr,
                root,
                path,
                text,
            } => {
                pending_change.insert((root, path), (mgr, text));
                // Drain any already-queued jobs without blocking, coalescing more changes.
                while let Ok(job) = rx.try_recv() {
                    match job {
                        NotifyJob::Change {
                            mgr,
                            root,
                            path,
                            text,
                        } => {
                            pending_change.insert((root, path), (mgr, text));
                        }
                        other => {
                            flush_pending_changes(&mut pending_change);
                            run_notify_job(other);
                        }
                    }
                }
            }
            other => {
                flush_pending_changes(&mut pending_change);
                run_notify_job(other);
            }
        }
    }
}

fn flush_pending_changes(
    pending: &mut HashMap<(String, String), (Arc<Mutex<LspManager>>, String)>,
) {
    for ((root, path), (mgr, text)) in pending.drain() {
        if let Ok(mut g) = mgr.lock() {
            let _ = g.did_change(&root, &path, &text);
        }
    }
}

fn run_notify_job(job: NotifyJob) {
    match job {
        NotifyJob::Open {
            mgr,
            root,
            path,
            language_id,
            text,
        } => run_did_open(mgr, root, path, language_id, text),
        NotifyJob::Change {
            mgr,
            root,
            path,
            text,
        } => {
            if let Ok(mut g) = mgr.lock() {
                let _ = g.did_change(&root, &path, &text);
            }
        }
        NotifyJob::Save {
            mgr,
            root,
            path,
            text,
        } => {
            if let Ok(mut g) = mgr.lock() {
                let _ = g.did_save(&root, &path, text.as_deref());
            }
        }
        NotifyJob::Close { mgr, root, path } => {
            if let Ok(mut g) = mgr.lock() {
                let _ = g.did_close_path(&root, &path);
            }
        }
    }
}

fn run_did_open(
    mgr: Arc<Mutex<LspManager>>,
    root: String,
    path: String,
    language_id: String,
    text: String,
) {
    // prepare under lock → handshake outside lock → finish under lock
    let prep = match mgr.lock() {
        Ok(mut g) => g.prepare_did_open(&root, &path, &language_id, &text),
        Err(_) => return,
    };
    let prep = match prep {
        Ok(p) => p,
        Err(e) => {
            if !e.contains("no language server") && !e.contains("not installed") {
                crate::model::store::append_global_error_log(
                    "lsp",
                    &format!("didOpen {path}: {e}"),
                );
            }
            return;
        }
    };
    match prep {
        crate::lsp::client::DidOpenPrep::Done => {}
        crate::lsp::client::DidOpenPrep::NeedsHandshake {
            uninit,
            spawn_id,
            uri,
            language_id,
            text,
            root_path,
        } => {
            let session = match uninit.handshake() {
                Ok(s) => s,
                Err(e) => {
                    if let Ok(mut g) = mgr.lock() {
                        g.abort_spawn(&spawn_id);
                    }
                    crate::model::store::append_global_error_log(
                        "lsp",
                        &format!("didOpen handshake {path}: {e}"),
                    );
                    return;
                }
            };
            if let Ok(mut g) = mgr.lock() {
                if let Err(e) =
                    g.finish_did_open(spawn_id, session, uri, language_id, text, root_path)
                {
                    crate::model::store::append_global_error_log(
                        "lsp",
                        &format!("didOpen finish {path}: {e}"),
                    );
                }
            }
        }
    }
}

fn request_worker_loop(rx: Arc<Mutex<mpsc::Receiver<(HostCtl, Arc<Mutex<LspManager>>)>>>) {
    loop {
        let job = {
            let guard = match rx.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            match guard.recv() {
                Ok(j) => j,
                Err(_) => return,
            }
        };
        run_request_job(job.0, job.1);
    }
}

fn run_request_job(ctl: HostCtl, mgr: Arc<Mutex<LspManager>>) {
    let push = match mgr.lock() {
        Ok(g) => g.push_sink(),
        Err(_) => return,
    };
    match ctl {
        HostCtl::LspCompletion {
            root,
            path,
            line,
            character,
            trigger_kind,
            trigger_character,
            request_id,
        } => {
            // Phase 1: resolve SessionIo under the lock (fast).
            // Phase 2: wait on the RPC with the lock released so concurrent
            // hover/didChange/CodeLens do not serialize on Mutex<LspManager>.
            let pending = match mgr.lock() {
                Ok(mut g) => g.completion(
                    &root,
                    &path,
                    line,
                    character,
                    trigger_kind,
                    trigger_character.as_deref(),
                ),
                Err(_) => Err("lsp manager lock poisoned".into()),
            };
            let (items, is_incomplete, error) = match pending {
                Ok(p) => match p.wait_completions() {
                    Ok(list) => (list.items, list.is_incomplete, None),
                    Err(e) => (Vec::new(), false, Some(e)),
                },
                Err(e) => (Vec::new(), false, Some(e)),
            };
            push_lsp_completion(&*push, request_id, items, is_incomplete, error);
        }
        HostCtl::LspCompletionResolve {
            root,
            path,
            item,
            request_id,
        } => {
            let pending = match mgr.lock() {
                Ok(mut g) => g.resolve_completion(&root, &path, &item),
                Err(_) => Err("lsp manager lock poisoned".into()),
            };
            let (resolved, error) = match pending {
                Ok(p) => match p.wait_resolve(&item) {
                    Ok(it) => (Some(it), None),
                    Err(e) => (None, Some(e)),
                },
                Err(e) => (None, Some(e)),
            };
            push_lsp_completion_resolve(&*push, request_id, resolved, error);
        }
        HostCtl::LspHover {
            root,
            path,
            line,
            character,
            request_id,
        } => {
            let pending = match mgr.lock() {
                Ok(mut g) => g.hover(&root, &path, line, character),
                Err(_) => Err("lsp manager lock poisoned".into()),
            };
            let (hover, error) = match pending {
                Ok(p) => match p.wait_hover() {
                    Ok(h) => (h, None),
                    Err(e) => (None, Some(e)),
                },
                Err(e) => (None, Some(e)),
            };
            push_lsp_hover(&*push, request_id, hover, error);
        }
        HostCtl::LspDefinition {
            root,
            path,
            line,
            character,
            request_id,
        } => {
            let pending = match mgr.lock() {
                Ok(mut g) => g.definition(&root, &path, line, character),
                Err(_) => Err("lsp manager lock poisoned".into()),
            };
            let (locations, error) = match pending {
                Ok(p) => match p.wait_locations() {
                    Ok(locs) => (locs, None),
                    Err(e) => (Vec::new(), Some(e)),
                },
                Err(e) => (Vec::new(), Some(e)),
            };
            push_lsp_definition(&*push, request_id, locations, error);
        }
        HostCtl::LspReferences {
            root,
            path,
            line,
            character,
            include_declaration,
            request_id,
        } => {
            let pending = match mgr.lock() {
                Ok(mut g) => g.references(&root, &path, line, character, include_declaration),
                Err(_) => Err("lsp manager lock poisoned".into()),
            };
            let (locations, error) = match pending {
                Ok(p) => match p.wait_locations() {
                    Ok(locs) => (locs, None),
                    Err(e) => (Vec::new(), Some(e)),
                },
                Err(e) => (Vec::new(), Some(e)),
            };
            push_lsp_references(&*push, request_id, locations, error);
        }
        HostCtl::LspDocumentSymbol {
            root,
            path,
            request_id,
        } => {
            let pending = match mgr.lock() {
                Ok(mut g) => g.document_symbols(&root, &path),
                Err(_) => Err("lsp manager lock poisoned".into()),
            };
            let (symbols, error) = match pending {
                Ok(p) => match p.wait_document_symbols() {
                    Ok(syms) => (syms, None),
                    Err(e) => (Vec::new(), Some(e)),
                },
                Err(e) => (Vec::new(), Some(e)),
            };
            push_lsp_document_symbol(&*push, request_id, symbols, error);
        }
        _ => {}
    }
}

/// Dispatch one language-client HostCtl onto the ordered notify worker or the
/// bounded request pool (no unbounded `thread::spawn` per message).
///
/// Notifications (didOpen/Change/Save/Close) are FIFO on one thread; didChange
/// coalesces to the latest text per path. Requests use a fixed pool so CodeLens
/// fanout cannot explode OS threads. Replies still go through the manager's
/// push sink.
pub(super) fn handle_client_ctl(ctl: HostCtl, mgr: Arc<Mutex<LspManager>>) {
    let workers = lsp_workers();
    match ctl {
        HostCtl::LspDidOpen {
            root,
            path,
            language_id,
            text,
        } => {
            let tx = workers.notify_tx.lock().unwrap_or_else(|p| p.into_inner());
            let _ = tx.send(NotifyJob::Open {
                mgr,
                root,
                path,
                language_id,
                text,
            });
        }
        HostCtl::LspDidChange { root, path, text } => {
            let tx = workers.notify_tx.lock().unwrap_or_else(|p| p.into_inner());
            let _ = tx.send(NotifyJob::Change {
                mgr,
                root,
                path,
                text,
            });
        }
        HostCtl::LspDidSave { root, path, text } => {
            let tx = workers.notify_tx.lock().unwrap_or_else(|p| p.into_inner());
            let _ = tx.send(NotifyJob::Save {
                mgr,
                root,
                path,
                text,
            });
        }
        HostCtl::LspDidClose { root, path } => {
            let tx = workers.notify_tx.lock().unwrap_or_else(|p| p.into_inner());
            let _ = tx.send(NotifyJob::Close { mgr, root, path });
        }
        HostCtl::LspCompletion { .. }
        | HostCtl::LspCompletionResolve { .. }
        | HostCtl::LspHover { .. }
        | HostCtl::LspDefinition { .. }
        | HostCtl::LspReferences { .. }
        | HostCtl::LspDocumentSymbol { .. } => {
            let tx = workers.request_tx.lock().unwrap_or_else(|p| p.into_inner());
            let _ = tx.send((ctl, mgr));
        }
        _ => {}
    }
}

/// Returns true when `ctl` is a language-client op handled by [`handle_client_ctl`].
pub(super) fn is_client_ctl(ctl: &HostCtl) -> bool {
    matches!(
        ctl,
        HostCtl::LspDidOpen { .. }
            | HostCtl::LspDidChange { .. }
            | HostCtl::LspDidSave { .. }
            | HostCtl::LspDidClose { .. }
            | HostCtl::LspCompletion { .. }
            | HostCtl::LspCompletionResolve { .. }
            | HostCtl::LspHover { .. }
            | HostCtl::LspDefinition { .. }
            | HostCtl::LspReferences { .. }
            | HostCtl::LspDocumentSymbol { .. }
    )
}

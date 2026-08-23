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
//! Language-client ops (didOpen / completion / …) run on a short-lived worker
//! thread holding `Arc<Mutex<LspManager>>` so a slow `initialize` never blocks
//! the 16ms host control loop.

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
    )
}

// ─── Language client (LspManager) ────────────────────────────────────────────

use std::sync::{Arc, Mutex};

use super::push_proto::{
    push_lsp_completion, push_lsp_definition, push_lsp_document_symbol, push_lsp_hover,
    push_lsp_references,
};
use super::HostCtl;
use crate::lsp::LspManager;

/// Dispatch one language-client HostCtl on a worker thread.
///
/// Notifications (didOpen/Change/Save/Close) are fire-and-forget.
/// Requests always push a typed reply so Monaco providers never hang.
/// Replies go through the manager's own push sink (same Arc the reader
/// thread uses for diagnostics), so this works from both the detached
/// swapper (cloneable push) and the attached fold (`&dyn Fn` only).
pub(super) fn handle_client_ctl(ctl: HostCtl, mgr: Arc<Mutex<LspManager>>) {
    std::thread::spawn(move || {
        let push = match mgr.lock() {
            Ok(g) => g.push_sink(),
            Err(_) => return,
        };
        match ctl {
            HostCtl::LspDidOpen {
                root,
                path,
                language_id,
                text,
            } => {
                if let Ok(mut g) = mgr.lock() {
                    if let Err(e) = g.did_open(&root, &path, &language_id, &text) {
                        // Missing server is expected until install — don't spam the error log
                        // for "no language server for .md" style misses on every open.
                        if !e.contains("no language server") && !e.contains("not installed") {
                            crate::model::store::append_global_error_log(
                                "lsp",
                                &format!("didOpen {path}: {e}"),
                            );
                        }
                    }
                }
            }
            HostCtl::LspDidChange { root, path, text } => {
                if let Ok(mut g) = mgr.lock() {
                    let _ = g.did_change(&root, &path, &text);
                }
            }
            HostCtl::LspDidSave { root, path, text } => {
                if let Ok(mut g) = mgr.lock() {
                    let _ = g.did_save(&root, &path, text.as_deref());
                }
            }
            HostCtl::LspDidClose { root, path } => {
                if let Ok(mut g) = mgr.lock() {
                    let _ = g.did_close_path(&root, &path);
                }
            }
            HostCtl::LspCompletion {
                root,
                path,
                line,
                character,
                request_id,
            } => {
                let (items, error) = match mgr.lock() {
                    Ok(mut g) => match g.completion(&root, &path, line, character) {
                        Ok(items) => (items, None),
                        Err(e) => (Vec::new(), Some(e)),
                    },
                    Err(_) => (Vec::new(), Some("lsp manager lock poisoned".into())),
                };
                push_lsp_completion(&*push, request_id, items, error);
            }
            HostCtl::LspHover {
                root,
                path,
                line,
                character,
                request_id,
            } => {
                let (hover, error) = match mgr.lock() {
                    Ok(mut g) => match g.hover(&root, &path, line, character) {
                        Ok(h) => (h, None),
                        Err(e) => (None, Some(e)),
                    },
                    Err(_) => (None, Some("lsp manager lock poisoned".into())),
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
                let (locations, error) = match mgr.lock() {
                    Ok(mut g) => match g.definition(&root, &path, line, character) {
                        Ok(locs) => (locs, None),
                        Err(e) => (Vec::new(), Some(e)),
                    },
                    Err(_) => (Vec::new(), Some("lsp manager lock poisoned".into())),
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
                let (locations, error) = match mgr.lock() {
                    Ok(mut g) => {
                        match g.references(&root, &path, line, character, include_declaration) {
                            Ok(locs) => (locs, None),
                            Err(e) => (Vec::new(), Some(e)),
                        }
                    }
                    Err(_) => (Vec::new(), Some("lsp manager lock poisoned".into())),
                };
                push_lsp_references(&*push, request_id, locations, error);
            }
            HostCtl::LspDocumentSymbol {
                root,
                path,
                request_id,
            } => {
                let (symbols, error) = match mgr.lock() {
                    Ok(mut g) => match g.document_symbols(&root, &path) {
                        Ok(syms) => (syms, None),
                        Err(e) => (Vec::new(), Some(e)),
                    },
                    Err(_) => (Vec::new(), Some("lsp manager lock poisoned".into())),
                };
                push_lsp_document_symbol(&*push, request_id, symbols, error);
            }
            _ => {}
        }
    });
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
            | HostCtl::LspHover { .. }
            | HostCtl::LspDefinition { .. }
            | HostCtl::LspReferences { .. }
            | HostCtl::LspDocumentSymbol { .. }
    )
}

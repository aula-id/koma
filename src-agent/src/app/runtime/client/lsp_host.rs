//! Host-side language-server status / install / uninstall for the GUI Settings
//! "Language servers" section.
//!
//! Entirely host-local (never the daemon), mirroring [`super::keys`]: a
//! `std::thread::spawn` worker does the blocking work (`reqwest`, npm, pip, go).
//!
//! Two flavors:
//! - **detached** (`host_swapper`): push replies straight through a cloned sink
//! - **attached** (`push_loop`): send replies over an `mpsc` channel drained by
//!   the fold loop (the fold's `push` is `&dyn Fn(String)` and cannot be cloned)

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

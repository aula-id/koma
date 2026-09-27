//! Each root owns its language processes, independently of chat attachments.
use super::WorkspaceRef;
use crate::app::runtime::client::{lsp_host, HostCtl};
use crate::lsp::LspManager;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

static MANAGERS: OnceLock<Mutex<HashMap<(WorkspaceRef, String), Arc<Mutex<LspManager>>>>> =
    OnceLock::new();

thread_local! { static CLIENT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) }; }
fn client_id() -> String {
    CLIENT.with(|v| v.borrow().clone())
}
pub(super) fn with_client<T>(client: &str, operation: impl FnOnce() -> T) -> T {
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            CLIENT.with(|v| *v.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let _restore = Restore(CLIENT.with(|v| v.replace(client.to_string())));
    operation()
}
#[derive(Deserialize)]
#[serde(tag = "r", rename_all_fields = "camelCase")]
enum Message {
    LspDidOpen {
        path: String,
        language_id: String,
        text: String,
    },
    LspDidChange {
        path: String,
        text: String,
    },
    LspDidSave {
        path: String,
        text: Option<String>,
    },
    LspDidClose {
        path: String,
    },
    LspCompletion {
        path: String,
        line: u32,
        character: u32,
        trigger_kind: u32,
        trigger_character: Option<String>,
        request_id: String,
    },
    LspCompletionResolve {
        path: String,
        item: Box<crate::lsp::LspCompletionItem>,
        request_id: String,
    },
    LspHover {
        path: String,
        line: u32,
        character: u32,
        request_id: String,
    },
    LspDefinition {
        path: String,
        line: u32,
        character: u32,
        request_id: String,
    },
    LspReferences {
        path: String,
        line: u32,
        character: u32,
        include_declaration: bool,
        request_id: String,
    },
    LspDocumentSymbol {
        path: String,
        request_id: String,
    },
}

pub(super) fn dispatch(workspace: &WorkspaceRef, body: &Value) -> Result<Value, String> {
    let msg: Message = serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
    let manager = manager(workspace)?;
    let root = workspace.root.clone();
    let ctl = match msg {
        Message::LspDidOpen {
            path,
            language_id,
            text,
        } => HostCtl::LspDidOpen {
            root,
            path,
            language_id,
            text,
        },
        Message::LspDidChange { path, text } => HostCtl::LspDidChange { root, path, text },
        Message::LspDidSave { path, text } => HostCtl::LspDidSave { root, path, text },
        Message::LspDidClose { path } => HostCtl::LspDidClose { root, path },
        Message::LspCompletion {
            path,
            line,
            character,
            trigger_kind,
            trigger_character,
            request_id,
        } => HostCtl::LspCompletion {
            root,
            path,
            line,
            character,
            trigger_kind,
            trigger_character,
            request_id,
        },
        Message::LspCompletionResolve {
            path,
            item,
            request_id,
        } => HostCtl::LspCompletionResolve {
            root,
            path,
            item,
            request_id,
        },
        Message::LspHover {
            path,
            line,
            character,
            request_id,
        } => HostCtl::LspHover {
            root,
            path,
            line,
            character,
            request_id,
        },
        Message::LspDefinition {
            path,
            line,
            character,
            request_id,
        } => HostCtl::LspDefinition {
            root,
            path,
            line,
            character,
            request_id,
        },
        Message::LspReferences {
            path,
            line,
            character,
            include_declaration,
            request_id,
        } => HostCtl::LspReferences {
            root,
            path,
            line,
            character,
            include_declaration,
            request_id,
        },
        Message::LspDocumentSymbol { path, request_id } => HostCtl::LspDocumentSymbol {
            root,
            path,
            request_id,
        },
    };
    // A notification ACK means the server pipe has received it. Query callers
    // can await the workspace lane instead of relying on timing sleeps.
    match ctl {
        HostCtl::LspDidOpen {
            root,
            path,
            language_id,
            text,
        } => manager
            .lock()
            .map_err(|_| "Coding LSP lock failed")?
            .did_open(&root, &path, &language_id, &text)?,
        HostCtl::LspDidChange { root, path, text } => manager
            .lock()
            .map_err(|_| "Coding LSP lock failed")?
            .did_change(&root, &path, &text)?,
        HostCtl::LspDidSave { root, path, text } => manager
            .lock()
            .map_err(|_| "Coding LSP lock failed")?
            .did_save(&root, &path, text.as_deref())?,
        HostCtl::LspDidClose { root, path } => manager
            .lock()
            .map_err(|_| "Coding LSP lock failed")?
            .did_close_path(&root, &path)?,
        ctl => lsp_host::handle_client_ctl(ctl, manager),
    }
    Ok(Value::Null)
}

fn manager(workspace: &WorkspaceRef) -> Result<Arc<Mutex<LspManager>>, String> {
    Ok({
        let mut managers = MANAGERS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "Coding LSP manager lock failed")?;
        let client_id = client_id();
        if managers.len() >= 128 && !managers.contains_key(&(workspace.clone(), client_id.clone()))
        {
            return Err("Too many window/workspace language sessions; close unused windows".into());
        }
        Arc::clone(managers.entry((workspace.clone(),client_id.clone())).or_insert_with(|| {
            let workspace = workspace.clone();
            Arc::new(Mutex::new(LspManager::new(move |json| {
                if let Ok(event) = serde_json::from_str::<Value>(&json) {
                    super::event(json!({"k":"CodingEvent","workspace":workspace,"clientId":client_id,"event":event}));
                }
            })))
        }))
    })
}

pub(super) fn query(
    workspace: &WorkspaceRef,
    path: &str,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    if method == "workspace/executeCommand" {
        return Err("Use the asynchronous language command operation".into());
    }
    let manager = manager(workspace)?;
    let pending = manager
        .lock()
        .map_err(|_| "Coding LSP manager lock failed")?
        .extended_request(&workspace.root, path, method, params.clone())?;
    let response = pending.wait_raw()?;
    let manager = manager
        .lock()
        .map_err(|_| "Coding LSP manager lock failed")?;
    if method == "textDocument/semanticTokens/full" {
        return Ok(
            json!({"legend":manager.semantic_legend(&workspace.root, path)?,"data":response.get("data")}),
        );
    }
    if method == "textDocument/rename" {
        manager.validate_edit_versions(&response)?;
    }
    if method == "codeAction/resolve" {
        if let Some(edit) = response.get("edit") {
            manager.validate_edit_versions(edit)?;
        }
    }
    if method == "textDocument/codeAction" {
        if let Some(actions) = response.as_array() {
            for action in actions {
                if let Some(edit) = action.get("edit") {
                    manager.validate_edit_versions(edit)?;
                }
            }
        }
    }
    Ok(response)
}

pub(super) fn shutdown() {
    if let Some(managers) = MANAGERS.get() {
        if let Ok(mut managers) = managers.lock() {
            for (_, manager) in managers.drain() {
                if let Ok(mut manager) = manager.lock() {
                    manager.cleanup_all();
                }
            }
        }
    }
}

pub(super) fn edit_preview(workspace: &WorkspaceRef, ticket: &str) -> Result<Value, String> {
    let value = crate::lsp::client::pending_workspace_edit(&workspace.root, ticket)?;
    manager(workspace)?
        .lock()
        .map_err(|_| "Coding LSP manager lock failed")?
        .validate_edit_versions(&value["edit"])?;
    Ok(value)
}
pub(super) fn command(
    workspace: &WorkspaceRef,
    path: &str,
    params: &Value,
) -> Result<Value, String> {
    static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let guard = ACTIVE
        .fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |n| if n < 8 { Some(n + 1) } else { None },
        )
        .map_err(|_| "Too many active language commands")?;
    let _ = guard;
    let result = (|| {
        let pending = manager(workspace)?
            .lock()
            .map_err(|_| "Coding LSP manager lock failed")?
            .extended_request(
                &workspace.root,
                path,
                "workspace/executeCommand",
                params.clone(),
            )?;
        let id = uuid::Uuid::new_v4().to_string();
        let id_copy = id.clone();
        let workspace = workspace.clone();
        let client_id = client_id();
        std::thread::Builder::new().name("coding-lsp-command".into()).spawn(move||{let result=pending.wait_raw();ACTIVE.fetch_sub(1,std::sync::atomic::Ordering::AcqRel);super::event(json!({"k":"CodingEvent","workspace":workspace,"clientId":client_id,"event":{"k":"LspCommandResult","id":id_copy,"error":result.err()}}));}).map_err(|e|e.to_string())?;
        Ok(json!({"id":id}))
    })();
    if result.is_err() {
        ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
    result
}

pub(super) fn restart(workspace: &WorkspaceRef) -> Result<Value, String> {
    let managers = {
        let mut all = MANAGERS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "Coding language registry failed")?;
        let keys: Vec<_> = all
            .keys()
            .filter(|(w, _)| w == workspace)
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|k| all.remove(&k))
            .collect::<Vec<_>>()
    };
    for manager in managers {
        manager
            .lock()
            .map_err(|_| "Coding LSP manager lock failed")?
            .cleanup_all();
    }
    super::event(
        json!({"k":"CodingEvent","workspace":workspace,"event":{"k":"LspWorkspaceRestart"}}),
    );
    Ok(Value::Null)
}

/// Forward filesystem hints only to an existing workspace manager.
pub(super) fn files_changed(workspace: &WorkspaceRef, changes: Vec<(std::path::PathBuf, u32)>) {
    if changes.is_empty() {
        return;
    }
    let managers = MANAGERS
        .get()
        .and_then(|all| {
            all.lock().ok().map(|all| {
                all.iter()
                    .filter(|((w, _), _)| w == workspace)
                    .map(|(_, m)| m.clone())
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();
    for manager in managers {
        if let Ok(manager) = manager.lock() {
            manager.did_change_watched_files(&changes);
        }
    }
}

pub(super) fn release(workspace: &WorkspaceRef) -> Result<Value, String> {
    let manager = MANAGERS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Language registry lock failed")?
        .remove(&(workspace.clone(), client_id()));
    if let Some(manager) = manager {
        manager
            .lock()
            .map_err(|_| "Language manager lock failed")?
            .cleanup_all();
    }
    Ok(Value::Null)
}

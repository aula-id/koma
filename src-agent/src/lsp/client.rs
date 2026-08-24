//! Host-spawned LSP JSON-RPC client (stdio, Content-Length framing).
//!
//! One process per catalogue server id, lazily started on first document open.
//! Completions / hover / definition / references / documentSymbol are
//! request/response; diagnostics arrive as
//! `textDocument/publishDiagnostics` notifications and are pushed to the GUI.
//!
//! Threading mirrors [`crate::app::runtime::client::terminal_host`]: a reader
//! thread owns stdout, the control loop (via [`LspManager`]) owns stdin writes
//! under a mutex. No dependency on tower-lsp / lsp-types — wire shapes are
//! hand-rolled `serde_json::Value`s for the small v1 surface.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::catalog::{self, ServerSpec};
use super::resolve::{self, Source};

/// Default timeout for LSP request/response round-trips.
const REQ_TIMEOUT: Duration = Duration::from_secs(15);

/// One diagnostic pushed to the GUI (Monaco markers + Problems drawer).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspDiagnostic {
    pub uri: String,
    /// 0-based line.
    pub line: u32,
    /// 0-based character.
    pub character: u32,
    /// 0-based end line.
    pub end_line: u32,
    /// 0-based end character.
    pub end_character: u32,
    /// LSP DiagnosticSeverity: 1=Error 2=Warning 3=Info 4=Hint.
    pub severity: u8,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// One text edit (0-based range) — used for completion `additionalTextEdits`
/// (auto-import lines) and primary `textEdit`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspTextEdit {
    pub range: LspRange,
    pub new_text: String,
}

/// One completion item for Monaco.
///
/// Carries enough fields for auto-import: `additionalTextEdits` (often filled
/// only after `completionItem/resolve`) and opaque `data` for the resolve
/// round-trip. `serde::Deserialize` so the GUI can send the item back on resolve.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspCompletionItem {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Secondary label line (e.g. module path from `labelDetails.description`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    /// LSP InsertTextFormat: 1=PlainText, 2=Snippet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text_format: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_text: Option<String>,
    /// Primary text edit (preferred over insert_text when present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_edit: Option<LspTextEdit>,
    /// Extra edits applied on accept — typically auto-import statements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_text_edits: Option<Vec<LspTextEdit>>,
    /// Opaque server token required for `completionItem/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Hover payload for Monaco.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspHover {
    pub contents: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<LspRange>,
}

/// A 0-based range in a document.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

/// Go-to-definition / references location.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

/// One document symbol (flattened; nested children expanded by the client).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspDocumentSymbol {
    pub name: String,
    /// LSP SymbolKind (1..26).
    pub kind: u32,
    pub range: LspRange,
    pub selection_range: LspRange,
}

/// Live runtime row for the footer Language Servers drawer.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspRuntimeServer {
    /// Session key (catalogue id, or `vscode-langservers:<bin>`).
    pub id: String,
    /// Human label for the drawer.
    pub name: String,
    /// Absolute workspace root this process was initialized for.
    pub root: String,
    /// `starting` | `ready` | `working` | `error`
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u8>,
    /// Open documents currently routed to this session.
    pub open_docs: u32,
}

/// In-flight `$/progress` work-done payload (shared with the reader thread).
#[derive(Debug, Clone, Default)]
struct WorkProgress {
    title: Option<String>,
    message: Option<String>,
    percentage: Option<u8>,
    /// True while a begin..end workDone sequence is open.
    active: bool,
}

/// Mutable runtime projection shared between the control loop and reader.
#[derive(Debug, Clone)]
struct RuntimeState {
    phase: String,
    progress: WorkProgress,
    error: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            phase: "starting".into(),
            progress: WorkProgress::default(),
            error: None,
        }
    }
}

/// Internal reply from the reader thread to a waiting request.
enum PendingReply {
    Ok(serde_json::Value),
    Err(String),
}

#[derive(Clone)]
struct OpenDoc {
    server_id: String,
    /// Absolute workspace root the doc was opened under (for revive).
    root: PathBuf,
    /// LSP languageId sent on didOpen (used to detect language switches).
    language_id: String,
    version: i32,
    /// Last known full text — needed to re-didOpen after a crash/restart.
    text: String,
}

struct ServerSession {
    /// Catalogue / spawn id (e.g. `rust-analyzer`).
    id: String,
    /// Display name for the footer drawer.
    name: String,
    /// Workspace root this process was initialized for (absolute).
    root: PathBuf,
    child: Child,
    /// Clonable IO handle — request/notify only need this (not the Child).
    io: SessionIo,
    /// Live phase + `$/progress` (reader + control loop).
    runtime: Arc<Mutex<RuntimeState>>,
}

/// Cheap clone of the pieces needed to talk to a live server without holding
/// `LspManager`'s mutex across the blocking `request` round-trip.
#[derive(Clone)]
struct SessionIo {
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<u64, Sender<PendingReply>>>>,
}

/// In-flight LSP request handle: caller resolves `SessionIo` under the manager
/// lock, drops the lock, then waits on this handle. Prevents CodeLens/references
/// from serializing hover/completion behind a single `Mutex<LspManager>`.
pub struct LspPendingRequest {
    io: SessionIo,
    method: &'static str,
    params: serde_json::Value,
}

impl LspPendingRequest {
    pub fn wait(self) -> Result<serde_json::Value, String> {
        self.io.request(self.method, self.params)
    }
}

/// Host-owned map of live language servers + open documents.
pub struct LspManager {
    servers: HashMap<String, ServerSession>,
    /// file URI → open doc metadata.
    docs: HashMap<String, OpenDoc>,
    /// Push sink for diagnostics (and future server-log envelopes).
    push: Arc<dyn Fn(String) + Send + Sync>,
}

impl LspManager {
    pub fn new(push: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            servers: HashMap::new(),
            docs: HashMap::new(),
            push: Arc::new(push),
        }
    }

    /// Kill every live server. Called on host-relay teardown.
    pub fn cleanup_all(&mut self) {
        let uris: Vec<String> = self.docs.keys().cloned().collect();
        for uri in uris {
            self.did_close(&uri);
        }
        for (_, mut s) in self.servers.drain() {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
        // Clear the footer Language Servers list.
        push_runtime_snapshot(&*self.push, &[], true, &[]);
    }

    /// Push a full live-server snapshot (open-doc counts included).
    fn emit_runtime(&self) {
        let servers = self.runtime_rows();
        push_runtime_snapshot(&*self.push, &servers, true, &[]);
    }

    fn runtime_rows(&self) -> Vec<LspRuntimeServer> {
        let mut out = Vec::with_capacity(self.servers.len());
        for (id, session) in &self.servers {
            let open_docs = self
                .docs
                .values()
                .filter(|d| d.server_id == *id)
                .count() as u32;
            out.push(session.to_runtime_row(open_docs));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        out
    }

    /// Clone of the GUI push sink (for request replies from worker threads).
    pub fn push_sink(&self) -> Arc<dyn Fn(String) + Send + Sync> {
        Arc::clone(&self.push)
    }

    /// `textDocument/didOpen` — start the matching server lazily if needed.
    pub fn did_open(
        &mut self,
        root: &str,
        path: &str,
        language_id: &str,
        text: &str,
    ) -> Result<(), String> {
        let abs = abs_path(root, path)?;
        let uri = path_to_uri(&abs);
        // Prefer extension-derived LSP languageId (tsx → typescriptreact). The
        // GUI Monarch map uses "typescript" for highlighting and must not win
        // for didOpen — vtsls typechecks JSX as errors under plain typescript.
        let derived = language_id_for_path(path);
        let language_id = if derived != "plaintext" {
            derived
        } else if !language_id.is_empty() {
            language_id
        } else {
            derived
        };
        if let Some(existing) = self.docs.get(&uri) {
            // Same language: treat as a full-document change. Language switch:
            // close and fall through so the server sees a fresh didOpen.
            if existing.language_id == language_id {
                return self.did_change(root, path, text);
            }
            self.did_close(&uri);
        }

        let ext = abs
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let spec = catalog::find_by_extension(&ext)
            .ok_or_else(|| format!("no language server for .{ext}"))?;
        let (spawn_id, binary, args) = resolve_spawn(spec, &ext)?;
        let root_path = PathBuf::from(root);
        if !root_path.is_absolute() {
            return Err("workspace root must be absolute".into());
        }

        // Dead/zombie session: revive (re-opens sibling docs) or just free the slot.
        if self.servers.get(&spawn_id).is_some_and(|s| s.is_dead()) {
            let has_siblings = self.docs.values().any(|d| d.server_id == spawn_id);
            if has_siblings {
                self.revive_server(&spawn_id)?;
            } else {
                self.drop_dead_server(&spawn_id);
            }
        }

        if let Some(session) = self.servers.get(&spawn_id) {
            // One process per (server, root). Refuse silently-wrong roots.
            if session.root != root_path {
                return Err(format!(
                    "LSP {} already running for {} (requested {})",
                    session.id,
                    session.root.display(),
                    root_path.display()
                ));
            }
        } else {
            self.spawn_into_slot(&spawn_id, spec, &binary, &args, &root_path)?;
        }

        let version = 1;
        {
            let session = self
                .servers
                .get_mut(&spawn_id)
                .ok_or_else(|| "server missing after spawn".to_string())?;
            let params = serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            });
            session.notify("textDocument/didOpen", params)?;
        }

        self.docs.insert(
            uri,
            OpenDoc {
                server_id: spawn_id,
                root: root_path,
                language_id: language_id.to_string(),
                version,
                text: text.to_string(),
            },
        );
        self.emit_runtime();
        Ok(())
    }

    /// Full-document `textDocument/didChange`.
    pub fn did_change(&mut self, root: &str, path: &str, text: &str) -> Result<(), String> {
        let abs = abs_path(root, path)?;
        let uri = path_to_uri(&abs);
        self.ensure_server_alive_for_uri(&uri)?;
        let doc = self
            .docs
            .get_mut(&uri)
            .ok_or_else(|| "document not open".to_string())?;
        doc.version = doc.version.saturating_add(1);
        doc.text = text.to_string();
        let version = doc.version;
        let server_id = doc.server_id.clone();
        let session = self
            .servers
            .get_mut(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": text }],
        });
        session.notify("textDocument/didChange", params)
    }

    /// `textDocument/didSave`.
    pub fn did_save(&mut self, root: &str, path: &str, text: Option<&str>) -> Result<(), String> {
        let abs = abs_path(root, path)?;
        let uri = path_to_uri(&abs);
        if let Some(t) = text {
            if let Some(doc) = self.docs.get_mut(&uri) {
                doc.text = t.to_string();
            }
        }
        self.ensure_server_alive_for_uri(&uri)?;
        let server_id = self
            .docs
            .get(&uri)
            .map(|d| d.server_id.clone())
            .ok_or_else(|| "document not open".to_string())?;
        let session = self
            .servers
            .get_mut(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        let mut params = serde_json::Map::new();
        params.insert("textDocument".into(), serde_json::json!({ "uri": uri }));
        if let Some(t) = text {
            params.insert("text".into(), serde_json::Value::String(t.to_string()));
        }
        session.notify("textDocument/didSave", serde_json::Value::Object(params))
    }

    /// `textDocument/didClose` by file URI or root+path.
    pub fn did_close_path(&mut self, root: &str, path: &str) -> Result<(), String> {
        let abs = abs_path(root, path)?;
        let uri = path_to_uri(&abs);
        self.did_close(&uri);
        Ok(())
    }

    pub fn did_close(&mut self, uri: &str) {
        let Some(doc) = self.docs.remove(uri) else {
            return;
        };
        if let Some(session) = self.servers.get_mut(&doc.server_id) {
            if !session.is_dead() {
                let params = serde_json::json!({
                    "textDocument": { "uri": uri }
                });
                let _ = session.notify("textDocument/didClose", params);
            }
        }
        // Clear markers for this URI.
        push_diagnostics(&*self.push, uri, Vec::new());
        // Keep the runtime row (server stays warm) but refresh open-doc counts.
        self.emit_runtime();
    }

    /// `textDocument/completion`.
    ///
    /// Returns a pending request — caller must `.wait()` **after** dropping
    /// the `LspManager` lock so hover/didChange are not serialized behind RPC.
    pub fn completion(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
    ) -> Result<LspPendingRequest, String> {
        let (uri, io) = self.uri_io(root, path)?;
        Ok(LspPendingRequest {
            io,
            method: "textDocument/completion",
            params: serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "triggerKind": 1 }
            }),
        })
    }

    /// `completionItem/resolve` — fills `additionalTextEdits` (auto-import) etc.
    pub fn resolve_completion(
        &mut self,
        root: &str,
        path: &str,
        item: &LspCompletionItem,
    ) -> Result<LspPendingRequest, String> {
        let (_uri, io) = self.uri_io(root, path)?;
        // Rebuild a minimal CompletionItem the server can resolve. `data` is the
        // critical opaque token for vtsls/tsserver auto-import.
        let mut params = serde_json::json!({
            "label": item.label,
        });
        if let Some(obj) = params.as_object_mut() {
            if let Some(k) = item.kind {
                obj.insert("kind".into(), serde_json::json!(k));
            }
            if let Some(ref d) = item.detail {
                obj.insert("detail".into(), serde_json::Value::String(d.clone()));
            }
            if let Some(ref t) = item.insert_text {
                obj.insert("insertText".into(), serde_json::Value::String(t.clone()));
            }
            if let Some(f) = item.insert_text_format {
                obj.insert("insertTextFormat".into(), serde_json::json!(f));
            }
            if let Some(ref s) = item.sort_text {
                obj.insert("sortText".into(), serde_json::Value::String(s.clone()));
            }
            if let Some(ref f) = item.filter_text {
                obj.insert("filterText".into(), serde_json::Value::String(f.clone()));
            }
            if let Some(ref data) = item.data {
                obj.insert("data".into(), data.clone());
            }
            if let Some(ref te) = item.text_edit {
                obj.insert(
                    "textEdit".into(),
                    serde_json::json!({
                        "range": {
                            "start": {
                                "line": te.range.start_line,
                                "character": te.range.start_character
                            },
                            "end": {
                                "line": te.range.end_line,
                                "character": te.range.end_character
                            }
                        },
                        "newText": te.new_text,
                    }),
                );
            }
        }
        Ok(LspPendingRequest {
            io,
            method: "completionItem/resolve",
            params,
        })
    }

    /// `textDocument/hover`.
    pub fn hover(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
    ) -> Result<LspPendingRequest, String> {
        let (uri, io) = self.uri_io(root, path)?;
        Ok(LspPendingRequest {
            io,
            method: "textDocument/hover",
            params: serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
        })
    }

    /// `textDocument/definition`.
    pub fn definition(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
    ) -> Result<LspPendingRequest, String> {
        let (uri, io) = self.uri_io(root, path)?;
        Ok(LspPendingRequest {
            io,
            method: "textDocument/definition",
            params: serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
        })
    }

    /// `textDocument/references`.
    pub fn references(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<LspPendingRequest, String> {
        let (uri, io) = self.uri_io(root, path)?;
        Ok(LspPendingRequest {
            io,
            method: "textDocument/references",
            params: serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": include_declaration },
            }),
        })
    }

    /// `textDocument/documentSymbol` — flattened list (children expanded).
    pub fn document_symbols(
        &mut self,
        root: &str,
        path: &str,
    ) -> Result<LspPendingRequest, String> {
        let (uri, io) = self.uri_io(root, path)?;
        Ok(LspPendingRequest {
            io,
            method: "textDocument/documentSymbol",
            params: serde_json::json!({
                "textDocument": { "uri": uri },
            }),
        })
    }

    /// Resolve URI + clone SessionIo so the caller can drop `LspManager` before
    /// the blocking request wait. Also revives a dead server if needed.
    fn uri_io(&mut self, root: &str, path: &str) -> Result<(String, SessionIo), String> {
        let (uri, server_id) = self.uri_server(root, path)?;
        self.ensure_server_alive(&server_id)?;
        let session = self
            .servers
            .get(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        Ok((uri, session.io.clone()))
    }

    fn uri_server(&self, root: &str, path: &str) -> Result<(String, String), String> {
        let abs = abs_path(root, path)?;
        let uri = path_to_uri(&abs);
        let doc = self
            .docs
            .get(&uri)
            .ok_or_else(|| "document not open in LSP".to_string())?;
        Ok((uri, doc.server_id.clone()))
    }

    /// Spawn a fresh server into `servers[spawn_id]` (slot must be empty).
    fn spawn_into_slot(
        &mut self,
        spawn_id: &str,
        spec: &ServerSpec,
        binary: &Path,
        args: &[&str],
        root_path: &Path,
    ) -> Result<(), String> {
        let display = display_name_for(spawn_id, spec);
        push_runtime_snapshot(
            &*self.push,
            &[LspRuntimeServer {
                id: spawn_id.to_string(),
                name: display.clone(),
                root: root_path.to_string_lossy().into_owned(),
                phase: "starting".into(),
                title: Some("Starting".into()),
                message: None,
                percentage: None,
                open_docs: 0,
            }],
            false,
            &[],
        );
        let session = match spawn_server(
            spawn_id,
            display,
            binary,
            args,
            root_path,
            Arc::clone(&self.push),
        ) {
            Ok(s) => s,
            Err(e) => {
                push_runtime_snapshot(
                    &*self.push,
                    &[LspRuntimeServer {
                        id: spawn_id.to_string(),
                        name: display_name_for(spawn_id, spec),
                        root: root_path.to_string_lossy().into_owned(),
                        phase: "error".into(),
                        title: Some("Failed to start".into()),
                        message: Some(e.clone()),
                        percentage: None,
                        open_docs: 0,
                    }],
                    false,
                    &[],
                );
                return Err(e);
            }
        };
        self.servers.insert(spawn_id.to_string(), session);
        self.emit_runtime();
        Ok(())
    }

    /// Kill + remove a dead session. Docs stay so revive can re-open them.
    fn drop_dead_server(&mut self, server_id: &str) {
        if let Some(mut session) = self.servers.remove(server_id) {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
        push_runtime_snapshot(&*self.push, &[], false, &[server_id]);
    }

    fn ensure_server_alive_for_uri(&mut self, uri: &str) -> Result<(), String> {
        let server_id = self
            .docs
            .get(uri)
            .map(|d| d.server_id.clone())
            .ok_or_else(|| "document not open".to_string())?;
        self.ensure_server_alive(&server_id)
    }

    /// If the session is dead (or missing), respawn and re-didOpen all its docs.
    ///
    /// **Known stall:** revive/spawn runs `initialize` under the caller's
    /// `Mutex<LspManager>` (via `uri_io`). First open and server revive still
    /// serialize every LSP feature for the whole handshake (seconds for
    /// rust-analyzer). Request wait itself is unlocked via [`LspPendingRequest`];
    /// hoisting spawn out of the lock is a follow-up.
    fn ensure_server_alive(&mut self, server_id: &str) -> Result<(), String> {
        let dead = match self.servers.get(server_id) {
            Some(s) => s.is_dead(),
            None => true,
        };
        if !dead {
            return Ok(());
        }
        self.revive_server(server_id)
    }

    fn revive_server(&mut self, server_id: &str) -> Result<(), String> {
        // Snapshot docs belonging to this server before we drop the slot.
        let docs: Vec<(String, OpenDoc)> = self
            .docs
            .iter()
            .filter(|(_, d)| d.server_id == server_id)
            .map(|(uri, d)| (uri.clone(), d.clone()))
            .collect();
        if docs.is_empty() {
            // No open docs — just clear the zombie so a later didOpen can spawn.
            if self.servers.contains_key(server_id) {
                self.drop_dead_server(server_id);
            }
            return Err("server not running".into());
        }

        let root = docs[0].1.root.clone();
        // Derive extension from first doc URI path for spawn resolve.
        let sample_path = uri_to_path_lossy(&docs[0].0);
        let ext = Path::new(&sample_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let spec = catalog::find(server_id)
            .or_else(|| catalog::find_by_extension(&ext))
            .ok_or_else(|| format!("no language server for {server_id}"))?;
        let (_resolved_id, binary, args) = resolve_spawn(spec, &ext)?;
        // Always reinsert under the original `server_id` key docs already reference
        // (vscode-langservers multi-bin keeps the same catalogue id).

        if self.servers.contains_key(server_id) {
            self.drop_dead_server(server_id);
        }
        self.spawn_into_slot(server_id, spec, &binary, &args, &root)?;

        // Re-open every previously tracked document with its last known text.
        for (uri, doc) in docs {
            let version = doc.version.max(1);
            {
                let session = self
                    .servers
                    .get_mut(server_id)
                    .ok_or_else(|| "server missing after revive".to_string())?;
                let params = serde_json::json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": doc.language_id,
                        "version": version,
                        "text": doc.text,
                    }
                });
                session.notify("textDocument/didOpen", params)?;
            }
            if let Some(d) = self.docs.get_mut(&uri) {
                d.version = version;
                d.server_id = server_id.to_string();
            }
        }
        self.emit_runtime();
        Ok(())
    }
}


impl ServerSession {
    /// True when the reader marked the session dead or the OS process has exited.
    fn is_dead(&self) -> bool {
        {
            let st = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
            if st.phase == "error" {
                return true;
            }
        }
        // Child already reaped by OS? try_wait needs &mut — use a non-blocking check
        // via try_wait on a duplicated approach: lock isn't needed; Child::try_wait
        // requires &mut self, so we only trust the runtime phase from the reader.
        false
    }

    fn to_runtime_row(&self, open_docs: u32) -> LspRuntimeServer {
        let state = self
            .runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let (title, message, percentage) = if state.phase == "working" || state.progress.active {
            (
                state.progress.title.clone(),
                state.progress.message.clone(),
                state.progress.percentage,
            )
        } else if state.phase == "error" {
            (
                Some("Stopped".into()),
                state.error.clone(),
                None,
            )
        } else if state.phase == "starting" {
            (Some("Starting".into()), None, None)
        } else {
            (None, None, None)
        };
        let phase = if state.progress.active && state.phase != "error" {
            "working".to_string()
        } else {
            state.phase
        };
        LspRuntimeServer {
            id: self.id.clone(),
            name: self.name.clone(),
            root: self.root.to_string_lossy().into_owned(),
            phase,
            title,
            message,
            percentage,
            open_docs,
        }
    }

    fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), String> {
        self.io.notify(method, params)
    }

    fn request(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.io.request(method, params)
    }
}

impl SessionIo {
    fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), String> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        write_message(&self.stdin, &msg)
    }

    fn request(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx): (Sender<PendingReply>, Receiver<PendingReply>) = mpsc::channel();
        {
            let mut map = self
                .pending
                .lock()
                .map_err(|_| "pending lock poisoned".to_string())?;
            map.insert(id, tx);
        }
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(e) = write_message(&self.stdin, &msg) {
            let mut map = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            map.remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(REQ_TIMEOUT) {
            Ok(PendingReply::Ok(v)) => Ok(v),
            Ok(PendingReply::Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => {
                let mut map = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                map.remove(&id);
                Err(format!("LSP {method} timed out"))
            }
            Err(RecvTimeoutError::Disconnected) => Err("LSP server closed".into()),
        }
    }
}

fn write_message(stdin: &Arc<Mutex<ChildStdin>>, msg: &serde_json::Value) -> Result<(), String> {
    let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut guard = stdin
        .lock()
        .map_err(|_| "stdin lock poisoned".to_string())?;
    guard
        .write_all(header.as_bytes())
        .and_then(|_| guard.write_all(&body))
        .and_then(|_| guard.flush())
        .map_err(|e| format!("LSP stdin write failed: {e}"))
}

fn spawn_server(
    id: &str,
    name: String,
    binary: &Path,
    args: &[&str],
    root: &Path,
    push: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<ServerSession, String> {
    let mut cmd = Command::new(binary);
    for a in args {
        cmd.arg(a);
    }
    cmd.current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Avoid Windows console flash (same helper family as MCP/sec).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {} failed: {e}", binary.display()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "child stdin missing".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout missing".to_string())?;
    let stderr = child.stderr.take();

    let stdin = Arc::new(Mutex::new(stdin));
    let pending: Arc<Mutex<HashMap<u64, Sender<PendingReply>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let runtime = Arc::new(Mutex::new(RuntimeState::default()));
    let pending_reader = Arc::clone(&pending);
    let push_reader = Arc::clone(&push);
    let stdin_reader = Arc::clone(&stdin);
    let runtime_reader = Arc::clone(&runtime);
    let stdin_init = Arc::clone(&stdin);
    let id_reader = id.to_string();
    let name_reader = name.clone();
    let root_reader = root.to_string_lossy().into_owned();

    // stderr drain so a chatty server never blocks.
    if let Some(stderr) = stderr {
        std::thread::Builder::new()
            .name(format!("lsp-err-{id}"))
            .spawn(move || {
                let mut r = BufReader::new(stderr);
                let mut buf = String::new();
                while r.read_line(&mut buf).ok().filter(|n| *n > 0).is_some() {
                    buf.clear();
                }
            })
            .ok();
    }

    // Reader thread: Content-Length frames → pending map, diagnostics, progress.
    std::thread::Builder::new()
        .name(format!("lsp-out-{id}"))
        .spawn(move || {
            reader_loop(
                stdout,
                ReaderCtx {
                    pending: pending_reader,
                    push: push_reader,
                    stdin: stdin_reader,
                    runtime: runtime_reader,
                    server_id: id_reader,
                    server_name: name_reader,
                    server_root: root_reader,
                },
            );
        })
        .map_err(|e| format!("lsp reader spawn: {e}"))?;

    let session = ServerSession {
        id: id.to_string(),
        name,
        root: root.to_path_buf(),
        child,
        io: SessionIo {
            stdin: stdin_init,
            next_id: Arc::new(AtomicU64::new(1)),
            pending,
        },
        runtime,
    };

    // initialize → initialized
    let root_uri = path_to_uri(root);
    let init_params = serde_json::json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "koma", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "rootPath": root.to_string_lossy(),
        "capabilities": {
            "textDocument": {
                "synchronization": {
                    "dynamicRegistration": false,
                    "willSave": false,
                    "willSaveWaitUntil": false,
                    "didSave": true
                },
                "completion": {
                    "completionItem": {
                        "snippetSupport": true,
                        "commitCharactersSupport": true,
                        "documentationFormat": ["plaintext", "markdown"],
                        "deprecatedSupport": true,
                        "preselectSupport": true,
                        "insertReplaceSupport": false,
                        "labelDetailsSupport": true,
                        "resolveSupport": {
                            "properties": [
                                "documentation",
                                "detail",
                                "additionalTextEdits",
                                "insertText",
                                "insertTextFormat",
                                "command"
                            ]
                        }
                    },
                    "contextSupport": true,
                    "completionItemKind": { "valueSet": null }
                },
                "hover": {
                    "contentFormat": ["plaintext", "markdown"]
                },
                "definition": {
                    "linkSupport": false
                },
                "references": {
                    "dynamicRegistration": false
                },
                "documentSymbol": {
                    "hierarchicalDocumentSymbolSupport": true
                },
                "publishDiagnostics": {
                    "relatedInformation": false,
                    "versionSupport": false
                }
            },
            "window": {
                "workDoneProgress": true
            },
            "workspace": {
                "workspaceFolders": true
            }
        },
        "workspaceFolders": [{
            "uri": root_uri,
            "name": root.file_name().and_then(|s| s.to_str()).unwrap_or("workspace")
        }],
        "initializationOptions": {}
    });
    let _caps = session.request("initialize", init_params)?;
    session.notify("initialized", serde_json::json!({}))?;
    {
        let mut st = session.runtime.lock().unwrap_or_else(|p| p.into_inner());
        st.phase = "ready".into();
        st.error = None;
    }

    Ok(session)
}

struct ReaderCtx {
    pending: Arc<Mutex<HashMap<u64, Sender<PendingReply>>>>,
    push: Arc<dyn Fn(String) + Send + Sync>,
    stdin: Arc<Mutex<ChildStdin>>,
    runtime: Arc<Mutex<RuntimeState>>,
    server_id: String,
    server_name: String,
    server_root: String,
}

fn reader_loop<R: Read>(stdout: R, ctx: ReaderCtx) {
    let ReaderCtx {
        pending,
        push,
        stdin,
        runtime,
        server_id,
        server_name,
        server_root,
    } = ctx;
    let mut reader = BufReader::new(stdout);
    loop {
        let body = match read_frame(&mut reader) {
            Ok(Some(b)) => b,
            Ok(None) => break,
            Err(_) => break,
        };
        let msg: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Response to a request we sent — OR a server→client request (method + id).
        if let Some(id_val) = msg.get("id").cloned() {
            if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                // Server→client request. Acknowledge workDoneProgress/create; ignore rest.
                let result = if method == "window/workDoneProgress/create" {
                    serde_json::Value::Null
                } else {
                    // Unsupported server request — null result is the least-bad ack.
                    serde_json::Value::Null
                };
                let reply = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": result,
                });
                let _ = write_message(&stdin, &reply);
                continue;
            }
            let id = match id_val {
                serde_json::Value::Number(n) => n
                    .as_u64()
                    .or_else(|| n.as_i64().map(|i| i as u64)),
                serde_json::Value::String(s) => s.parse().ok(),
                _ => None,
            };
            let Some(id) = id else { continue };
            let tx = {
                let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
                map.remove(&id)
            };
            if let Some(tx) = tx {
                if let Some(err) = msg.get("error") {
                    let msg = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("LSP error")
                        .to_string();
                    let _ = tx.send(PendingReply::Err(msg));
                } else {
                    let result = msg.get("result").cloned().unwrap_or(serde_json::Value::Null);
                    let _ = tx.send(PendingReply::Ok(result));
                }
            }
            continue;
        }

        // Notification from server.
        if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
            match method {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = msg.get("params") {
                        handle_publish_diagnostics(params, &*push);
                    }
                }
                "$/progress" => {
                    if let Some(params) = msg.get("params") {
                        handle_progress(
                            params,
                            &runtime,
                            &*push,
                            &server_id,
                            &server_name,
                            &server_root,
                        );
                    }
                }
                _ => {
                    // window/logMessage, telemetry/event, etc. — ignore.
                }
            }
        }
    }

    // Fail every waiter so nobody hangs after crash/EOF.
    let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
    for (_, tx) in map.drain() {
        let _ = tx.send(PendingReply::Err("LSP server closed".into()));
    }
    // Mark this server dead in the footer (control loop may still hold the slot
    // until the next did_* call; surface the death immediately).
    {
        let mut st = runtime.lock().unwrap_or_else(|p| p.into_inner());
        st.phase = "error".into();
        st.error = Some("server closed".into());
        st.progress = WorkProgress::default();
    }
    push_runtime_snapshot(
        &*push,
        &[LspRuntimeServer {
            id: server_id,
            name: server_name,
            root: server_root,
            phase: "error".into(),
            title: Some("Stopped".into()),
            message: Some("server closed".into()),
            percentage: None,
            open_docs: 0,
        }],
        false,
        &[],
    );
}

fn handle_progress(
    params: &serde_json::Value,
    runtime: &Mutex<RuntimeState>,
    push: &dyn Fn(String),
    server_id: &str,
    server_name: &str,
    server_root: &str,
) {
    let value = match params.get("value") {
        Some(v) => v,
        None => return,
    };
    let kind = value.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    {
        let mut st = runtime.lock().unwrap_or_else(|p| p.into_inner());
        match kind {
            "begin" => {
                st.progress.active = true;
                st.progress.title = value
                    .get("title")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());
                st.progress.message = value
                    .get("message")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());
                st.progress.percentage = value
                    .get("percentage")
                    .and_then(|p| p.as_u64())
                    .map(|n| n.min(100) as u8);
                if st.phase != "error" {
                    st.phase = "working".into();
                }
            }
            "report" => {
                if let Some(m) = value.get("message").and_then(|t| t.as_str()) {
                    st.progress.message = Some(m.to_string());
                }
                if let Some(p) = value.get("percentage").and_then(|p| p.as_u64()) {
                    st.progress.percentage = Some(p.min(100) as u8);
                }
                if st.phase != "error" {
                    st.progress.active = true;
                    st.phase = "working".into();
                }
            }
            "end" => {
                if let Some(m) = value.get("message").and_then(|t| t.as_str()) {
                    st.progress.message = Some(m.to_string());
                }
                st.progress.active = false;
                st.progress.percentage = None;
                if st.phase != "error" {
                    st.phase = "ready".into();
                }
                // Keep last title briefly visible via message only.
                st.progress.title = None;
            }
            _ => return,
        }
    }
    let row = {
        let st = runtime.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let phase = if st.progress.active && st.phase != "error" {
            "working".to_string()
        } else {
            st.phase.clone()
        };
        LspRuntimeServer {
            id: server_id.to_string(),
            name: server_name.to_string(),
            root: server_root.to_string(),
            phase,
            title: st.progress.title.clone().or_else(|| {
                if st.phase == "error" {
                    Some("Error".into())
                } else {
                    None
                }
            }),
            message: st.progress.message.clone().or(st.error.clone()),
            percentage: st.progress.percentage,
            // Reader doesn't know open-doc count; GUI keeps previous value on merge.
            open_docs: 0,
        }
    };
    // Partial update — GUI merges by id and preserves openDocs when 0 arrives
    // from the reader path (see store reducer).
    push_runtime_snapshot(push, &[row], false, &[]);
}

fn read_frame<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("LSP header read: {e}"))?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length.ok_or_else(|| "LSP frame missing Content-Length".to_string())?;
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .map_err(|e| format!("LSP body read: {e}"))?;
    Ok(Some(buf))
}

fn handle_publish_diagnostics(params: &serde_json::Value, push: &dyn Fn(String)) {
    let uri = params
        .get("uri")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    if uri.is_empty() {
        return;
    }
    let empty = Vec::new();
    let arr = params
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .unwrap_or(&empty);
    let mut out = Vec::with_capacity(arr.len());
    for d in arr {
        let range = d.get("range");
        let start = range.and_then(|r| r.get("start"));
        let end = range.and_then(|r| r.get("end"));
        let line = start
            .and_then(|s| s.get("line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let character = start
            .and_then(|s| s.get("character"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let end_line = end
            .and_then(|s| s.get("line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(line as u64) as u32;
        let end_character = end
            .and_then(|s| s.get("character"))
            .and_then(|v| v.as_u64())
            .unwrap_or(character as u64) as u32;
        let severity = d
            .get("severity")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u8;
        let message = d
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        if message.is_empty() {
            continue;
        }
        let source = d
            .get("source")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let code = match d.get("code") {
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        out.push(LspDiagnostic {
            uri: uri.clone(),
            line,
            character,
            end_line,
            end_character,
            severity,
            message,
            source,
            code,
        });
    }
    push_diagnostics(push, &uri, out);
}

/// Emit diagnostics via the shared push_proto helper when available; otherwise a
/// raw envelope (used during unit-less bring-up). The real helper lives in
/// `push_proto` and is called from a thin wrapper in `lsp_host` / here via JSON.
pub fn push_diagnostics(push: &dyn Fn(String), uri: &str, diagnostics: Vec<LspDiagnostic>) {
    let env = serde_json::json!({
        "k": "LspDiagnostics",
        "uri": uri,
        "diagnostics": diagnostics,
    });
    if let Ok(s) = serde_json::to_string(&env) {
        push(s);
    }
}

/// Emit a Language Servers runtime snapshot / delta for the footer drawer.
fn push_runtime_snapshot(
    push: &dyn Fn(String),
    servers: &[LspRuntimeServer],
    replace: bool,
    removed: &[&str],
) {
    let env = serde_json::json!({
        "k": "LspRuntime",
        "servers": servers,
        "replace": replace,
        "removed": removed,
    });
    if let Ok(s) = serde_json::to_string(&env) {
        push(s);
    }
}

fn display_name_for(spawn_id: &str, spec: &ServerSpec) -> String {
    if let Some(rest) = spawn_id.strip_prefix("vscode-langservers:") {
        return match rest {
            "vscode-html-language-server" => "HTML Language Server".into(),
            "vscode-css-language-server" => "CSS Language Server".into(),
            "vscode-json-language-server" => "JSON Language Server".into(),
            other => other.to_string(),
        };
    }
    spec.name.to_string()
}

// ─── Spawn resolution ────────────────────────────────────────────────────────

/// Pick binary + args for a catalogue entry. Multi-binary packages (vscode-langservers)
/// select the right binary from the file extension.
fn resolve_spawn(
    spec: &ServerSpec,
    ext: &str,
) -> Result<(String, PathBuf, Vec<&'static str>), String> {
    let (binary_name, args): (&str, &[&str]) = if spec.id == "vscode-langservers" {
        match ext {
            "html" | "htm" | "xhtml" => ("vscode-html-language-server", &["--stdio"]),
            "css" | "scss" | "less" => ("vscode-css-language-server", &["--stdio"]),
            _ => ("vscode-json-language-server", &["--stdio"]),
        }
    } else {
        (spec.binary, spec.args)
    };

    // For multi-binary, key the session by binary name so json/html/css can
    // co-exist as separate processes under the same catalogue id.
    let session_id = if spec.id == "vscode-langservers" {
        format!("vscode-langservers:{binary_name}")
    } else {
        spec.id.to_string()
    };

    // Prefer managed path for the chosen binary name.
    if let Some(p) = super::manifest::managed_binary_path(spec.id, binary_name) {
        return Ok((session_id, p, args.to_vec()));
    }

    // Fall back to status_one path (primary binary) then PATH lookup.
    if let Some(st) = resolve::status_one(spec.id) {
        if st.source != Source::Missing {
            if let Some(p) = st.path {
                // If status points at a different binary (json default), try PATH for ours.
                let pb = PathBuf::from(&p);
                if pb
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(binary_name) || n == binary_name)
                {
                    return Ok((session_id, pb, args.to_vec()));
                }
            }
        }
    }
    if let Some(p) = which_binary(binary_name) {
        return Ok((session_id, p, args.to_vec()));
    }
    Err(format!(
        "language server '{binary_name}' not installed (Settings → Language servers)"
    ))
}

fn which_binary(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let cmd = dir.join(format!("{name}.cmd"));
            if cmd.is_file() {
                return Some(cmd);
            }
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

// ─── Path / URI helpers ──────────────────────────────────────────────────────

fn abs_path(root: &str, path: &str) -> Result<PathBuf, String> {
    let root_p = PathBuf::from(root);
    if !root_p.is_absolute() {
        return Err("root must be absolute".into());
    }
    let rel = Path::new(path);
    if rel.is_absolute() {
        return Ok(rel.to_path_buf());
    }
    for c in rel.components() {
        if matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir) {
            return Err("path escapes workspace".into());
        }
    }
    Ok(root_p.join(rel))
}

fn path_to_uri(path: &Path) -> String {
    // Manual file:// URI — avoids depending on url crate feature flags.
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        let norm = s.replace('\\', "/");
        if norm.starts_with('/') {
            return format!("file://{norm}");
        }
        return format!("file:///{norm}");
    }
    #[cfg(not(windows))]
    {
        format!("file://{s}")
    }
}

/// Map a file path to the Monaco / LSP language id (best-effort).
pub fn language_id_for_path(path: &str) -> &'static str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let ext = file
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" | "pyi" => "python",
        "go" => "go",
        "c" => "c",
        "h" | "hpp" | "hh" | "cpp" | "cc" | "cxx" => "cpp",
        "json" | "jsonc" => "json",
        "html" | "htm" | "xhtml" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "sh" | "bash" | "zsh" => "shellscript",
        "php" | "phtml" | "php3" | "php4" | "php5" | "phps" => "php",
        "toml" => "toml",
        "lua" => "lua",
        "zig" | "zon" => "zig",
        "nix" => "nix",
        "md" | "markdown" => "markdown",
        "yaml" | "yml" => "yaml",
        _ => "plaintext",
    }
}

// ─── Result parsers ──────────────────────────────────────────────────────────

pub fn parse_completions_public(result: &serde_json::Value) -> Vec<LspCompletionItem> {
    parse_completions(result)
}
pub fn parse_one_completion_public(it: &serde_json::Value) -> Option<LspCompletionItem> {
    parse_one_completion(it)
}
pub fn parse_hover_public(result: &serde_json::Value) -> Option<LspHover> {
    parse_hover(result)
}
pub fn parse_locations_public(result: &serde_json::Value) -> Vec<LspLocation> {
    parse_locations(result)
}
pub fn parse_document_symbols_public(result: &serde_json::Value) -> Vec<LspDocumentSymbol> {
    parse_document_symbols(result)
}

fn parse_completions(result: &serde_json::Value) -> Vec<LspCompletionItem> {
    let items = if let Some(arr) = result.as_array() {
        arr.clone()
    } else if let Some(arr) = result.get("items").and_then(|i| i.as_array()) {
        arr.clone()
    } else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(parse_one_completion)
        .take(200)
        .collect()
}

fn parse_one_completion(it: &serde_json::Value) -> Option<LspCompletionItem> {
    let label = match it.get("label") {
        Some(serde_json::Value::String(s)) => s.clone(),
        // label can be CompletionItemLabelDetails-shaped in rare servers
        Some(obj) if obj.is_object() => obj
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => return None,
    };
    if label.is_empty() {
        return None;
    }
    let kind = it.get("kind").and_then(|k| k.as_u64()).map(|k| k as u32);
    let detail = it
        .get("detail")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let label_description = it
        .get("labelDetails")
        .and_then(|ld| ld.get("description"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let insert_text = it
        .get("insertText")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let insert_text_format = it
        .get("insertTextFormat")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let documentation = match it.get("documentation") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(obj) => obj
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        None => None,
    };
    let sort_text = it
        .get("sortText")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let filter_text = it
        .get("filterText")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    // textEdit can be TextEdit or InsertReplaceEdit — only plain TextEdit for now.
    let text_edit = it.get("textEdit").and_then(parse_text_edit);
    let additional_text_edits = it
        .get("additionalTextEdits")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(parse_text_edit)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());
    let data = it.get("data").cloned();
    Some(LspCompletionItem {
        label,
        kind,
        detail,
        label_description,
        insert_text,
        insert_text_format,
        documentation,
        sort_text,
        filter_text,
        text_edit,
        additional_text_edits,
        data,
    })
}

fn parse_text_edit(v: &serde_json::Value) -> Option<LspTextEdit> {
    let range = v.get("range").and_then(parse_range)?;
    let new_text = v.get("newText")?.as_str()?.to_string();
    Some(LspTextEdit { range, new_text })
}

/// Best-effort file path from a file:// URI (for revive spawn resolve only).
fn uri_to_path_lossy(uri: &str) -> String {
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("file:"))
        .unwrap_or(uri);
    // Percent-decode a few common sequences; enough for ordinary paths.
    let decoded = rest
        .replace("%20", " ")
        .replace("%5B", "[")
        .replace("%5D", "]");
    // On Windows file:///C:/... — strip leading slash before drive.
    if decoded.len() >= 3
        && decoded.as_bytes()[0] == b'/'
        && decoded.as_bytes()[1].is_ascii_alphabetic()
        && decoded.as_bytes()[2] == b':'
    {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

fn parse_hover(result: &serde_json::Value) -> Option<LspHover> {
    let contents = result.get("contents")?;
    let text = markup_to_string(contents);
    if text.trim().is_empty() {
        return None;
    }
    let range = result.get("range").and_then(parse_range);
    Some(LspHover {
        contents: text,
        range,
    })
}

fn markup_to_string(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .map(markup_to_string)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    if let Some(obj) = v.as_object() {
        if let Some(s) = obj.get("value").and_then(|x| x.as_str()) {
            return s.to_string();
        }
        if let Some(s) = obj.get("language").and_then(|x| x.as_str()) {
            let val = obj
                .get("value")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            return format!("```{s}\n{val}\n```");
        }
    }
    String::new()
}

fn parse_locations(result: &serde_json::Value) -> Vec<LspLocation> {
    if result.is_null() {
        return Vec::new();
    }
    if let Some(arr) = result.as_array() {
        return arr.iter().filter_map(parse_one_location).collect();
    }
    parse_one_location(result).into_iter().collect()
}

fn parse_one_location(v: &serde_json::Value) -> Option<LspLocation> {
    // Location
    if let (Some(uri), Some(range)) = (
        v.get("uri").and_then(|u| u.as_str()),
        v.get("range").and_then(parse_range),
    ) {
        return Some(LspLocation {
            uri: uri.to_string(),
            range,
        });
    }
    // LocationLink
    if let (Some(uri), Some(range)) = (
        v.get("targetUri").and_then(|u| u.as_str()),
        v.get("targetRange")
            .or_else(|| v.get("targetSelectionRange"))
            .and_then(parse_range),
    ) {
        return Some(LspLocation {
            uri: uri.to_string(),
            range,
        });
    }
    None
}

fn parse_range(v: &serde_json::Value) -> Option<LspRange> {
    let start = v.get("start")?;
    let end = v.get("end")?;
    Some(LspRange {
        start_line: start.get("line")?.as_u64()? as u32,
        start_character: start.get("character")?.as_u64()? as u32,
        end_line: end.get("line")?.as_u64()? as u32,
        end_character: end.get("character")?.as_u64()? as u32,
    })
}

fn parse_document_symbols(result: &serde_json::Value) -> Vec<LspDocumentSymbol> {
    if result.is_null() {
        return Vec::new();
    }
    let Some(arr) = result.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        collect_document_symbol(item, &mut out);
    }
    out
}

fn collect_document_symbol(v: &serde_json::Value, out: &mut Vec<LspDocumentSymbol>) {
    // DocumentSymbol (hierarchical) has `range` + `selectionRange` + optional children.
    if let (Some(name), Some(kind), Some(range), Some(sel)) = (
        v.get("name").and_then(|x| x.as_str()),
        v.get("kind").and_then(|x| x.as_u64()),
        v.get("range").and_then(parse_range),
        v.get("selectionRange")
            .or_else(|| v.get("range"))
            .and_then(parse_range),
    ) {
        out.push(LspDocumentSymbol {
            name: name.to_string(),
            kind: kind as u32,
            range,
            selection_range: sel,
        });
        if let Some(children) = v.get("children").and_then(|c| c.as_array()) {
            for c in children {
                collect_document_symbol(c, out);
            }
        }
        return;
    }
    // SymbolInformation — flat, location.range.
    if let (Some(name), Some(kind), Some(loc)) = (
        v.get("name").and_then(|x| x.as_str()),
        v.get("kind").and_then(|x| x.as_u64()),
        v.get("location"),
    ) {
        if let Some(range) = loc.get("range").and_then(parse_range) {
            out.push(LspDocumentSymbol {
                name: name.to_string(),
                kind: kind as u32,
                range: range.clone(),
                selection_range: range,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_id_rust() {
        assert_eq!(language_id_for_path("src/main.rs"), "rust");
    }

    #[test]
    fn language_id_php() {
        assert_eq!(language_id_for_path("app/Http/Controllers/UserController.php"), "php");
        assert_eq!(language_id_for_path("resources/views/welcome.phtml"), "php");
    }

    #[test]
    fn parse_completion_list() {
        let v = serde_json::json!({
            "items": [
                { "label": "foo", "kind": 3, "detail": "fn" },
                { "label": "bar" }
            ]
        });
        let items = parse_completions(&v);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "foo");
        assert_eq!(items[0].kind, Some(3));
    }

    #[test]
    fn parse_hover_markdown() {
        let v = serde_json::json!({
            "contents": { "kind": "markdown", "value": "**hi**" }
        });
        let h = parse_hover(&v).expect("hover");
        assert!(h.contents.contains("hi"));
    }

    #[test]
    fn path_uri_unix() {
        let u = path_to_uri(Path::new("/tmp/foo.rs"));
        assert!(u.starts_with("file://"));
        assert!(u.contains("/tmp/foo.rs"));
    }
}

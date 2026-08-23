//! Host-spawned LSP JSON-RPC client (stdio, Content-Length framing).
//!
//! One process per catalogue server id, lazily started on first document open.
//! Completions / hover / definition are request/response; diagnostics arrive as
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

/// One completion item for Monaco.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspCompletionItem {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
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
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

/// Go-to-definition location.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

/// Internal reply from the reader thread to a waiting request.
enum PendingReply {
    Ok(serde_json::Value),
    Err(String),
}

struct OpenDoc {
    server_id: String,
    language_id: String,
    #[allow(dead_code)]
    version: i32,
}

struct ServerSession {
    /// Catalogue id (e.g. `rust-analyzer`).
    #[allow(dead_code)]
    id: String,
    /// Workspace root this process was initialized for (absolute).
    #[allow(dead_code)]
    root: PathBuf,
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, Sender<PendingReply>>>>,
    /// Language ids this multi-binary session was spawned for (vscode-langservers).
    #[allow(dead_code)]
    language_ids: Vec<String>,
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
        if self.docs.contains_key(&uri) {
            // Re-open: bump as full change.
            return self.did_change(root, path, text);
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

        if !self.servers.contains_key(&spawn_id) {
            let session = spawn_server(
                &spawn_id,
                &binary,
                &args,
                &root_path,
                Arc::clone(&self.push),
            )?;
            self.servers.insert(spawn_id.clone(), session);
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
                language_id: language_id.to_string(),
                version,
            },
        );
        Ok(())
    }

    /// Full-document `textDocument/didChange`.
    pub fn did_change(&mut self, root: &str, path: &str, text: &str) -> Result<(), String> {
        let abs = abs_path(root, path)?;
        let uri = path_to_uri(&abs);
        let doc = self
            .docs
            .get_mut(&uri)
            .ok_or_else(|| "document not open".to_string())?;
        doc.version = doc.version.saturating_add(1);
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
        let server_id = self
            .docs
            .get(&uri)
            .map(|d| d.server_id.clone())
            .ok_or_else(|| "document not open".to_string())?;
        let session = self
            .servers
            .get_mut(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        let doc = serde_json::json!({ "uri": uri });
        let mut params = serde_json::json!({ "textDocument": doc });
        if let Some(t) = text {
            params
                .as_object_mut()
                .unwrap()
                .insert("text".into(), serde_json::Value::String(t.to_string()));
        }
        session.notify("textDocument/didSave", params)
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
            let params = serde_json::json!({
                "textDocument": { "uri": uri }
            });
            let _ = session.notify("textDocument/didClose", params);
        }
        // Clear markers for this URI.
        push_diagnostics(&*self.push, uri, Vec::new());
    }

    /// `textDocument/completion`.
    pub fn completion(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspCompletionItem>, String> {
        let (uri, server_id) = self.uri_server(root, path)?;
        let session = self
            .servers
            .get_mut(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "triggerKind": 1 }
        });
        let result = session.request("textDocument/completion", params)?;
        Ok(parse_completions(&result))
    }

    /// `textDocument/hover`.
    pub fn hover(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<LspHover>, String> {
        let (uri, server_id) = self.uri_server(root, path)?;
        let session = self
            .servers
            .get_mut(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
        });
        let result = session.request("textDocument/hover", params)?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(parse_hover(&result))
    }

    /// `textDocument/definition`.
    pub fn definition(
        &mut self,
        root: &str,
        path: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, String> {
        let (uri, server_id) = self.uri_server(root, path)?;
        let session = self
            .servers
            .get_mut(&server_id)
            .ok_or_else(|| "server not running".to_string())?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
        });
        let result = session.request("textDocument/definition", params)?;
        Ok(parse_locations(&result))
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
}

impl ServerSession {
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
    let pending_reader = Arc::clone(&pending);
    let push_reader = Arc::clone(&push);
    let stdin_init = Arc::clone(&stdin);

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

    // Reader thread: Content-Length frames → pending map or diagnostics push.
    std::thread::Builder::new()
        .name(format!("lsp-out-{id}"))
        .spawn(move || {
            reader_loop(stdout, pending_reader, push_reader);
        })
        .map_err(|e| format!("lsp reader spawn: {e}"))?;

    let session = ServerSession {
        id: id.to_string(),
        root: root.to_path_buf(),
        child,
        stdin: stdin_init,
        next_id: AtomicU64::new(1),
        pending,
        language_ids: Vec::new(),
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
                        "snippetSupport": false,
                        "documentationFormat": ["plaintext", "markdown"]
                    },
                    "contextSupport": true
                },
                "hover": {
                    "contentFormat": ["plaintext", "markdown"]
                },
                "definition": {
                    "linkSupport": false
                },
                "publishDiagnostics": {
                    "relatedInformation": false,
                    "versionSupport": false
                }
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

    // rust-analyzer: wait is not required; first didOpen triggers analysis.
    let _ = id;
    Ok(session)
}

fn reader_loop<R: Read>(
    stdout: R,
    pending: Arc<Mutex<HashMap<u64, Sender<PendingReply>>>>,
    push: Arc<dyn Fn(String) + Send + Sync>,
) {
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

        // Response to a request we sent.
        if let Some(id) = msg.get("id").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().map(|i| i as u64))
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        }) {
            // Server→client request (has method + id) — reply with empty result.
            if msg.get("method").is_some() {
                // Best-effort: we don't currently support server requests (applyEdit…).
                continue;
            }
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
            if method == "textDocument/publishDiagnostics" {
                if let Some(params) = msg.get("params") {
                    handle_publish_diagnostics(params, &*push);
                }
            }
            // window/logMessage, $/progress, etc. — ignore for v1.
        }
    }

    // Fail every waiter so nobody hangs after crash/EOF.
    let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
    for (_, tx) in map.drain() {
        let _ = tx.send(PendingReply::Err("LSP server closed".into()));
    }
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
        .filter_map(|it| {
            let label = it.get("label")?.as_str()?.to_string();
            let kind = it.get("kind").and_then(|k| k.as_u64()).map(|k| k as u32);
            let detail = it
                .get("detail")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string());
            let insert_text = it
                .get("insertText")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string());
            let documentation = match it.get("documentation") {
                Some(serde_json::Value::String(s)) => Some(s.clone()),
                Some(obj) => obj
                    .get("value")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                None => None,
            };
            Some(LspCompletionItem {
                label,
                kind,
                detail,
                insert_text,
                documentation,
            })
        })
        .take(200)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_id_rust() {
        assert_eq!(language_id_for_path("src/main.rs"), "rust");
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

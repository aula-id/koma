//! Security daemon CLIENT.
//!
//! koma can drive a long-lived Python security daemon (`koma_sec_daemon`, vendored
//! under `src-security/` and provisioned by [`crate::security`]). This module is the
//! Rust-side manager: it spawns the daemon as a child process, talks to it over
//! newline-delimited JSON on the child's stdin/stdout, discovers its tools, and
//! offers a synchronous dispatch entry point for the tool layer.
//!
//! It deliberately MIRRORS the MCP client manager ([`crate::app::mcp`]): a runtime
//! [`Handle`] plus a mutex-guarded [`Inner`], a generation counter to discard stale
//! results from a torn-down child, and the same sync→async dispatch bridge (clone a
//! handle under a brief lock, drop the lock, `spawn` the async work, block the
//! calling thread on a `std::sync::mpsc::recv_timeout` — NEVER `block_on`, which
//! panics inside a runtime).
//!
//! ## Scope
//!
//! Building block only — nothing wires this into [`crate::tool::ToolCtx`] or the
//! app lifecycle yet. When the security daemon is not installed
//! ([`crate::security::is_installed`] is false) the manager stays fully inert:
//! [`SecDaemonManager::start`] is a no-op, no child is spawned, and every accessor
//! returns empty, so behaviour is byte-identical to a build without the daemon.
//!
//! ## Tool naming
//!
//! The daemon already namespaces its tools with a `sec_` prefix (e.g. `sec_http`,
//! `sec_remote`), so — unlike MCP — no client-side renaming happens here; the names
//! ride through verbatim.
//!
//! ## Protocol (newline-delimited JSON)
//!
//! - **Spawn:** `python -m koma_sec_daemon --token <TOKEN>` with cwd =
//!   [`crate::security::security_dir`], python = [`crate::security::venv_python`],
//!   stdin/stdout/stderr all piped.
//! - **Handshake:** write `{"v":1,"token":"<TOKEN>"}\n`; read the first stdout line
//!   `{"ok":true,"tools":[{name,description,parameters,risk,compute,domain},…]}`.
//! - **Call:** write `{"id":N,"op":"call","tool":"<name>","args":{…}}\n`; the daemon
//!   replies (possibly out of order) `{"id":N,"ok":true,"result":"…"}` or
//!   `{"id":N,"ok":false,"error":"…"}`.
//!
//! ## Concurrency model
//!
//! The child process and its I/O live on the app's tokio runtime. A single **writer
//! task** owns the child's stdin and receives outgoing frames over an
//! `mpsc::UnboundedSender<String>` (so the synchronous dispatch path can enqueue a
//! frame without touching the stdin handle directly). A single **reader task** owns
//! the child's stdout, parses each frame, and fulfils the matching `oneshot` from a
//! shared `pending` map keyed by request id. A third **stderr-drain task** reads the
//! child's stderr to EOF and discards it, so a chatty child never blocks on a full
//! stderr pipe. The mutex is never held across an `.await`.
//!
//! NOTE: this is a building block — nothing references it yet (the tool-system and
//! `/security` cockpit wiring land in follow-up tasks), so the whole module is
//! `#![allow(dead_code)]` to keep `cargo build`/`clippy` clean until then. The same
//! pattern is used by the other not-yet-fully-wired app modules (e.g. `subagent`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::dto::openrouter::request::{ToolDef, ToolFunctionDef};

/// In-flight calls awaiting a reply, keyed by request id. Shared between
/// [`SecDaemonManager`]'s dispatch path (which inserts an entry) and the reader task
/// (which removes + fulfils it). Aliased to keep the otherwise-noisy nested generic
/// readable at its several use sites.
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>>;

/// How long the spawn + handshake (write token, read the first `{"ok":true,…}`
/// line) may take before the start task gives up and the manager stays inert.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a single tool call round-trip may take before
/// [`SecDaemonManager::execute_blocking`] gives up and returns an error string.
/// Security tooling is slow (network scans, remote exploit attempts), hence the
/// generous budget.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// A tool advertised by the daemon during the handshake.
///
/// `parameters` is the raw JSON-Schema object the daemon supplied (forwarded
/// verbatim to the model). `risk`/`compute`/`domain` are the daemon's own metadata
/// used by the `/security` cockpit and the risk-gating logic.
#[derive(Clone)]
struct SecToolDesc {
    /// Tool name, already `sec_`-prefixed by the daemon.
    name: String,
    /// Human-readable description (empty when the daemon omits one).
    description: String,
    /// Raw JSON-Schema `parameters` object, verbatim from the daemon.
    parameters: serde_json::Value,
    /// Whether the tool is considered risky (gated behind confirmation upstream).
    risk: bool,
    /// Free-form compute-class hint (e.g. `"light"`, `"heavy"`).
    compute: String,
    /// Free-form domain tag (e.g. `"web"`, `"network"`).
    domain: String,
}

/// Mutable manager state, guarded by a single mutex.
///
/// Written by the start/stop/restart paths and the dispatch path; read by the
/// synchronous accessors. The mutex is only ever held for cheap, synchronous work —
/// never across an `.await`.
#[derive(Default)]
struct Inner {
    /// `true` once the handshake succeeded and the reader/writer tasks are live.
    running: bool,
    /// Monotonic generation, bumped by every [`SecDaemonManager::stop`] /
    /// [`SecDaemonManager::start`]. A start task captures the generation BEFORE its
    /// spawn+handshake await and re-checks it AFTER (under the lock, before storing):
    /// a mismatch means a `stop`/`restart` superseded this attempt, so its result is
    /// discarded and its child is killed.
    generation: u64,
    /// Sender into the writer task that owns the child's stdin. `Some` only while a
    /// child is live; dropping it (on stop) closes the writer task, which closes
    /// stdin and lets the child exit.
    writer: Option<mpsc::UnboundedSender<String>>,
    /// In-flight calls awaiting a reply, keyed by request id. The reader task fulfils
    /// (and removes) an entry when a frame with that id arrives; `execute_blocking`
    /// removes its own entry on timeout. Shared with the reader task via the `Arc`.
    pending: PendingMap,
    /// Monotonic source of request ids handed out by `execute_blocking`.
    next_id: u64,
    /// Handle to the live child, kept so [`SecDaemonManager::stop`] can `start_kill`
    /// it. `None` when no child is running.
    child: Option<tokio::process::Child>,
    /// Tools advertised by the daemon at the last successful handshake. Empty when
    /// not running.
    tools: Vec<SecToolDesc>,
}

/// The security daemon client manager.
///
/// Holds the runtime [`Handle`] (so async work can be spawned from synchronous code)
/// and a mutex-guarded [`Inner`]. Constructed inert via [`Self::new`]; a child is
/// only spawned by [`Self::start`], and only when the daemon is installed.
pub struct SecDaemonManager {
    handle: Handle,
    inner: Mutex<Inner>,
}

impl SecDaemonManager {
    /// Build an inert manager: not running, no child, no tools. Spawns nothing.
    ///
    /// Call [`Self::start`] to actually launch the daemon (a no-op unless it is
    /// installed).
    pub fn new(handle: &Handle) -> Arc<Self> {
        Arc::new(Self {
            handle: handle.clone(),
            inner: Mutex::new(Inner::default()),
        })
    }

    /// Launch the security daemon in the background. NON-BLOCKING — returns
    /// immediately; the spawn + handshake happen on a task spawned onto the runtime.
    ///
    /// If the daemon is not installed ([`crate::security::is_installed`]) this is a
    /// no-op and the manager stays inert. Otherwise the generation is bumped (so any
    /// previous child's tasks are superseded) and the start task:
    /// 1. spawns `python -m koma_sec_daemon --token <token>` (stdio piped),
    /// 2. writes the handshake frame and reads the first reply line (bounded by
    ///    [`CONNECT_TIMEOUT`]),
    /// 3. on success: stores the tools + child, installs the writer/reader/stderr
    ///    tasks, and flips `running` — but only if the generation still matches.
    ///
    /// ## Generation guard
    ///
    /// The captured generation is re-checked under the lock right before the result
    /// is stored. If a [`Self::stop`]/[`Self::restart`] bumped it mid-handshake, the
    /// freshly spawned child is killed and nothing is stored, so a slow start can
    /// never resurrect a daemon the user just stopped.
    pub fn start(self: &Arc<Self>, token: String) {
        // Not installed → stay inert. No child, no tasks, no cost.
        if !crate::security::is_installed() {
            return;
        }

        // Bump the generation under the lock and capture it. Any in-flight start task
        // from a previous call now holds a stale generation and will discard itself.
        let gen_at_start = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };

        let mgr = Arc::clone(self);
        self.handle.spawn(async move {
            // Spawn + handshake, bounded by CONNECT_TIMEOUT. On any failure the child
            // (if it was spawned) is dropped here, which reaps it.
            let connected = match tokio::time::timeout(CONNECT_TIMEOUT, spawn_and_handshake(&token))
                .await
            {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => {
                    eprintln!("sec: security daemon failed to start: {e}");
                    return;
                }
                Err(_) => {
                    eprintln!(
                        "sec: security daemon handshake timed out after {}s",
                        CONNECT_TIMEOUT.as_secs()
                    );
                    return;
                }
            };

            let Connected {
                mut child,
                stdin,
                stdout,
                stderr,
                tools,
            } = connected;

            // Build the channels/maps the live tasks will share, but DON'T spawn the
            // tasks until we know the generation still matches (otherwise a superseded
            // start would leak tasks driving a child we're about to kill).
            let (tx, rx) = mpsc::unbounded_channel::<String>();
            let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

            // Store the result — but only if this attempt is still current.
            {
                let mut inner = mgr.inner.lock().unwrap_or_else(|p| p.into_inner());
                if inner.generation != gen_at_start {
                    // Superseded mid-handshake (stop/restart bumped the generation).
                    // Kill the just-spawned child and store nothing; the tasks were
                    // never spawned, so there is nothing else to tear down.
                    let _ = child.start_kill();
                    return;
                }
                inner.running = true;
                inner.tools = tools;
                inner.writer = Some(tx);
                inner.pending = Arc::clone(&pending);
                inner.child = Some(child);
            }

            // Now that the result is committed under the matching generation, spawn
            // the three long-lived tasks. Each is guarded by the captured generation
            // (via `mgr`) so it becomes a no-op once a later stop/restart supersedes
            // this child.
            mgr.handle.spawn(writer_task(stdin, rx));
            mgr.handle.spawn(reader_task(
                stdout,
                Arc::clone(&pending),
                Arc::clone(&mgr),
                gen_at_start,
            ));
            mgr.handle.spawn(stderr_drain_task(stderr));
        });
    }

    /// Stop the daemon: bump the generation (superseding any in-flight start and the
    /// live tasks), kill the child, clear the tools, set `running = false`, and fail
    /// every pending caller with an error so no `execute_blocking` hangs.
    ///
    /// Idempotent and non-blocking: the actual child kill (`start_kill`) only signals
    /// termination; the child is reaped when its handle drops. Dropping the `writer`
    /// sender also closes the writer task, which closes the child's stdin.
    pub fn stop(self: &Arc<Self>) {
        // Take everything out under one lock, then release before doing any async kill.
        let (mut child, pending) = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.generation = inner.generation.wrapping_add(1);
            inner.running = false;
            inner.tools.clear();
            // Drop the writer sender → the writer task ends → child stdin closes.
            inner.writer = None;
            // Swap out the pending map so we fail its waiters below; the reader task
            // keeps its own Arc clone but will find an empty map (and exits anyway
            // once stdout closes after the kill).
            let pending = std::mem::take(&mut inner.pending);
            let child = inner.child.take();
            (child, pending)
        };

        // Fail every in-flight caller so their recv_timeout returns promptly instead
        // of waiting out CALL_TIMEOUT against a dead child.
        {
            let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
            for (_id, tx) in map.drain() {
                let _ = tx.send(Err("security daemon stopped".to_string()));
            }
        }

        // Signal the child to terminate. Best-effort: if there is no child this is a
        // no-op. The handle is dropped here, which reaps the process.
        if let Some(child) = child.as_mut() {
            let _ = child.start_kill();
        }
    }

    /// Restart the daemon: [`Self::stop`] then [`Self::start`] with the new token.
    pub fn restart(self: &Arc<Self>, token: String) {
        self.stop();
        self.start(token);
    }

    /// THE SYNC→ASYNC BRIDGE. Send a tool call to the daemon and block until the
    /// reply (or [`CALL_TIMEOUT`]) lands.
    ///
    /// Mirrors [`crate::app::mcp::McpManager::execute_blocking`]: under a brief lock it
    /// grabs the writer + a fresh id + a clone of the `pending` map, then drops the
    /// lock. It registers a `oneshot` under the id, spawns an async task that writes
    /// the call frame to the child's stdin (via the writer channel) — the reader task
    /// will fulfil the `oneshot` when the matching reply arrives — and bridges that
    /// `oneshot` to a `std::sync::mpsc` it blocks on with `recv_timeout`. We do NOT
    /// use `Handle::block_on` because the synchronous tool path may already be inside
    /// the tokio runtime, where `block_on` panics.
    ///
    /// Returns `Err("security daemon not running")` when no child is live, and on
    /// timeout removes the pending entry and returns `Err("sec tool '<name>' timed
    /// out")`.
    pub fn execute_blocking(&self, tool: &str, args: &serde_json::Value) -> Result<String, String> {
        self.request(serde_json::json!({ "op": "call", "tool": tool, "args": args }))
    }

    /// THE SYNC→ASYNC BRIDGE (op-generic). Send one request `frame` to the daemon and
    /// block until the reply (or [`CALL_TIMEOUT`]) lands, returning the daemon's
    /// `result` string on `ok` or its `error` string otherwise.
    ///
    /// This is the shared core extracted from [`Self::execute_blocking`]; it knows
    /// nothing about the `op` it carries, so `call`/`health`/`install` (and any future
    /// op) all dispatch through the SAME path — the reader task already routes purely
    /// by request id. The caller supplies the frame WITHOUT an `id`; `request` injects
    /// the fresh id under the lock, registers the matching `oneshot`, and the reader
    /// fulfils it when the id-tagged reply arrives.
    ///
    /// Under a brief lock it grabs the writer + a fresh id + a clone of the `pending`
    /// map, drops the lock, then spawns an async task that enqueues the frame to the
    /// writer and bridges the `oneshot` to a `std::sync::mpsc` this thread blocks on
    /// with `recv_timeout`. `Handle::block_on` is NOT used because the synchronous tool
    /// path may already be inside the tokio runtime, where `block_on` panics.
    ///
    /// Returns `Err("security daemon not running")` when no child is live, and on
    /// timeout removes the pending entry and returns `Err("sec request '<op>' timed
    /// out")`.
    fn request(&self, mut frame: serde_json::Value) -> Result<String, String> {
        // Grab the writer, a fresh id, and the pending map under a brief lock, then
        // drop the guard before spawning.
        let (writer, id, pending) = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if !inner.running {
                return Err("security daemon not running".to_string());
            }
            let writer = match &inner.writer {
                Some(w) => w.clone(),
                None => return Err("security daemon not running".to_string()),
            };
            let id = inner.next_id;
            inner.next_id = inner.next_id.wrapping_add(1);
            (writer, id, Arc::clone(&inner.pending))
        };

        // Inject the fresh id into the caller-supplied frame. The frame is always a
        // JSON object built by us (`json!({ "op": .. })`), so `as_object_mut` is
        // present; fall back to a no-op only to stay panic-free.
        if let Some(obj) = frame.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!(id));
        }
        // For the timeout/disconnect messages: identify the request by its op.
        let op = frame
            .get("op")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("request")
            .to_string();
        let frame = frame.to_string();

        // Register the oneshot under this id so the reader task can fulfil it.
        let (otx, orx) = oneshot::channel::<Result<String, String>>();
        {
            let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
            map.insert(id, otx);
        }

        // Bridge the async oneshot to a sync mpsc this thread blocks on. The spawned
        // task: enqueue the frame to the writer, then await the oneshot and forward
        // its result. If the writer channel is already closed (child gone), report
        // that instead of waiting out the timeout.
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        let pending_for_task = Arc::clone(&pending);
        self.handle.spawn(async move {
            if writer.send(format!("{frame}\n")).is_err() {
                // Writer task is gone → child stdin is closed. Clean up our pending
                // entry and report the failure.
                pending_for_task
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                let _ = tx.send(Err("security daemon not running".to_string()));
                return;
            }
            let result = match orx.await {
                Ok(r) => r,
                // Sender dropped without sending (reader task gone / daemon stopped).
                Err(_) => Err("security daemon stopped before reply".to_string()),
            };
            let _ = tx.send(result);
        });

        match rx.recv_timeout(CALL_TIMEOUT) {
            Ok(r) => r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Drop the pending entry so a late reply is ignored rather than
                // fulfilling a stale oneshot.
                pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                Err(format!("sec request '{op}' timed out"))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                Err(format!("sec request '{op}' task dropped before result"))
            }
        }
    }

    /// Round-trip the daemon's health probe. Heavy (full IPC call) — callers fetch on
    /// panel open/refresh only.
    pub fn health(&self) -> Result<Vec<InstallHealthEntry>, String> {
        let raw = self.request(serde_json::json!({ "op": "health" }))?;
        serde_json::from_str::<Vec<InstallHealthEntry>>(&raw)
            .map_err(|e| format!("failed to parse health response: {e}"))
    }

    /// Kick off [`Self::health`] on a blocking-pool thread and deliver the result over an
    /// unbounded channel. NON-BLOCKING — returns immediately; the caller drains the
    /// receiver in the event loop (mirrors the `version_rx` / `warm_rx` async-result
    /// pattern) so the `/security` panel never freezes on the cold first probe.
    ///
    /// `spawn_blocking` (NOT `spawn`) is mandatory: `health()` calls `request()`, which
    /// blocks the calling thread on a synchronous `recv_timeout` — running that on an
    /// async worker thread would stall the runtime. `self.handle` is the manager's private
    /// tokio [`Handle`], accessible here as this is an inherent method on the same type.
    pub fn health_async(
        self: &Arc<Self>,
    ) -> mpsc::UnboundedReceiver<Result<Vec<InstallHealthEntry>, String>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let me = Arc::clone(self);
        self.handle.spawn_blocking(move || {
            // A dropped receiver (panel closed before the probe finished) makes this a no-op.
            let _ = tx.send(me.health());
        });
        rx
    }

    /// Install/repair a single dependency by manifest key; returns the daemon's status
    /// string.
    pub fn install(&self, key: &str) -> Result<String, String> {
        self.request(serde_json::json!({ "op": "install", "key": key }))
    }

    /// Wire [`ToolDef`]s for every advertised tool, ready to extend the request
    /// `tools` array. Empty when the daemon is not running.
    pub fn tool_defs(&self) -> Vec<ToolDef> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .tools
            .iter()
            .map(|t| ToolDef {
                kind: "function".into(),
                function: ToolFunctionDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    }

    /// The advertised tool names (already `sec_`-prefixed by the daemon), for the
    /// advertise allow-list. Empty when not running.
    pub fn tool_names(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.tools.iter().map(|t| t.name.clone()).collect()
    }

    /// Look up a tool's `risk` flag. `false` for an unknown tool (fail-open on the
    /// lookup, not on the gate — callers treat a missing tool as non-risky here).
    pub fn tool_risk(&self, name: &str) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .tools
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.risk)
            .unwrap_or(false)
    }

    /// `true` once the handshake succeeded and the daemon is live.
    pub fn is_running(&self) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.running
    }

    /// A serializable snapshot for the `/security` cockpit: running/installed flags
    /// plus the per-tool metadata. Cheap — locks the inner state and copies the small
    /// tool list.
    pub fn status(&self) -> SecStatus {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        SecStatus {
            running: inner.running,
            installed: crate::security::is_installed(),
            tools: inner
                .tools
                .iter()
                .map(|t| SecToolInfo {
                    name: t.name.clone(),
                    domain: t.domain.clone(),
                    risk: t.risk,
                    compute: t.compute.clone(),
                })
                .collect(),
        }
    }
}

/// Serializable summary of the daemon for the `/security` panel.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecStatus {
    /// `true` when the daemon child is live and the handshake succeeded.
    pub running: bool,
    /// `true` when the daemon is provisioned on disk ([`crate::security::is_installed`]).
    pub installed: bool,
    /// One entry per advertised tool.
    pub tools: Vec<SecToolInfo>,
}

/// Per-tool metadata surfaced in [`SecStatus`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecToolInfo {
    /// Tool name (already `sec_`-prefixed).
    pub name: String,
    /// Free-form domain tag (e.g. `"web"`, `"network"`).
    pub domain: String,
    /// Whether the tool is considered risky.
    pub risk: bool,
    /// Free-form compute-class hint.
    pub compute: String,
}

/// One dependency's install-health entry, as reported by the daemon's `health` op.
///
/// Mirrors the daemon's per-entry JSON exactly: `key`, `name`, `tier`, `present`,
/// `method`, `tools`, `hint`. Surfaced in the `/security` cockpit's install panel so
/// the user can see what is installed and repair missing dependencies via
/// [`SecDaemonManager::install`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InstallHealthEntry {
    /// Manifest key, used as the argument to [`SecDaemonManager::install`].
    pub key: String,
    /// Human-readable dependency name.
    pub name: String,
    /// Tier/priority of the dependency.
    pub tier: u8,
    /// Whether the dependency is currently present.
    pub present: bool,
    /// Install/detection method (e.g. `"pip"`, `"apt"`, `"binary"`).
    pub method: String,
    /// Tool names this dependency backs.
    pub tools: Vec<String>,
    /// Free-form hint for installing/repairing the dependency.
    pub hint: String,
}

/// The product of a successful spawn + handshake: the live child, its split I/O
/// halves, and the tools it advertised. `stdout` is a `BufReader` wrapping the raw
/// child stdout so any bytes already buffered during the handshake read are not lost
/// when the reader task takes over.
struct Connected {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    stderr: tokio::process::ChildStderr,
    tools: Vec<SecToolDesc>,
}

/// Spawn `python -m koma_sec_daemon --token <token>` (stdio piped), perform the
/// newline-delimited-JSON handshake, and return the live child + I/O + advertised
/// tools.
///
/// The handshake writes `{"v":1,"token":"<token>"}\n` to the child's stdin, then
/// reads the FIRST line of its stdout, which must parse as `{"ok":true,"tools":[…]}`.
/// Any spawn/IO/parse failure — or an `{"ok":false}` reply — is returned as `Err`.
async fn spawn_and_handshake(token: &str) -> Result<Connected, String> {
    let python = crate::security::venv_python()
        .map_err(|e| format!("cannot locate security venv python: {e}"))?;
    let dir = crate::security::security_dir()
        .map_err(|e| format!("cannot locate security dir: {e}"))?;

    let mut child = tokio::process::Command::new(&python)
        .arg("-m")
        .arg("koma_sec_daemon")
        .arg("--token")
        .arg(token)
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn koma_sec_daemon failed: {e}"))?;

    // Take ownership of the piped I/O halves. All three were requested above, so
    // these are present; a missing handle is a hard error (child gets dropped/reaped).
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "child stdin not piped".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout not piped".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child stderr not piped".to_string())?;

    // Write the handshake frame.
    let hello = serde_json::json!({ "v": 1, "token": token }).to_string();
    stdin
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("handshake flush failed: {e}"))?;

    // Read the first stdout line = the handshake reply.
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("handshake read failed: {e}"))?;
    if n == 0 {
        return Err("daemon closed stdout before handshake reply".to_string());
    }

    let reply: serde_json::Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("handshake reply was not valid JSON: {e}"))?;

    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let err = reply
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("handshake rejected");
        return Err(format!("daemon rejected handshake: {err}"));
    }

    let tools = parse_tools(reply.get("tools"));

    // Pass the SAME BufReader into Connected so the reader task continues from where
    // the handshake left off. Using into_inner() here would DISCARD any bytes the
    // BufReader already buffered past the first line — currently safe only because the
    // daemon never sends a second line unsolicited, but wrong in principle.
    Ok(Connected {
        child,
        stdin,
        stdout: reader,
        stderr,
        tools,
    })
}

/// Parse the `tools` array from a handshake reply into [`SecToolDesc`]s. A missing or
/// non-array value yields an empty list; malformed entries degrade field-by-field
/// (missing strings → empty, missing `risk` → false) rather than failing the whole
/// handshake.
fn parse_tools(value: Option<&serde_json::Value>) -> Vec<SecToolDesc> {
    let arr = match value.and_then(serde_json::Value::as_array) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|t| {
            // A tool with no name is unusable; skip it.
            let name = t.get("name").and_then(serde_json::Value::as_str)?.to_string();
            Some(SecToolDesc {
                name,
                description: t
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                parameters: t
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
                risk: t
                    .get("risk")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                compute: t
                    .get("compute")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                domain: t
                    .get("domain")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// Owns the child's stdin and forwards every frame received on `rx` to it. Exits when
/// the channel closes (all senders dropped — i.e. [`SecDaemonManager::stop`] cleared
/// the `writer`) or on the first write error (child gone), closing stdin on the way
/// out so the child observes EOF.
async fn writer_task(mut stdin: tokio::process::ChildStdin, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(frame) = rx.recv().await {
        if stdin.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        if stdin.flush().await.is_err() {
            break;
        }
    }
    // Explicitly close stdin so the child sees EOF promptly (drop would do this too).
    let _ = stdin.shutdown().await;
}

/// Mark the daemon as stopped, but ONLY if the generation still matches the one this
/// reader was started under. This prevents a fresh `start()` — which would have bumped
/// the generation — from being clobbered by an EOF event from the previous child's
/// reader task racing the new start.
///
/// On match: sets `running = false`, clears `tools`, and drops `writer` (closing the
/// writer task and therefore stdin). The lock is held synchronously and dropped
/// before returning; no `.await` occurs here.
fn mark_stopped(mgr: &SecDaemonManager, gen: u64) {
    let mut guard = mgr.inner.lock().unwrap_or_else(|p| p.into_inner());
    if guard.generation != gen {
        // A stop/restart already superseded this child; leave the new state alone.
        return;
    }
    guard.running = false;
    guard.tools.clear();
    // Drop the writer sender → writer task ends → stdin closes → child sees EOF.
    guard.writer = None;
}

/// Owns the child's stdout and dispatches replies. Each line is parsed as a JSON
/// frame; a frame carrying an `id` fulfils (and removes) the matching `oneshot` from
/// `pending` with `Ok(result)` / `Err(error)`. Frames without an `id` (or for an
/// unknown id) are logged and ignored. Exits when stdout reaches EOF (child gone),
/// at which point every still-pending caller is failed so none hang.
///
/// `mgr` and `gen` are used to mark the daemon stopped on EOF (generation-guarded
/// so a superseding `start()` is never clobbered by this stale reader).
async fn reader_task(
    stdout: BufReader<tokio::process::ChildStdout>,
    pending: PendingMap,
    mgr: Arc<SecDaemonManager>,
    gen: u64,
) {
    // `stdout` is the same BufReader used for the handshake read, so any bytes
    // buffered past the handshake line are not lost.
    let mut lines = stdout.lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let frame: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("sec: ignoring unparseable daemon frame: {e}");
                        continue;
                    }
                };
                // Only id-bearing frames map to a pending call.
                let id = match frame.get("id").and_then(serde_json::Value::as_u64) {
                    Some(id) => id,
                    None => {
                        // Non-reply frame (e.g. a log/event); nothing to fulfil.
                        continue;
                    }
                };
                let tx = {
                    let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
                    map.remove(&id)
                };
                let Some(tx) = tx else {
                    // No waiter for this id (timed out + removed, or duplicate reply).
                    continue;
                };
                let result = if frame.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                    Ok(frame
                        .get("result")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string())
                } else {
                    Err(frame
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("tool reported an error")
                        .to_string())
                };
                // The receiver may have gone away (caller timed out between our remove
                // and now); ignore the send error in that case.
                let _ = tx.send(result);
            }
            // EOF or read error: the child closed stdout. Stop reading and fail any
            // stragglers, then mark the manager stopped so stale tools stop advertising.
            Ok(None) => break,
            Err(e) => {
                eprintln!("sec: error reading daemon stdout: {e}");
                break;
            }
        }
    }

    // Child is gone: fail every remaining waiter so no execute_blocking hangs out the
    // full CALL_TIMEOUT.
    {
        let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
        for (_id, tx) in map.drain() {
            let _ = tx.send(Err("security daemon closed connection".to_string()));
        }
    }

    // Flip the manager to stopped (generation-guarded) so is_running() returns false
    // and tool_names()/tool_defs() stop advertising dead tools after a daemon crash.
    mark_stopped(&mgr, gen);
}

/// Reads the child's stderr to EOF and discards it, so a chatty child never blocks on
/// a full stderr pipe. Errors are ignored — this is a best-effort drain.
async fn stderr_drain_task(mut stderr: tokio::process::ChildStderr) {
    let mut buf = Vec::new();
    let _ = stderr.read_to_end(&mut buf).await;
}

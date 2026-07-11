//! MCP (Model Context Protocol) CLIENT.
//!
//! koma can act as an MCP *client*: for each enabled `mcp_servers` entry in the
//! global config it spawns/connects an [`rmcp`] client, discovers that server's
//! tools, advertises them to the model as ordinary function-calling tools, and
//! routes the model's calls back to the owning server.
//!
//! ## Scope
//!
//! This is a focused client only — there is NO management UI here (config.json is
//! hand-edited). If `mcp_servers` is empty the manager is inert: it spawns no
//! connections, advertises no tools, and adds zero per-request cost, so behaviour
//! is byte-identical to a build without MCP.
//!
//! ## Tool naming
//!
//! A remote tool `echo` on a server named `My Server` is advertised to the model
//! as `mcp__my_server__echo`. The server-name segment is sanitised
//! (lowercased; every non-`[a-z0-9_]` run collapses to `_`) so the namespaced
//! name is a stable, model-safe identifier. The manager keeps the reverse map
//! `namespaced -> (server uuid, original tool name)` so a call can be dispatched.
//!
//! ## Two backends: Local vs Proxy
//!
//! [`McpManager`] is a thin facade over an internal [`McpBackend`]:
//!
//! - **`Local`** — the manager OWNS the `rmcp` connections (the behaviour described
//!   below). Built by [`McpManager::connect_all`]; used by the standalone `--local`
//!   TUI and as the session-daemon's fallback when the global MCP daemon is
//!   unreachable.
//! - **`Proxy`** — the manager owns NO connections; it forwards every request over
//!   the shared frame codec to the GLOBAL MCP daemon (`koma --mcp-daemon`), which
//!   owns the one real `Local` manager. Built by [`McpManager::connect_proxy`]; used
//!   by session-daemons so N sessions share ONE copy of every heavyweight MCP server
//!   instead of each spawning their own. `tool_defs`/`tool_names` are served from a
//!   cache populated at connect (and refreshed on `reconnect`); `execute_blocking`
//!   /`server_status` do a fresh connect-per-call to the daemon socket.
//!
//! Every public method matches on the backend, so the call sites (ToolCtx dispatch,
//! the `/mcp` panel, the reconnect handler) are identical for both.
//!
//! ## Concurrency model (Local backend)
//!
//! The `rmcp` connections are async and must live on the app's tokio runtime (a
//! stdio connection owns the child process; dropping the service kills the child).
//! The manager therefore holds the runtime [`Handle`] and stores each live
//! [`RunningService`] on it.
//!
//! - **Startup is non-blocking.** [`McpManager::connect_all`] returns immediately;
//!   each server connects in a background task spawned on the handle, bounded by a
//!   connect timeout. A slow/failed server never freezes the app — it just
//!   contributes zero tools and a logged status. Tools appear in the snapshot once
//!   their server finishes connecting.
//! - **Dispatch is a sync→async bridge.** [`Tool::run`](crate::tool::Tool::run) is
//!   synchronous and may or may not run inside a tokio context, so we cannot use
//!   `Handle::block_on` (it panics inside a runtime). Instead
//!   [`McpManager::execute_blocking`] clones the owning server's `Peer` out of the
//!   snapshot, `spawn`s the async `call_tool` on the runtime handle, and blocks the
//!   calling thread on an `mpsc::recv_timeout` until the result lands (the same
//!   channel pattern as `tool::internet::http_get_blocking`, but spawned onto the
//!   existing runtime rather than a fresh thread, because the connection lives on
//!   that runtime).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, Tool as RmcpTool};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;

use crate::dto::openrouter::{ToolDef, ToolFunctionDef};
use crate::ipc::mcp_proto::{McpRequest, McpResponse};
use crate::model::app_config::{McpServerEntry, McpTransport};

// Sync proxy-wire IO (`proxy_request`) and naming/result-flattening helpers
// (`namespace_tools`/`sanitize_server_name`/`flatten_result`) live in the
// sibling `proxy`/`util` modules (file size); re-imported here so every
// existing bare call site in this file keeps compiling unchanged.
mod proxy;
mod util;
use proxy::proxy_request;
use util::flatten_result;

// The connect machinery (`connect_all`/`connect_proxy`/`reconnect`/
// `spawn_connect`, roughly the first half of `impl McpManager`) lives in the
// sibling `connect` module (file size) as another `impl McpManager` block —
// no `use` needed here, it's reached through the type, not this module's path.
mod connect;

/// How long a single server connect (spawn + MCP initialize + first tool list)
/// may take before it is abandoned. A hung server then contributes zero tools
/// instead of stalling forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a single `call_tool` round-trip may take before [`McpManager::execute_blocking`]
/// gives up and returns an error string to the model.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// A discovered remote tool, namespaced for advertisement.
#[derive(Clone)]
struct DiscoveredTool {
    /// Namespaced name advertised to the model: `mcp__<server>__<tool>`.
    namespaced: String,
    /// Description as reported by the server (empty when the server omits one).
    description: String,
    /// The tool's raw JSON-Schema parameters object (verbatim from the server).
    parameters: serde_json::Value,
    /// uuid of the owning server entry (used to find the live `Peer` to call).
    server_uuid: String,
    /// The tool's ORIGINAL (un-namespaced) name, as the server knows it.
    original: String,
}

/// One live MCP server connection plus the tools discovered on it.
struct ServerConn {
    /// The running rmcp client service. Held so the connection (and, for stdio, the
    /// child process) stays alive for the lifetime of the manager. Never read
    /// directly — calls go through the cloned [`Self::peer`] — but its ownership IS
    /// the connection, so it must not be dropped.
    #[allow(dead_code)]
    service: RunningService<RoleClient, ()>,
    /// A cheap clone handle onto the service, used to issue `call_tool` requests
    /// from the dispatch path. `Peer` is `Clone + Send + Sync`.
    peer: Peer<RoleClient>,
}

/// Mutable snapshot the manager exposes: the live connections (by server uuid) and
/// the flattened list of discovered tools. Guarded by a single mutex because it is
/// written by the background connect tasks and read by the (synchronous) UI/tool
/// threads.
#[derive(Default)]
struct Snapshot {
    /// Live connections keyed by server uuid. A server that failed to connect has
    /// no entry here.
    conns: HashMap<String, ServerConn>,
    /// All discovered tools across all connected servers, in connection order.
    tools: Vec<DiscoveredTool>,
    /// Monotonic config generation. Bumped by every [`McpManager::reconnect`] under
    /// the snapshot lock. A background connect task captures the generation BEFORE
    /// its `connect_one` await and re-checks it AFTER (under the lock, before
    /// inserting): if a `reconnect` bumped the generation while the connect was in
    /// flight, the task's result belongs to a torn-down config and is discarded.
    /// This stops a slow in-flight connect from an OLD generation reappearing in
    /// the snapshot ~20s after the user deleted that server (and from a reused uuid
    /// double-inserting).
    generation: u64,
}

/// How long the [`McpBackend::Proxy`] waits for the global MCP daemon to answer a
/// single request before giving up. Sized to just outrun the daemon's own
/// [`CALL_TIMEOUT`] on a `Call` (the daemon returns a timeout string of its own at
/// 60s, so the proxy's read should not fire first), with a small margin for the
/// round-trip. A hung/vanished daemon then surfaces as an error string to the model
/// instead of hanging the tool thread forever.
const PROXY_IO_TIMEOUT: Duration = Duration::from_secs(65);

/// The internal backend of an [`McpManager`]: either it OWNS the connections
/// (`Local`) or it forwards to the global MCP daemon that does (`Proxy`). See the
/// module docs ("Two backends").
enum McpBackend {
    /// This manager owns the live `rmcp` connections. Holds the runtime [`Handle`]
    /// (so async work can be spawned from sync code) and a mutex-guarded
    /// [`Snapshot`] of live connections + discovered tools.
    Local {
        handle: tokio::runtime::Handle,
        snapshot: Mutex<Snapshot>,
    },
    /// This manager owns NO connections; it proxies to the global MCP daemon at
    /// `sock`. `cache` holds the `(defs, names)` fetched at connect (and refreshed
    /// on `reconnect`) so the hot advertise accessors (`tool_defs`/`tool_names`)
    /// answer WITHOUT a socket round-trip; `execute_blocking`/`server_status` do a
    /// fresh connect-per-call.
    Proxy {
        sock: PathBuf,
        cache: Mutex<(Vec<ToolDef>, Vec<String>)>,
    },
}

/// How long a cached [`McpManager::server_status_cached`] answer is served before a
/// background refresh is kicked off. Short enough that the `/mcp` panel feels live,
/// long enough that a per-frame/per-tick reader (the panel render, the snapshot
/// projection) never pays the underlying `server_status()` cost — which, on the
/// `Proxy` backend, is a blocking unix-socket round-trip to the global MCP daemon.
const STATUS_CACHE_TTL: Duration = Duration::from_secs(2);

/// The global MCP client manager — a facade over an [`McpBackend`].
///
/// A `Local` backend owns the `rmcp` connections (the runtime [`Handle`] + a
/// mutex-guarded [`Snapshot`]); a `Proxy` backend forwards to the global MCP daemon.
/// Every public method matches on the backend, so call sites are backend-agnostic.
/// Cloned cheaply via the `Arc` returned by [`Self::connect_all`] / [`Self::connect_proxy`].
pub struct McpManager {
    backend: McpBackend,
    /// Cache for [`Self::server_status_cached`]: `(last-refreshed-at, last value)`.
    /// `None` timestamp means "never refreshed yet" (first call after construction).
    /// Guarded independently of the backend's own state so a refresh never contends
    /// with `tool_defs`/`execute_blocking`.
    status_cache: Mutex<(Option<std::time::Instant>, std::collections::HashMap<String, usize>)>,
    /// Single-flight guard for the background refresh spawned by
    /// [`Self::server_status_cached`]: only one refresh may be in flight at a time,
    /// so N rapid callers (panel render + snapshot projection, both ~10Hz) don't each
    /// kick off their own blocking `server_status()` call.
    status_refreshing: std::sync::atomic::AtomicBool,
    /// Single-flight guard for the background advertise-cache refresh spawned by
    /// [`Self::advertise_cached`] on the `Proxy` backend (mirrors `status_refreshing`):
    /// only one `McpRequest::List` refresh may be in flight at a time.
    advertise_refreshing: std::sync::atomic::AtomicBool,
    /// Last-refreshed timestamp for the `Proxy` advertise cache. `None` = never
    /// refreshed since construction, so the connect-time prime (which races the global
    /// daemon's background server connect and can be seeded EMPTY) is treated as stale
    /// and the first [`Self::advertise_cached`] call kicks a refresh. Local backends
    /// ignore this (they serve live).
    advertise_cache_at: Mutex<Option<std::time::Instant>>,
}

impl McpManager {
    /// Wire [`ToolDef`]s for every discovered tool, ready to extend the request
    /// `tools` array. Empty when nothing has connected yet (or no servers are
    /// configured), so the advertise path pays nothing.
    pub fn tool_defs(&self) -> Vec<ToolDef> {
        match &self.backend {
            McpBackend::Local { snapshot, .. } => {
                let snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
                snap.tools
                    .iter()
                    .map(|t| ToolDef {
                        kind: "function".into(),
                        function: ToolFunctionDef {
                            name: t.namespaced.clone(),
                            description: t.description.clone(),
                            parameters: t.parameters.clone(),
                        },
                    })
                    .collect()
            }
            // Proxy: serve from the cache primed at connect / refreshed on reconnect.
            McpBackend::Proxy { cache, .. } => {
                cache.lock().unwrap_or_else(|p| p.into_inner()).0.clone()
            }
        }
    }

    /// The namespaced names of every discovered tool, for the advertise allow-list
    /// (the stream filter keeps only tools whose name is in the advertise set, so
    /// the MCP names must be appended there or they would be dropped).
    pub fn tool_names(&self) -> Vec<String> {
        match &self.backend {
            McpBackend::Local { snapshot, .. } => {
                let snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
                snap.tools.iter().map(|t| t.namespaced.clone()).collect()
            }
            // Proxy: serve the cached names (kept in lockstep with the cached defs).
            McpBackend::Proxy { cache, .. } => {
                cache.lock().unwrap_or_else(|p| p.into_inner()).1.clone()
            }
        }
    }

    /// Advertise accessors ([`Self::tool_defs`] + [`Self::tool_names`]) for the
    /// request-build path, returning both in ONE call so defs and names always come
    /// from the same read.
    ///
    /// - **Local** — the snapshot walk is pure in-memory, so serve it live.
    /// - **Proxy** — the `(defs, names)` cache primed at `connect_proxy` races the
    ///   global daemon's own (background) server connect and can be seeded EMPTY. An
    ///   empty cache is exactly the cold-start window (first run, or a freshly spawned
    ///   session daemon on `/new`) where the model needs the tools THIS turn, so on an
    ///   empty cache we pay a single blocking live `McpRequest::List` INLINE (the
    ///   daemon answers a List straight from its snapshot, so it is fast; bounded by
    ///   `proxy_request`'s IO timeout). Once the daemon has connected its servers this
    ///   returns them immediately; while it is still connecting it returns empty and we
    ///   simply retry inline next turn. A NON-empty cache is served immediately and, if
    ///   stale, refreshed by a single-flight BACKGROUND `List` (never blocking the warm
    ///   path) so a later-added/removed server is eventually reflected.
    pub fn advertise_cached(self: &Arc<Self>) -> (Vec<ToolDef>, Vec<String>) {
        use std::sync::atomic::Ordering;

        // Local: pure in-memory snapshot walk — serve live, no cache needed.
        let cache = match &self.backend {
            McpBackend::Local { .. } => return (self.tool_defs(), self.tool_names()),
            McpBackend::Proxy { cache, .. } => cache,
        };

        let (defs, names, empty) = {
            let c = cache.lock().unwrap_or_else(|p| p.into_inner());
            (c.0.clone(), c.1.clone(), c.0.is_empty())
        };

        // Empty cache: cold-start window — fetch live INLINE so the tools are on the
        // wire this turn instead of a turn later.
        if empty {
            if let McpBackend::Proxy { sock, cache } = &self.backend {
                match proxy_request(sock, &McpRequest::List) {
                    Ok(McpResponse::Tools { defs, names }) => {
                        *cache.lock().unwrap_or_else(|p| p.into_inner()) =
                            (defs.clone(), names.clone());
                        *self
                            .advertise_cache_at
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = Some(std::time::Instant::now());
                        return (defs, names);
                    }
                    Ok(_) => {}
                    Err(e) => crate::model::store::append_global_error_log(
                        "mcp",
                        &format!("proxy: inline advertise List failed: {e:#}"),
                    ),
                }
            }
            // Daemon not ready yet (or a transient error) — return the empty cache;
            // the next turn retries inline.
            return (defs, names);
        }

        // Non-empty cache: serve it now; if stale, kick a single-flight BACKGROUND
        // refresh so a later config change is eventually reflected without blocking.
        let stale = match *self.advertise_cache_at.lock().unwrap_or_else(|p| p.into_inner()) {
            Some(at) => at.elapsed() >= STATUS_CACHE_TTL,
            None => true,
        };

        if stale
            && self
                .advertise_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            let mgr = Arc::clone(self);
            std::thread::spawn(move || {
                if let McpBackend::Proxy { sock, cache } = &mgr.backend {
                    match proxy_request(sock, &McpRequest::List) {
                        Ok(McpResponse::Tools { defs, names }) => {
                            *cache.lock().unwrap_or_else(|p| p.into_inner()) = (defs, names);
                        }
                        Ok(_) => {}
                        Err(e) => crate::model::store::append_global_error_log(
                            "mcp",
                            &format!("proxy: advertise refresh List failed: {e:#}"),
                        ),
                    }
                }
                *mgr.advertise_cache_at.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(std::time::Instant::now());
                mgr.advertise_refreshing.store(false, Ordering::Release);
            });
        }

        (defs, names)
    }

    /// Per-server discovered-tool count, keyed by server uuid.
    ///
    /// Read by the `/mcp` dashboard to show a LIVE status next to each configured
    /// server (`● N tools` when connected, `○ —` otherwise). Built from the live
    /// snapshot: a server that connected has an entry in `conns` (so it appears in
    /// the map even with zero tools, distinguishing "connected, no tools" from
    /// "not connected" — the latter is simply absent). The count comes from the
    /// flattened `tools` list, tallied by `server_uuid`. Best-effort + cheap: it
    /// just locks the snapshot and walks two small collections.
    pub fn server_status(&self) -> std::collections::HashMap<String, usize> {
        match &self.backend {
            McpBackend::Local { snapshot, .. } => {
                let snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
                // Seed every CONNECTED server at 0 so one with no tools still shows as
                // connected; then tally the discovered tools by their owning server.
                let mut counts: std::collections::HashMap<String, usize> =
                    snap.conns.keys().map(|uuid| (uuid.clone(), 0)).collect();
                for t in &snap.tools {
                    *counts.entry(t.server_uuid.clone()).or_insert(0) += 1;
                }
                counts
            }
            // Proxy: ask the global daemon for the live per-server map. Best-effort —
            // any connect/decode failure (or an unexpected reply) reads as an EMPTY map
            // so the `/mcp` panel degrades to "no live status" rather than erroring.
            McpBackend::Proxy { sock, .. } => match proxy_request(sock, &McpRequest::Status) {
                Ok(McpResponse::Status(map)) => map,
                _ => std::collections::HashMap::new(),
            },
        }
    }

    /// Cached, non-blocking twin of [`Self::server_status`] for HOT call sites (the
    /// `/mcp` panel render and the snapshot projection, both invoked at up to ~10Hz).
    ///
    /// Returns the last-known status map immediately. If the cache is empty or older
    /// than [`STATUS_CACHE_TTL`], it also kicks off a refresh:
    /// - **Local** — `server_status()` is a cheap in-memory snapshot walk (no IO), so
    ///   the refresh runs inline, synchronously, before returning.
    /// - **Proxy** — `server_status()` does a blocking unix-socket round-trip to the
    ///   global MCP daemon, so the refresh is spawned onto a background `std::thread`;
    ///   this call returns the STALE cached value (or an empty map on the very first
    ///   call) immediately, and the next call after the refresh lands sees the fresh
    ///   value.
    ///
    /// A single-flight [`AtomicBool`](std::sync::atomic::AtomicBool) guard
    /// (`status_refreshing`) ensures only one refresh is ever in flight, so N callers
    /// racing past a stale cache don't each open their own daemon connection.
    pub fn server_status_cached(self: &Arc<Self>) -> std::collections::HashMap<String, usize> {
        use std::sync::atomic::Ordering;

        let stale = {
            let cache = self.status_cache.lock().unwrap_or_else(|p| p.into_inner());
            match cache.0 {
                Some(at) => at.elapsed() >= STATUS_CACHE_TTL,
                None => true,
            }
        };

        if stale
            && self
                .status_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            match &self.backend {
                // Local: no IO, so refresh inline — no thread spawn needed, and the
                // caller gets the fresh value on this very call.
                McpBackend::Local { .. } => {
                    let fresh = self.server_status();
                    *self.status_cache.lock().unwrap_or_else(|p| p.into_inner()) =
                        (Some(std::time::Instant::now()), fresh);
                    self.status_refreshing.store(false, Ordering::Release);
                }
                // Proxy: blocking daemon round-trip, so refresh off-thread and serve
                // the stale value now; the next call picks up the fresh one.
                McpBackend::Proxy { .. } => {
                    let mgr = Arc::clone(self);
                    std::thread::spawn(move || {
                        let fresh = mgr.server_status();
                        *mgr.status_cache.lock().unwrap_or_else(|p| p.into_inner()) =
                            (Some(std::time::Instant::now()), fresh);
                        mgr.status_refreshing.store(false, Ordering::Release);
                    });
                }
            }
        }

        self.status_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .1
            .clone()
    }

    /// Execute a namespaced MCP tool call and return its flattened text result.
    ///
    /// - **Local** — THE SYNC→ASYNC BRIDGE: looks up `(server, original tool name)`
    ///   for `namespaced_name`, clones the owning server's `Peer`, runs the async
    ///   `call_tool` ON the runtime handle, and blocks this (synchronous,
    ///   possibly-in-runtime) thread on an `mpsc::recv_timeout`. We deliberately do
    ///   NOT use `Handle::block_on` because `Tool::run` may already be executing
    ///   inside the tokio runtime, where `block_on` panics. The
    ///   [`rmcp::model::CallToolResult`] content blocks are flattened into one string;
    ///   a server-flagged error (`is_error == Some(true)`) is returned as `Err(...)`.
    /// - **Proxy** — forwards a [`McpRequest::Call`] to the global MCP daemon over the
    ///   shared frame codec (a fresh connect-per-call: simple + robust; the daemon
    ///   fans concurrent connections out itself) and returns its [`McpResponse`].
    ///   The read is bounded by [`PROXY_IO_TIMEOUT`] so a hung daemon surfaces an
    ///   error string instead of hanging the tool thread forever. The daemon folds a
    ///   tool-level error into [`McpResponse::CallResult`] (so it comes back as `Ok`,
    ///   exactly as the model expects a tool error); a PROTOCOL fault comes back as
    ///   [`McpResponse::Error`] → `Err`.
    pub fn execute_blocking(
        &self,
        namespaced_name: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let (handle, snapshot) = match &self.backend {
            McpBackend::Local { handle, snapshot } => (handle, snapshot),
            // PROXY: forward the call to the global daemon. `server_uuid` is left
            // empty — the daemon resolves the owning server from the namespaced
            // `tool` name (matching the in-process resolution below).
            McpBackend::Proxy { sock, .. } => {
                let req = McpRequest::Call {
                    server_uuid: String::new(),
                    tool: namespaced_name.to_string(),
                    args: args.to_string(),
                };
                return match proxy_request(sock, &req) {
                    // The daemon already folded any tool-level error into the result
                    // string, so a CallResult is always the model-facing tool output.
                    Ok(McpResponse::CallResult(s)) => Ok(s),
                    // A protocol fault (malformed args, etc.) is a real error.
                    Ok(McpResponse::Error(e)) => Err(e),
                    Ok(other) => Err(format!(
                        "MCP tool '{namespaced_name}': unexpected daemon response {other:?}"
                    )),
                    // Connect/timeout/decode failure talking to the daemon.
                    Err(e) => Err(format!(
                        "MCP tool '{namespaced_name}': global daemon call failed: {e:#}"
                    )),
                };
            }
        };

        // --- Local dispatch (unchanged) ---
        // Resolve the owning server + original tool name, and clone the Peer so the
        // async closure owns it (and we drop the snapshot lock before spawning).
        let (peer, original) = {
            let snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
            let tool = snap
                .tools
                .iter()
                .find(|t| t.namespaced == namespaced_name)
                .ok_or_else(|| format!("unknown MCP tool '{namespaced_name}'"))?;
            let conn = snap.conns.get(&tool.server_uuid).ok_or_else(|| {
                format!("MCP server for tool '{namespaced_name}' is not connected")
            })?;
            (conn.peer.clone(), tool.original.clone())
        };

        // Convert the JSON arguments into the `JsonObject` (serde_json::Map) rmcp
        // wants. A non-object payload (or `{}`) becomes "no arguments".
        let arguments = match args {
            serde_json::Value::Object(map) if !map.is_empty() => Some(map.clone()),
            _ => None,
        };

        // Channel + recv_timeout: the spawned async task sends the call result back;
        // this thread blocks until it lands or the timeout fires. Mirrors
        // `tool::internet::http_get_blocking`, but spawns onto the runtime handle
        // (where the connection lives) instead of a fresh OS thread.
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        let original_for_task = original.clone();
        handle.spawn(async move {
            let mut params = CallToolRequestParams::new(original_for_task);
            if let Some(map) = arguments {
                params = params.with_arguments(map);
            }
            let result = match peer.call_tool(params).await {
                Ok(res) => flatten_result(res),
                Err(e) => Err(format!("call failed: {e}")),
            };
            let _ = tx.send(result);
        });

        match rx.recv_timeout(CALL_TIMEOUT) {
            Ok(r) => r,
            // Distinguish a real timeout (the task is still running but slow) from a
            // dropped sender (the spawned task vanished without sending — e.g. the
            // runtime was shut down), so the model sees the actual cause.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(format!(
                "MCP tool '{namespaced_name}' timed out after {}s",
                CALL_TIMEOUT.as_secs()
            )),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(format!(
                "MCP tool '{namespaced_name}' task dropped before result (runtime shut down?)"
            )),
        }
    }
}

/// Connect to a single server (spawn/initialize the rmcp client and list its
/// tools), bounded by [`CONNECT_TIMEOUT`]. Returns the live service plus the
/// discovered tools.
async fn connect_one(
    server: &McpServerEntry,
) -> Result<(RunningService<RoleClient, ()>, Vec<RmcpTool>), String> {
    // The whole connect (transport build + MCP initialize + first tool list) is
    // wrapped in a single timeout so a server that hangs at any stage is abandoned.
    let fut = async {
        let service: RunningService<RoleClient, ()> = match server.transport {
            McpTransport::Stdio => {
                if server.command.trim().is_empty() {
                    return Err("stdio server has no command".to_string());
                }
                // Build the child-process transport: the configured command + args,
                // plus any configured environment variables.
                let args = server.args.clone();
                let env = server.env.clone();
                let cmd = tokio::process::Command::new(&server.command).configure(|c| {
                    for a in &args {
                        c.arg(a);
                    }
                    for (k, v) in &env {
                        c.env(k, v);
                    }
                });
                let transport = TokioChildProcess::new(cmd)
                    .map_err(|e| format!("spawn '{}' failed: {e}", server.command))?;
                ().serve(transport)
                    .await
                    .map_err(|e| format!("initialize failed: {e}"))?
            }
            McpTransport::Http => {
                if server.url.trim().is_empty() {
                    return Err("http server has no url".to_string());
                }
                let transport = StreamableHttpClientTransport::from_uri(server.url.clone());
                ().serve(transport)
                    .await
                    .map_err(|e| format!("initialize failed: {e}"))?
            }
        };

        let tools = service
            .list_all_tools()
            .await
            .map_err(|e| format!("list_tools failed: {e}"))?;
        Ok((service, tools))
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, fut).await {
        Ok(res) => res,
        Err(_) => Err(format!(
            "connect timed out after {}s",
            CONNECT_TIMEOUT.as_secs()
        )),
    }
}

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
//! ## Concurrency model
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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, Tool as RmcpTool};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;

use crate::dto::openrouter::{ToolDef, ToolFunctionDef};
use crate::model::app_config::{McpServerEntry, McpTransport};

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

/// The global MCP client manager.
///
/// Holds the runtime [`Handle`] (so async work can be spawned from sync code) and a
/// mutex-guarded [`Snapshot`] of live connections + discovered tools. Cloned cheaply
/// via the `Arc` returned by [`Self::connect_all`].
pub struct McpManager {
    handle: tokio::runtime::Handle,
    snapshot: Mutex<Snapshot>,
}

impl McpManager {
    /// Build the manager and kick off a background connect for every ENABLED
    /// server. Returns immediately — connecting never blocks startup.
    ///
    /// With no enabled servers this is effectively a no-op constructor: the
    /// snapshot stays empty, so [`Self::tool_defs`] / [`Self::tool_names`] are empty
    /// and no task is spawned.
    pub fn connect_all(
        handle: &tokio::runtime::Handle,
        servers: &[McpServerEntry],
    ) -> Arc<Self> {
        let manager = Arc::new(Self {
            handle: handle.clone(),
            snapshot: Mutex::new(Snapshot::default()),
        });

        for server in servers {
            if !server.enabled {
                continue;
            }
            // One independent background connect per enabled server (see
            // `spawn_connect`): a hang or failure on one never blocks startup or
            // affects the others.
            manager.spawn_connect(server.clone());
        }

        manager
    }

    /// Apply a NEW server set live: tear down the current connections (so their
    /// child processes terminate) and reconnect from `servers`, all in the
    /// background. Returns immediately — the caller (a `/mcp` save/delete handler)
    /// is never blocked on teardown or reconnect.
    ///
    /// With no enabled servers this just clears the snapshot and spawns nothing, so
    /// "remove the last server" cleanly drops to zero tools.
    ///
    /// ## Concurrency
    ///
    /// The snapshot mutex is held ONLY to swap out the old `conns`/`tools` (a quick
    /// `std::mem::take`), then released before any `.await`: the old connections are
    /// torn down on a spawned task, and each reconnect runs on its own spawned task
    /// (via [`Self::spawn_connect`]). The lock is never held across an `.await` or
    /// across a spawn.
    pub fn reconnect(self: &Arc<Self>, servers: &[McpServerEntry]) {
        // Take the old connections out under the lock, then drop the guard BEFORE
        // doing any async teardown. `tools` is cleared here so stale tools stop
        // being advertised immediately; the new tools repopulate as servers
        // reconnect. (Holding the lock across the teardown await would violate the
        // no-lock-across-await rule and could deadlock the sync readers.)
        let old_conns: Vec<ServerConn> = {
            let mut snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
            snap.tools.clear();
            // Bump the generation under the SAME lock that clears conns+tools, so any
            // connect task spawned for the OLD config (which captured the previous
            // generation before its await) sees a mismatch when it re-locks to insert
            // and discards its now-stale result. Wrapping-add is just defensive; this
            // counter realistically never overflows.
            snap.generation = snap.generation.wrapping_add(1);
            std::mem::take(&mut snap.conns).into_values().collect()
        };

        // Tear down the old connections off-thread: `RunningService::cancel` is async
        // (it cancels the service and awaits cleanup, terminating the stdio child).
        // Best-effort — a failed cancel still drops the service, whose drop guard
        // aborts it. We do NOT block the caller on this.
        if !old_conns.is_empty() {
            self.handle.spawn(async move {
                for conn in old_conns {
                    if let Err(e) = conn.service.cancel().await {
                        eprintln!("mcp: teardown of a connection failed: {e}");
                    }
                }
            });
        }

        // Reconnect every enabled server, each on its own background task.
        for server in servers {
            if !server.enabled {
                continue;
            }
            self.spawn_connect(server.clone());
        }
    }

    /// Spawn ONE background connect task for `server` and write its result into the
    /// shared snapshot. The single place the connect-and-store routine lives, shared
    /// by [`Self::connect_all`] (startup) and [`Self::reconnect`] (live config save).
    ///
    /// ## Concurrency
    ///
    /// The snapshot lock is acquired only AFTER `connect_one` has awaited to
    /// completion — NEVER across the `.await` — and dropped before the task ends. A
    /// failed connect logs and contributes zero tools.
    ///
    /// ## Generation guard (stale-result discard)
    ///
    /// The task captures the snapshot's `generation` under a BRIEF lock at the very
    /// start (before the connect await), then re-checks it under the lock AFTER the
    /// await, before inserting. If a [`Self::reconnect`] bumped the generation while
    /// this connect was in flight (e.g. the user deleted the server, then it
    /// finished connecting ~20s later), the captured and current generations differ:
    /// the freshly built [`ServerConn`] is discarded (its drop guard cancels the
    /// service + terminates any stdio child) and NOTHING is inserted. Both lock
    /// regions are synchronous — the generation read and the check+insert each
    /// acquire, use, and drop the guard with no `.await` held across it.
    ///
    /// ## Duplicate-prefix guard (sanitized-name collision)
    ///
    /// Two distinct server names can sanitize to the SAME `<server>` segment (e.g.
    /// "My Server" and "my-server" both -> "my_server"), producing colliding
    /// `mcp__my_server__*` prefixes. `execute_blocking` resolves a call by the FIRST
    /// matching namespaced name, so the second server's same-named tools would be
    /// silently mis-dispatched to the first. When the post-await insert detects that
    /// the snapshot already holds tools with this server's sanitized prefix from a
    /// DIFFERENT server uuid, it logs a warning and SKIPS this server entirely
    /// (tools dropped, conn discarded) rather than advertise tools it can't dispatch
    /// correctly. This check is synchronous, under the same insert lock.
    fn spawn_connect(self: &Arc<Self>, server: McpServerEntry) {
        let mgr = Arc::clone(self);
        self.handle.spawn(async move {
            // Capture the CURRENT generation under a brief lock BEFORE the connect
            // await. If a reconnect bumps it while we're connecting, the post-await
            // re-check below will see the mismatch and discard this result.
            let gen_at_start = {
                let snap = mgr.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                snap.generation
            };

            match connect_one(&server).await {
                Ok((service, tools)) => {
                    let peer = service.peer().clone();
                    let discovered = namespace_tools(&server, &tools);
                    // The sanitized `<server>` segment this connection's tools live
                    // under, with the full `mcp__<prefix>__` boundary so a longer
                    // name (e.g. "my_server_2") can't false-match "my_server".
                    let my_full_prefix = format!("mcp__{}__", sanitize_server_name(&server.name));

                    // Hold the service in an Option so the lock region can MOVE it
                    // into the snapshot on the keep path; whatever is left here after
                    // the block (Some on a discard path, None on keep) is torn down
                    // outside the lock — keeping `service` referenceable after a
                    // CONDITIONAL move without upsetting the borrow checker.
                    let mut to_discard: Option<RunningService<RoleClient, ()>> = Some(service);
                    {
                        // Lock taken only now (post-await), released at end of this
                        // block — never held across an await.
                        let mut snap = mgr.snapshot.lock().unwrap_or_else(|p| p.into_inner());

                        if snap.generation != gen_at_start {
                            // A reconnect happened mid-connect: this result belongs to
                            // a torn-down config. Leave `to_discard` = Some(service) so
                            // it's cancelled below; insert nothing.
                        } else if snap.tools.iter().any(|t| {
                            t.namespaced.starts_with(&my_full_prefix)
                                && t.server_uuid != server.uuid
                        }) {
                            // Another server already occupies this sanitized prefix.
                            // Advertising these tools would let execute_blocking
                            // mis-route by name, so skip this server entirely (tools
                            // dropped, conn cancelled below).
                            eprintln!(
                                "mcp: server '{}' sanitizes to prefix '{}' already used by \
                                 another configured server; skipping its tools to avoid \
                                 mis-dispatch (rename one of the servers to fix)",
                                server.name, my_full_prefix
                            );
                        } else {
                            // Keep it: move the service into the snapshot and record
                            // its tools. `take()` leaves `to_discard = None` so nothing
                            // is torn down afterwards.
                            let service = to_discard.take().expect("service present");
                            snap.conns
                                .insert(server.uuid.clone(), ServerConn { service, peer });
                            snap.tools.extend(discovered);
                        }
                    }

                    // Tear down a discarded connection OUTSIDE the lock (no guard held
                    // across this await). Best-effort: a failed cancel still drops the
                    // service, whose drop guard aborts it + terminates any stdio child.
                    if let Some(service) = to_discard {
                        if let Err(e) = service.cancel().await {
                            eprintln!("mcp: teardown of a discarded connection failed: {e}");
                        }
                    }
                }
                Err(e) => {
                    // A failed server = logged status + zero tools. Never a panic or
                    // a hang; the rest of the app proceeds as if this server were
                    // absent.
                    eprintln!("mcp: server '{}' failed to connect: {e}", server.name);
                }
            }
        });
    }

    /// Wire [`ToolDef`]s for every discovered tool, ready to extend the request
    /// `tools` array. Empty when nothing has connected yet (or no servers are
    /// configured), so the advertise path pays nothing.
    pub fn tool_defs(&self) -> Vec<ToolDef> {
        let snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
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

    /// The namespaced names of every discovered tool, for the advertise allow-list
    /// (the stream filter keeps only tools whose name is in the advertise set, so
    /// the MCP names must be appended there or they would be dropped).
    pub fn tool_names(&self) -> Vec<String> {
        let snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
        snap.tools.iter().map(|t| t.namespaced.clone()).collect()
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
        let snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
        // Seed every CONNECTED server at 0 so one with no tools still shows as
        // connected; then tally the discovered tools by their owning server.
        let mut counts: std::collections::HashMap<String, usize> =
            snap.conns.keys().map(|uuid| (uuid.clone(), 0)).collect();
        for t in &snap.tools {
            *counts.entry(t.server_uuid.clone()).or_insert(0) += 1;
        }
        counts
    }

    /// THE SYNC→ASYNC BRIDGE. Execute a namespaced MCP tool call and return its
    /// flattened text result.
    ///
    /// Looks up `(server, original tool name)` for `namespaced_name`, clones the
    /// owning server's `Peer`, then runs the async `call_tool` ON the runtime handle
    /// and blocks this (synchronous, possibly-in-runtime) thread on an
    /// `mpsc::recv_timeout`. We deliberately do NOT use `Handle::block_on` because
    /// `Tool::run` may already be executing inside the tokio runtime, where
    /// `block_on` panics.
    ///
    /// The [`rmcp::model::CallToolResult`] content blocks are flattened into one
    /// string (text blocks concatenated; non-text blocks noted). A result the
    /// server flagged as an error (`is_error == Some(true)`) is returned as
    /// `Err(...)` so the caller surfaces it as a tool error.
    pub fn execute_blocking(
        &self,
        namespaced_name: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        // Resolve the owning server + original tool name, and clone the Peer so the
        // async closure owns it (and we drop the snapshot lock before spawning).
        let (peer, original) = {
            let snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
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
        self.handle.spawn(async move {
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

/// Turn a server's raw rmcp tools into namespaced [`DiscoveredTool`]s.
fn namespace_tools(server: &McpServerEntry, tools: &[RmcpTool]) -> Vec<DiscoveredTool> {
    let prefix = sanitize_server_name(&server.name);
    tools
        .iter()
        .map(|t| {
            let original = t.name.to_string();
            DiscoveredTool {
                namespaced: format!("mcp__{prefix}__{original}"),
                description: t
                    .description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
                // `input_schema` is an `Arc<JsonObject>` (serde_json::Map); wrap it
                // back into a `Value::Object` so it rides the wire as the tool's
                // raw JSON-Schema `parameters`, exactly like a built-in tool.
                parameters: serde_json::Value::Object((*t.input_schema).clone()),
                server_uuid: server.uuid.clone(),
                original,
            }
        })
        .collect()
}

/// Sanitise a server name into the `<server>` segment of a namespaced tool name:
/// lowercase, and collapse every run of non-`[a-z0-9_]` characters to a single
/// `_`, trimming leading/trailing `_`. An empty/garbage name degrades to
/// `"server"` so the namespaced tool name is always well-formed.
fn sanitize_server_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_underscore = false;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            prev_underscore = c == '_';
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "server".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Flatten an [`rmcp::model::CallToolResult`] into a single string for the model.
///
/// Text content blocks are concatenated (newline-separated); non-text blocks are
/// noted by kind so the model knows something non-textual came back. When the
/// server flagged the result as an error, the flattened text is returned as
/// `Err(...)` so the dispatcher renders it as a tool error.
fn flatten_result(res: rmcp::model::CallToolResult) -> Result<String, String> {
    use rmcp::model::RawContent;

    let mut parts: Vec<String> = Vec::new();
    for c in &res.content {
        // `Content` derefs to `RawContent`; match the underlying variant.
        match &c.raw {
            RawContent::Text(t) => parts.push(t.text.clone()),
            RawContent::Image(_) => parts.push("[image content]".to_string()),
            RawContent::Audio(_) => parts.push("[audio content]".to_string()),
            RawContent::Resource(_) => parts.push("[embedded resource]".to_string()),
            RawContent::ResourceLink(_) => parts.push("[resource link]".to_string()),
        }
    }
    // Fall back to structured content if there were no content blocks at all.
    if parts.is_empty() {
        if let Some(sc) = &res.structured_content {
            parts.push(sc.to_string());
        }
    }
    let text = parts.join("\n");

    if res.is_error == Some(true) {
        Err(if text.is_empty() {
            "tool reported an error".to_string()
        } else {
            text
        })
    } else {
        Ok(text)
    }
}

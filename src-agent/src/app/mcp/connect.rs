//! Connect machinery for [`McpManager`](super::McpManager): building a fresh
//! `Local`/`Proxy` manager, live-reconnecting to a new server set, and the
//! per-server background connect task. Split out of [`super`] (the `mcp`
//! module) for file size — pure code motion, no behaviour change.
//!
//! Kept as an `impl McpManager` block on the SAME type defined in `mod.rs`;
//! every private field/type this touches (`McpManager`'s fields, `McpBackend`,
//! `Snapshot`, `ServerConn`, `connect_one`) is already visible here without any
//! bump — private items are visible to their defining module's descendants,
//! and `mcp::connect` is a descendant of `mcp`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::service::{RoleClient, RunningService};

use crate::ipc::mcp_proto::{McpRequest, McpResponse};
use crate::model::app_config::McpServerEntry;

use super::util::{namespace_tools, sanitize_server_name};
use super::{connect_one, McpBackend, McpManager, ServerConn, Snapshot, ToolSource};

impl McpManager {
    /// Build a LOCAL manager and kick off a background connect for every ENABLED
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
            backend: McpBackend::Local {
                handle: handle.clone(),
                snapshot: Mutex::new(Snapshot::default()),
            },
            status_cache: Mutex::new((None, std::collections::HashMap::new())),
            status_refreshing: std::sync::atomic::AtomicBool::new(false),
            advertise_refreshing: std::sync::atomic::AtomicBool::new(false),
            advertise_cache_at: Mutex::new(None),
            ext_manager: Mutex::new(None),
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

    /// Build a PROXY manager pointed at the global MCP daemon listening on `sock`
    /// (`~/.koma/mcp.sock`). Fetches the daemon's advertised `(defs, names)` up front
    /// via [`McpRequest::List`] and seeds the cache so the hot advertise accessors
    /// answer without a round-trip.
    ///
    /// Returns `Err` if the daemon can't be reached (or answers something other than
    /// [`McpResponse::Tools`]); the caller (`run_daemon`) uses that to FALL BACK to a
    /// `Local` manager, so a missing/broken daemon is never worse than today.
    ///
    /// A `handle` is accepted for signature symmetry with [`Self::connect_all`] (and
    /// so a future async-proxy variant can spawn on it); the current proxy is fully
    /// synchronous connect-per-call, so it is not stored.
    pub fn connect_proxy(
        _handle: &tokio::runtime::Handle,
        sock: PathBuf,
    ) -> anyhow::Result<Arc<Self>> {
        // Prime the advertise cache from the daemon. A connect/list failure here is
        // the fallback trigger — surface it so the caller drops to a Local manager.
        let (defs, names) = match super::proxy::proxy_request(&sock, &McpRequest::List)? {
            McpResponse::Tools { defs, names } => (defs, names),
            other => {
                return Err(anyhow::anyhow!(
                    "global MCP daemon answered List with an unexpected response: {other:?}"
                ))
            }
        };

        Ok(Arc::new(Self {
            backend: McpBackend::Proxy {
                sock,
                cache: Mutex::new((defs, names)),
            },
            status_cache: Mutex::new((None, std::collections::HashMap::new())),
            status_refreshing: std::sync::atomic::AtomicBool::new(false),
            advertise_refreshing: std::sync::atomic::AtomicBool::new(false),
            advertise_cache_at: Mutex::new(None),
            ext_manager: Mutex::new(None),
        }))
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
        // PROXY: the global daemon owns the real connections, so forward the new
        // server set to IT and then refresh our local advertise cache from a fresh
        // List. Best-effort: on any daemon error the cache is left as-is (stale but
        // usable) — a failed live-reconnect is never worse than the prior state.
        let (handle, snapshot) = match &self.backend {
            McpBackend::Local { handle, snapshot } => (handle, snapshot),
            McpBackend::Proxy { sock, cache } => {
                match super::proxy::proxy_request(sock, &McpRequest::Reconnect { servers: servers.to_vec() }) {
                    Ok(McpResponse::Ack) => {}
                    Ok(other) => crate::model::store::append_global_error_log(
                        "mcp",
                        &format!("proxy: reconnect got an unexpected response ({other:?}); cache left unchanged"),
                    ),
                    Err(e) => crate::model::store::append_global_error_log(
                        "mcp",
                        &format!("proxy: reconnect to global daemon failed: {e:#}"),
                    ),
                }
                // Refresh the advertise cache so the panel/advertise reflect the new
                // set once the daemon has applied it. A List failure leaves the old
                // cache in place.
                match super::proxy::proxy_request(sock, &McpRequest::List) {
                    Ok(McpResponse::Tools { defs, names }) => {
                        *cache.lock().unwrap_or_else(|p| p.into_inner()) = (defs, names);
                    }
                    Ok(_) => {}
                    Err(e) => crate::model::store::append_global_error_log(
                        "mcp",
                        &format!("proxy: post-reconnect List failed: {e:#}"),
                    ),
                }
                return;
            }
        };

        // Take the old connections out under the lock, then drop the guard BEFORE
        // doing any async teardown. `tools` is cleared here so stale tools stop
        // being advertised immediately; the new tools repopulate as servers
        // reconnect. (Holding the lock across the teardown await would violate the
        // no-lock-across-await rule and could deadlock the sync readers.)
        let old_conns: Vec<ServerConn> = {
            let mut snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
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
            handle.spawn(async move {
                for conn in old_conns {
                    if let Err(e) = conn.service.cancel().await {
                        crate::model::store::append_global_error_log(
                            "mcp",
                            &format!("teardown of a connection failed: {e}"),
                        );
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
        // spawn_connect is only ever reached on a Local backend (connect_all /
        // reconnect). Extract the runtime handle to spawn on; a Proxy manager has no
        // connections to spawn, so it is a no-op guard.
        let handle = match &self.backend {
            McpBackend::Local { handle, .. } => handle.clone(),
            McpBackend::Proxy { .. } => return,
        };
        let mgr = Arc::clone(self);
        handle.spawn(async move {
            // The snapshot lives on the Local backend; bail if this manager is a Proxy
            // (unreachable — spawn_connect returned early above — but keeps the match
            // total without an unwrap).
            let snapshot = match &mgr.backend {
                McpBackend::Local { snapshot, .. } => snapshot,
                McpBackend::Proxy { .. } => return,
            };
            // Capture the CURRENT generation under a brief lock BEFORE the connect
            // await. If a reconnect bumps it while we're connecting, the post-await
            // re-check below will see the mismatch and discard this result.
            let gen_at_start = {
                let snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
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
                        let mut snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());

                        if snap.generation != gen_at_start {
                            // A reconnect happened mid-connect: this result belongs to
                            // a torn-down config. Leave `to_discard` = Some(service) so
                            // it's cancelled below; insert nothing.
                        } else if snap.tools.iter().any(|t| {
                            t.namespaced.starts_with(&my_full_prefix)
                                && !matches!(&t.source, ToolSource::McpServer(u) if u == &server.uuid)
                        }) {
                            // Another server already occupies this sanitized prefix.
                            // Advertising these tools would let execute_blocking
                            // mis-route by name, so skip this server entirely (tools
                            // dropped, conn cancelled below).
                            crate::model::store::append_global_error_log(
                                "mcp",
                                &format!(
                                    "server '{}' sanitizes to prefix '{}' already used by \
                                     another configured server; skipping its tools to avoid \
                                     mis-dispatch (rename one of the servers to fix)",
                                    server.name, my_full_prefix
                                ),
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
                            crate::model::store::append_global_error_log(
                                "mcp",
                                &format!("teardown of a discarded connection failed: {e}"),
                            );
                        }
                    }
                }
                Err(e) => {
                    // A failed server = logged status + zero tools. Never a panic or
                    // a hang; the rest of the app proceeds as if this server were
                    // absent.
                    crate::model::store::append_global_error_log(
                        "mcp",
                        &format!("server '{}' failed to connect: {e}", server.name),
                    );
                }
            }
        });
    }
}

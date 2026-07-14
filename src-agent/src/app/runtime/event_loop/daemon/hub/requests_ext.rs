//! Extension-STORE arm bodies for [`super::core::DaemonHub`] — the GUI marketplace that
//! wires the live koma.run store API to koma's install pipeline (`crate::app::ext`). Split
//! out of `requests.rs` for file size, and mirrors `requests_oauth.rs`'s shape: a thin
//! `ext` router the `requests.rs` group arm calls, plus per-request handlers.
//!
//! # Ownership: DAEMON, not host-local
//!
//! Browse/detail hit the PUBLIC store endpoints (no auth), but install MUTATES live runtime
//! state — it verifies + unpacks a signed artifact, registers its `contributes` into the
//! live [`crate::app::mcp::McpManager`], and spawns a daemon-kind child via the live
//! [`crate::app::ext::ExtHostManager`] — so the whole family is owned by the session daemon
//! (which holds those managers + the authoritative `AppConfig`), never the GUI host.
//!
//! # Async split (mirrors `ListModels`)
//!
//! Every network fetch is spawned on the runtime and its result shipped back on the hub's
//! `store_tx` channel; [`DaemonHub::drain_store_replies`] (run once per tick by the daemon
//! loop) turns each landed reply into a seq'd [`DaemonEvent`] to the requesting client. The
//! INSTALL step is two-phase: the spawned task does platform-detect + bearer + download +
//! integrity read (all off-loop), then hands the raw zip + integrity fields back so the
//! drain runs the fail-closed verify + unpack + register + spawn ON the event loop, where it
//! alone has `&mut AppState` and the managers. All logging goes through
//! [`store::append_global_error_log`] — never `eprintln!`/`println!` (this is TUI-owning
//! runtime code).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::app::ext::ExtHostManager;
use crate::app::state::AppState;
use crate::ipc::proto::{
    ClientRequest, DaemonEvent, InstalledExtWire, PanelWire, StoreContributesWire,
    StoreDetailWire, StoreItemWire,
};
use crate::model::app_config::{InstalledExtension, OAuthProvider};
use crate::model::store;

use super::core::{DaemonHub, StoreReply};

/// Base URL of the koma.run extension store API (contract v0).
const STORE_API_BASE: &str = "https://koma.run/api/v1/extensions";

impl DaemonHub {
    /// Route the whole GUI extension-store family to its specific handler below — called
    /// from `requests.rs`'s single store group arm so that router stays thin. `requests.rs`
    /// only ever passes the five store variants, so the `_` catch-all is unreachable in
    /// practice.
    pub(super) fn ext(
        &mut self,
        idx: usize,
        req: ClientRequest,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
    ) {
        match req {
            ClientRequest::StoreBrowse { query, category } => {
                self.store_browse(idx, handle, query, category)
            }
            ClientRequest::StoreDetail { id } => self.store_detail(idx, handle, id),
            ClientRequest::InstallExtension { id, version } => {
                self.install_extension(idx, state, handle, id, version)
            }
            ClientRequest::UninstallExtension { id } => self.uninstall_extension(idx, state, id),
            ClientRequest::ListInstalledExtensions => self.list_installed_extensions(idx, state),
            ClientRequest::ExtPanelMsg {
                ext_id,
                panel_id,
                req_id,
                payload,
            } => self.panel_msg(idx, state, handle, ext_id, panel_id, req_id, payload),
            _ => {}
        }
    }

    /// Route one GUI panel→daemon message (W8 panel bridge) to its backing extension WITHOUT
    /// blocking the event loop. Snapshots the requesting client id + the manager `Arc` + the
    /// [`InstalledExtension`] record (enabled + kind, for the auto-start decision) up front, then
    /// runs the auto-start-if-needed + `panel.msg` invoke on `spawn_blocking` — BOTH
    /// [`ExtHostManager::ensure_started`] and [`ExtHostManager::invoke_with_timeout`] block the
    /// calling thread on a sync→async bridge, so they must NEVER run on the event-loop thread.
    /// The outcome ships back on the hub's `store_tx` as a [`StoreReply::PanelReply`] the per-tick
    /// `drain_store_replies` turns into a seq'd [`DaemonEvent::ExtPanelReply`] to the requester.
    /// A missing ext manager (no session runtime) replies `ok:false` synchronously (no task).
    fn panel_msg(
        &mut self,
        idx: usize,
        state: &AppState,
        handle: &tokio::runtime::Handle,
        ext_id: String,
        panel_id: String,
        req_id: Option<String>,
        payload: serde_json::Value,
    ) {
        let client_id = self.clients[idx].id;
        let tx = self.store_tx.clone();
        // Snapshot the registry record up front — the closure runs off the event loop and must
        // not borrow `state`. `None` = not installed (→ the decision's error path).
        let record = state
            .rest
            .config
            .installed_extensions
            .iter()
            .find(|e| e.id == ext_id)
            .cloned();

        // No live ext manager (no session runtime) → nothing can service the panel; reply now.
        let Some(mgr) = state.rest.ext_manager.clone() else {
            let _ = tx.send(StoreReply::PanelReply {
                client_id,
                ext_id,
                panel_id,
                req_id,
                ok: false,
                payload: None,
                error: Some("extension not available".to_string()),
            });
            return;
        };

        // Off-loop: auto-start (if needed) + invoke `panel.msg`, then ship the outcome back.
        handle.spawn_blocking(move || {
            let reply = run_panel_msg(&mgr, client_id, ext_id, panel_id, req_id, payload, &record);
            let _ = tx.send(reply);
        });
    }

    /// Browse the store catalogue (PUBLIC, no auth): spawn a `GET /extensions[?q&category]`
    /// and reply out-of-band with a [`DaemonEvent::StoreCatalogue`]. A network/parse error
    /// pushes an EMPTY catalogue + an `error` string (never a hang / panic).
    fn store_browse(
        &mut self,
        idx: usize,
        handle: &tokio::runtime::Handle,
        query: Option<String>,
        category: Option<String>,
    ) {
        let client_id = self.clients[idx].id;
        let tx = self.store_tx.clone();
        handle.spawn(async move {
            let reply = match fetch_catalogue(query, category).await {
                Ok(items) => StoreReply::Catalogue {
                    client_id,
                    items,
                    error: None,
                },
                Err(e) => StoreReply::Catalogue {
                    client_id,
                    items: Vec::new(),
                    error: Some(e),
                },
            };
            let _ = tx.send(reply);
        });
    }

    /// Fetch one extension's detail (PUBLIC, no auth): spawn a `GET /extensions/{id}` and
    /// reply with a [`DaemonEvent::StoreItemDetail`] (`detail: None` + `error` on failure).
    fn store_detail(&mut self, idx: usize, handle: &tokio::runtime::Handle, id: String) {
        let client_id = self.clients[idx].id;
        let tx = self.store_tx.clone();
        handle.spawn(async move {
            let reply = match fetch_detail(&id).await {
                Ok(detail) => StoreReply::Detail {
                    client_id,
                    detail: Some(detail),
                    error: None,
                },
                Err(e) => StoreReply::Detail {
                    client_id,
                    detail: None,
                    error: Some(e),
                },
            };
            let _ = tx.send(reply);
        });
    }

    /// Install `id`: detect platform + resolve the KomaRun bearer synchronously (both need
    /// `state`), then spawn the download. A missing platform / missing KomaRun sign-in is a
    /// synchronous [`DaemonEvent::ExtensionOpResult`] reply (no task); otherwise the spawned
    /// task fetches the bearer, downloads + reads integrity, and ships an
    /// [`StoreReply::InstallArtifact`] the drain verifies + installs on-loop.
    fn install_extension(
        &mut self,
        idx: usize,
        state: &AppState,
        handle: &tokio::runtime::Handle,
        id: String,
        version: Option<String>,
    ) {
        let client_id = self.clients[idx].id;

        let Some(platform) = detect_platform() else {
            store::append_global_error_log(
                "ext install",
                &format!("no platform for extension {id} (unsupported host os/arch)"),
            );
            self.send_to(
                idx,
                DaemonEvent::ExtensionOpResult {
                    id,
                    ok: false,
                    error: Some("extensions are not available for this platform".to_string()),
                },
            );
            return;
        };

        // The account bearer is the KomaRun OAuth connection's fresh access token. No such
        // connection → the user hasn't signed in to koma.run, so installing is impossible.
        let Some(conn) = state
            .rest
            .config
            .oauth_conns
            .iter()
            .find(|c| c.provider == OAuthProvider::KomaRun)
        else {
            store::append_global_error_log(
                "ext install",
                &format!("no koma.run OAuth connection for extension {id} (platform {platform})"),
            );
            self.send_to(
                idx,
                DaemonEvent::ExtensionOpResult {
                    id,
                    ok: false,
                    error: Some("sign in to koma.run to install".to_string()),
                },
            );
            return;
        };
        let oauth_uuid = conn.uuid.clone();

        let tx = self.store_tx.clone();
        let platform = platform.to_string();
        handle.spawn(async move {
            // A fresh (possibly just-refreshed) koma.run access token. Empty ⇒ the
            // connection is gone / unrecoverable — treat as a sign-in failure.
            let (bearer, _account) = crate::service::oauth::manager::fresh_key(&oauth_uuid, "").await;
            if bearer.trim().is_empty() {
                store::append_global_error_log(
                    "ext install",
                    &format!(
                        "koma.run bearer empty/expired for extension {id} (platform {platform})"
                    ),
                );
                let _ = tx.send(StoreReply::InstallFailed {
                    client_id,
                    id,
                    error: "koma.run session expired — sign in again".to_string(),
                });
                return;
            }
            let reply = match fetch_install_artifact(&id, version.as_deref(), &platform, &bearer).await
            {
                Ok((zip, sha256, signature)) => StoreReply::InstallArtifact {
                    client_id,
                    id,
                    zip,
                    sha256,
                    signature,
                },
                Err(e) => StoreReply::InstallFailed {
                    client_id,
                    id,
                    error: e,
                },
            };
            let _ = tx.send(reply);
        });
    }

    /// Uninstall `id` (synchronous — no network): purge its contributions from the live MCP
    /// manager, stop its child process, remove its on-disk `extensions/<id>/` dir + registry
    /// entry (persisted), and drop its ext-agent registry. Replies with an
    /// [`DaemonEvent::ExtensionOpResult`] + a fresh [`DaemonEvent::InstalledExtensions`].
    fn uninstall_extension(&mut self, idx: usize, state: &mut AppState, id: String) {
        // Clone the manager Arcs up front so the immutable borrow of `state.rest` ends before
        // the `&mut` config / ext_agents mutations below.
        let mcp = state.rest.mcp_manager.clone();
        let ext_mgr = state.rest.ext_manager.clone();

        // Undo the tool registration (a no-op when no MCP manager / no tools), then stop the
        // running child (idempotent; absent extension is a no-op).
        crate::app::ext::register::purge_contributions(&id, mcp.as_ref());
        if let Some(mgr) = &ext_mgr {
            mgr.stop(&id);
        }

        // Remove the unpacked package dir. Guard the id against a path-escape before joining
        // (defense in depth — the id comes from the client): only a well-formed reverse-DNS
        // id is a real installed dir name, and anything else can't match a registry entry.
        if is_safe_ext_id(&id) {
            if let Ok(dir) = store::extensions_dir() {
                let target = dir.join(&id);
                if let Err(e) = std::fs::remove_dir_all(&target) {
                    // A missing dir (already gone) is fine; log anything else.
                    if e.kind() != std::io::ErrorKind::NotFound {
                        store::append_global_error_log(
                            "ext-uninstall",
                            &format!("remove {}: {e}", target.display()),
                        );
                    }
                }
            }
        } else {
            store::append_global_error_log(
                "ext-uninstall",
                &format!("refusing to remove dir for unsafe extension id {id:?}"),
            );
        }

        // Drop the registry entry + persist, then clear its ext-agent containment registry.
        state.rest.config.remove_extension_by_id(&id);
        if let Err(e) = state.rest.config.save() {
            store::append_global_error_log(
                "ext-uninstall",
                &format!("save config after uninstall {id}: {e:#}"),
            );
        }
        state.rest.ext_agents.remove(&id);

        self.send_to(
            idx,
            DaemonEvent::ExtensionOpResult {
                id,
                ok: true,
                error: None,
            },
        );
        self.send_installed_extensions(idx, state);
    }

    /// Reply with the current locally-installed extension registry (read-only).
    fn list_installed_extensions(&mut self, idx: usize, state: &AppState) {
        self.send_installed_extensions(idx, state);
    }

    /// Build + send client `idx` a [`DaemonEvent::InstalledExtensions`] from the live config
    /// registry. Shared by the read handler and the post-install/-uninstall re-push.
    fn send_installed_extensions(&mut self, idx: usize, state: &AppState) {
        let items = state
            .rest
            .config
            .installed_extensions
            .iter()
            .map(|e| {
                let (name, panels) = read_ext_manifest_info(&e.id);
                InstalledExtWire {
                    id: e.id.clone(),
                    name,
                    version: e.version.clone(),
                    tier: e.tier.clone(),
                    kind: e.kind.clone(),
                    enabled: e.enabled,
                    granted: e.granted.clone(),
                    panels,
                }
            })
            .collect();
        self.send_to(idx, DaemonEvent::InstalledExtensions { items });
    }

    /// Drain any landed [`StoreReply`]s and reply to each requesting client. Called once per
    /// tick by the daemon loop (right after `drain_oauth_pushes`), the exact
    /// `drain_list_models`/`drain_list_routes` mirror: a background reqwest task can't advance
    /// the per-client seq, so the queued reply is turned into a `send_to` frame here. Browse/
    /// detail replies map straight through; an `InstallArtifact` runs the on-loop
    /// verify+install step ([`Self::finish_install`]). A reply whose client has since vanished
    /// is silently dropped; the hub owns a `store_tx` clone, so `Disconnected` never fires.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_store_replies(
        &mut self,
        state: &mut AppState,
    ) {
        while let Ok(reply) = self.store_rx.try_recv() {
            match reply {
                StoreReply::Catalogue {
                    client_id,
                    items,
                    error,
                } => {
                    if let Some(i) = self.clients.iter().position(|c| c.id == client_id) {
                        self.send_to(i, DaemonEvent::StoreCatalogue { items, error });
                    }
                }
                StoreReply::Detail {
                    client_id,
                    detail,
                    error,
                } => {
                    if let Some(i) = self.clients.iter().position(|c| c.id == client_id) {
                        self.send_to(i, DaemonEvent::StoreItemDetail { detail, error });
                    }
                }
                StoreReply::InstallArtifact {
                    client_id,
                    id,
                    zip,
                    sha256,
                    signature,
                } => {
                    if let Some(i) = self.clients.iter().position(|c| c.id == client_id) {
                        self.finish_install(i, state, id, zip, sha256, signature);
                    }
                }
                StoreReply::InstallFailed {
                    client_id,
                    id,
                    error,
                } => {
                    if let Some(i) = self.clients.iter().position(|c| c.id == client_id) {
                        self.send_to(
                            i,
                            DaemonEvent::ExtensionOpResult {
                                id,
                                ok: false,
                                error: Some(error),
                            },
                        );
                    }
                }
                // W8 panel bridge: turn a panel.msg outcome into a seq'd `ExtPanelReply` to the
                // REQUESTING client (matched by id — a client that vanished mid-flight is silently
                // dropped, same safe `position` map as the store replies above).
                StoreReply::PanelReply {
                    client_id,
                    ext_id,
                    panel_id,
                    req_id,
                    ok,
                    payload,
                    error,
                } => {
                    if let Some(i) = self.clients.iter().position(|c| c.id == client_id) {
                        self.send_to(
                            i,
                            DaemonEvent::ExtPanelReply {
                                ext_id,
                                panel_id,
                                req_id,
                                ok,
                                payload,
                                error,
                            },
                        );
                    }
                }
            }
        }
    }

    /// Broadcast every queued daemon→panel push (`state.rest.ext_panel_pushes`, filled by the
    /// `drains::drain_ext_notifies` global drain when an extension sends `panel.push`) to ALL
    /// ATTACHED clients as a seq'd [`DaemonEvent::ExtPanelPush`]. Called once per tick by the
    /// daemon loop right after [`Self::drain_store_replies`]. Unlike `drain_store_replies` /
    /// `drain_oauth_pushes` — which target ONE initiating client by id — a panel push is NOT
    /// request-correlated (no initiator), so it fans out to EVERY attached client (an enrolled-
    /// but-not-yet-attached client is skipped, exactly like every other delta — it has no live
    /// shadow yet). Empty except in the ticks a push lands. `send_to` never removes a client (a
    /// dead channel just skips the seq bump), so iterating by index is sound.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_ext_panel_pushes(
        &mut self,
        state: &mut AppState,
    ) {
        if state.rest.ext_panel_pushes.is_empty() {
            return;
        }
        let pushes = std::mem::take(&mut state.rest.ext_panel_pushes);
        for (ext_id, panel_id, payload) in pushes {
            for i in 0..self.clients.len() {
                if self.clients[i].attached {
                    // Clone per recipient — one push fans out to every attached shadow.
                    self.send_to(
                        i,
                        DaemonEvent::ExtPanelPush {
                            ext_id: ext_id.clone(),
                            panel_id: panel_id.clone(),
                            payload: payload.clone(),
                        },
                    );
                }
            }
        }
    }

    /// The on-loop tail of the install: verify + unpack the downloaded zip (fail-closed),
    /// upsert the registry entry + persist, register its contributions, spawn it if
    /// daemon-kind, then reply with [`DaemonEvent::ExtensionOpResult`] + a fresh
    /// [`DaemonEvent::InstalledExtensions`]. A signature-verification or integrity failure is
    /// a hard stop surfaced as `ok:false`. When the artifact is UNSIGNED (koma.run signing
    /// infra may not be live yet), a DEBUG build falls back to `install_dev_unsigned` so the
    /// end-to-end flow is testable now (loudly logged); a release build rejects it.
    fn finish_install(
        &mut self,
        idx: usize,
        state: &mut AppState,
        id: String,
        zip: Vec<u8>,
        sha256: String,
        signature: Option<String>,
    ) {
        // Clone the manager Arcs so the later `&mut` config mutations don't overlap a
        // `state.rest` borrow.
        let mcp = state.rest.mcp_manager.clone();
        let ext_mgr = state.rest.ext_manager.clone();

        let installed: Result<crate::model::app_config::InstalledExtension> =
            match (&signature, sha256.trim().is_empty()) {
                // Signed + integrity present → the production fail-closed path.
                (Some(sig), false) => {
                    crate::app::ext::install::install_from_zip(&zip, &sha256, sig)
                }
                // No signature (or no advertised digest): koma.run signing not live yet.
                _ => install_unsigned_fallback(&id, &zip),
            };

        match installed {
            Ok(ext) => {
                state.rest.config.upsert_extension(ext.clone());
                if let Err(e) = state.rest.config.save() {
                    store::append_global_error_log(
                        "ext-install",
                        &format!("save config after install {}: {e:#}", ext.id),
                    );
                }
                // Register contributions (tools → live MCP snapshot) + auto-start a
                // daemon-kind child. Both best-effort: a failure is logged, not fatal —
                // the extension is installed on disk + in the registry regardless.
                if let Some(mgr) = &ext_mgr {
                    if let Err(e) =
                        crate::app::ext::register::register_contributions(&ext, mcp.as_ref(), mgr)
                    {
                        store::append_global_error_log(
                            "ext-install",
                            &format!("register contributions for {}: {e:#}", ext.id),
                        );
                    }
                    if ext.kind == "daemon" {
                        if let Err(e) = mgr.ensure_started(&ext) {
                            store::append_global_error_log(
                                "ext-install",
                                &format!("start extension {}: {e:#}", ext.id),
                            );
                        }
                    }
                }
                self.send_to(
                    idx,
                    DaemonEvent::ExtensionOpResult {
                        id: ext.id.clone(),
                        ok: true,
                        error: None,
                    },
                );
                self.send_installed_extensions(idx, state);
            }
            Err(e) => {
                store::append_global_error_log(
                    "ext install",
                    &format!("verify/unpack failed for extension {id}: {e:#}"),
                );
                self.send_to(
                    idx,
                    DaemonEvent::ExtensionOpResult {
                        id,
                        ok: false,
                        error: Some(format!("{e:#}")),
                    },
                );
            }
        }
    }
}

/// The auto-start decision for a `panel.msg` (W8 panel bridge), factored out as a PURE function
/// over the two inputs the on-`spawn_blocking` closure branches on so it is unit-testable without
/// a live manager. `running` is `mgr.is_running(&ext_id)`; `record` is the extension's registry
/// entry (`None` = not installed). Returns:
///   - `Ok(true)`  → already running → invoke straight away (an already-live extension is used
///                    regardless of its persisted `enabled` flag, which only gates auto-start).
///   - `Ok(false)` → a daemon-kind, ENABLED, not-yet-running extension → `ensure_started` first
///                    (a panel being open implies user intent; a blocking start is fine on the
///                    pool).
///   - `Err(msg)`  → not serviceable: MISSING / DISABLED / a ONESHOT-kind extension (spawned
///                    per-invoke, so it has no live panel backend) → surfaced as the reply error.
fn panel_start_decision(running: bool, record: Option<&InstalledExtension>) -> Result<bool, String> {
    if running {
        return Ok(true);
    }
    match record {
        // Disabled (any kind) → not available (its auto-start is intentionally off).
        Some(ext) if !ext.enabled => Err("extension not available".to_string()),
        // Enabled daemon, not yet running → auto-start it.
        Some(ext) if ext.kind == "daemon" => Ok(false),
        // Enabled but not a daemon (oneshot) → no persistent panel backend to talk to.
        Some(_) => Err("extension not available".to_string()),
        // Not installed.
        None => Err("extension not available".to_string()),
    }
}

/// The OFF-LOOP body of [`DaemonHub::panel_msg`]: apply the [`panel_start_decision`], start the
/// extension if the decision says so, then `invoke_with_timeout("panel.msg")` and package the
/// outcome as a [`StoreReply::PanelReply`]. Runs on `spawn_blocking` (both `ensure_started` and
/// `invoke_with_timeout` block on a sync→async bridge). Every failure — unavailable extension,
/// failed auto-start, or a timed-out/errored invoke — becomes an `ok:false` reply carrying a
/// human-readable `error`, logged via `append_global_error_log` (never `eprintln!`).
fn run_panel_msg(
    mgr: &Arc<ExtHostManager>,
    client_id: u64,
    ext_id: String,
    panel_id: String,
    req_id: Option<String>,
    payload: serde_json::Value,
    record: &Option<InstalledExtension>,
) -> StoreReply {
    // Decide + (maybe) start. `error` stays `None` on the happy path (running / just-started).
    let error: Option<String> = match panel_start_decision(mgr.is_running(&ext_id), record.as_ref())
    {
        Ok(true) => None,
        Ok(false) => match record.as_ref() {
            // `Ok(false)` is only returned for a `Some(daemon, enabled)` record, so this is set.
            Some(ext) => match mgr.ensure_started(ext) {
                Ok(()) => None,
                Err(e) => {
                    store::append_global_error_log(
                        "ext panel",
                        &format!("auto-start {ext_id} for panel.msg failed: {e:#}"),
                    );
                    Some(format!("extension failed to start: {e:#}"))
                }
            },
            None => Some("extension not available".to_string()),
        },
        Err(msg) => Some(msg),
    };

    if let Some(error) = error {
        return StoreReply::PanelReply {
            client_id,
            ext_id,
            panel_id,
            req_id,
            ok: false,
            payload: None,
            error: Some(error),
        };
    }

    // Running (or just-started) → invoke `panel.msg` with the SDK's `{ panelId, payload }` shape.
    // `panel_id` is cloned into the params (it is also returned in the reply); `ext_id` is
    // borrowed for the invoke, then moved into the reply.
    match mgr.invoke_with_timeout(
        &ext_id,
        "panel.msg",
        serde_json::json!({ "panelId": panel_id.clone(), "payload": payload }),
        Duration::from_secs(10),
    ) {
        Ok(value) => StoreReply::PanelReply {
            client_id,
            ext_id,
            panel_id,
            req_id,
            ok: true,
            payload: Some(value),
            error: None,
        },
        Err(e) => {
            store::append_global_error_log(
                "ext panel",
                &format!("panel.msg invoke for {ext_id} failed: {e:#}"),
            );
            StoreReply::PanelReply {
                client_id,
                ext_id,
                panel_id,
                req_id,
                ok: false,
                payload: None,
                error: Some(format!("{e:#}")),
            }
        }
    }
}

/// The DEBUG-only unsigned install fallback: write the zip to a temp file and install it via
/// [`crate::app::ext::install::install_dev_unsigned`] (which skips signature verification),
/// so the end-to-end store→install flow is testable before koma.run's signing infra is live.
/// LOUDLY logged. A release build has no such path — an unsigned artifact is rejected.
#[cfg(debug_assertions)]
fn install_unsigned_fallback(id: &str, zip: &[u8]) -> Result<crate::model::app_config::InstalledExtension> {
    store::append_global_error_log(
        "ext-install",
        &format!("UNSIGNED dev install of {id} (koma.run sent no signature — debug build only)"),
    );
    let tmp = std::env::temp_dir().join(format!("koma-ext-dl-{}.zip", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, zip)
        .map_err(|e| anyhow::anyhow!("write temp zip {}: {e}", tmp.display()))?;
    let r = crate::app::ext::install::install_dev_unsigned(&tmp);
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Release builds reject an unsigned artifact — the signature gate can never be bypassed in
/// production (see `install::install_dev_unsigned`'s `cfg(debug_assertions)`).
#[cfg(not(debug_assertions))]
fn install_unsigned_fallback(_id: &str, _zip: &[u8]) -> Result<crate::model::app_config::InstalledExtension> {
    anyhow::bail!("extension artifact is unsigned; refusing to install")
}

/// Detect this build's store platform token (`<os>-<arch>`), or `None` for a platform the
/// v0 store doesn't ship (e.g. windows-arm64). Uses `cfg!`-gated returns so it resolves at
/// compile time to the host triple. The v0 set is
/// `linux-x64` / `linux-arm64` / `darwin-x64` / `darwin-arm64` / `windows-x64`.
fn detect_platform() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("darwin-x64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("darwin-arm64");
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x64");
    }
    #[allow(unreachable_code)]
    {
        None
    }
}

/// Whether `id` is a well-formed reverse-DNS extension id safe to use as a directory name
/// under `extensions/` — the SAME whitelist `install::validate_id` enforces (non-empty,
/// only `[A-Za-z0-9._-]`, at least one alphanumeric, not `.`-wrapped). Belt-and-suspenders
/// on the uninstall path, whose `id` comes from the client.
fn is_safe_ext_id(id: &str) -> bool {
    let all_allowed = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    let has_alnum = id.chars().any(|c| c.is_ascii_alphanumeric());
    let dot_wrapped = id.starts_with('.') || id.ends_with('.');
    all_allowed && has_alnum && !dot_wrapped
}

/// Read the manifest for extension `id` and extract both the friendly name and
/// the panel list. A missing/unreadable/unparsable manifest degrades to using
/// the id as the name and an empty panel list (non-fatal for list rendering).
fn read_ext_manifest_info(id: &str) -> (String, Vec<PanelWire>) {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return (id.to_string(), Vec::new()),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (id.to_string(), Vec::new()),
    };
    let manifest: koma_extension::protocol::ExtensionManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            return (id.to_string(), Vec::new());
        }
    };
    let name = if manifest.name.is_empty() {
        id.to_string()
    } else {
        manifest.name
    };
    let panels = manifest
        .contributes
        .panels
        .into_iter()
        .map(|p| PanelWire {
            id: p.id,
            title: p.title,
            icon: p.icon,
        })
        .collect();
    (name, panels)
}

/// Read `contributes.panels` straight off `extensions_dir()/<id>/manifest.json` — the
/// registry (`InstalledExtension`) doesn't carry contributions, so this is a fresh,
/// best-effort re-read on every installed-list build: a missing/unreadable/unparsable
/// manifest degrades to an empty panel list (never fails the whole installed-list
/// projection over one bad entry), logged via `store::append_global_error_log` so a
/// parse failure is still visible. SAME logic as the GUI host's
/// `store_host::read_ext_panels` copy, mirroring this module's existing
/// map_summary/map_detail/map_contributes duplication (that module is left untouched;
/// see the file doc comment).
fn read_ext_panels(id: &str) -> Vec<PanelWire> {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return Vec::new(),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(), // not installed / unreadable — no panels
    };
    let manifest: koma_extension::protocol::ExtensionManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            return Vec::new();
        }
    };
    manifest
        .contributes
        .panels
        .into_iter()
        .map(|p| PanelWire {
            id: p.id,
            title: p.title,
            icon: p.icon,
        })
        .collect()
}

/// A shared reqwest client for the store fetches (default redirect policy — follows the
/// signed-URI redirect on the direct-stream fallback and any CDN hop for browse/detail).
fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// `GET /extensions[?q&category]` → the mapped catalogue rows. PUBLIC (no auth). A non-2xx
/// status or a parse error is an `Err(String)` the caller surfaces as the catalogue's error.
async fn fetch_catalogue(
    query: Option<String>,
    category: Option<String>,
) -> std::result::Result<Vec<StoreItemWire>, String> {
    // Build the URL with proper query-param encoding via reqwest::Url.
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(q) = query {
        let q = q.trim().to_string();
        if !q.is_empty() {
            pairs.push(("q", q));
        }
    }
    if let Some(c) = category {
        let c = c.trim().to_string();
        if !c.is_empty() {
            pairs.push(("category", c));
        }
    }
    let url = reqwest::Url::parse_with_params(STORE_API_BASE, &pairs)
        .map_err(|e| format!("bad store url: {e}"))?;

    let resp = http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("store request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("store returned HTTP {}", resp.status().as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("store response parse failed: {e}"))?;
    let items = body
        .get("items")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().map(map_summary).collect())
        .unwrap_or_default();
    Ok(items)
}

/// `GET /extensions/{id}` → the mapped detail. PUBLIC (no auth).
async fn fetch_detail(id: &str) -> std::result::Result<StoreDetailWire, String> {
    let url = format!("{STORE_API_BASE}/{id}");
    let resp = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("store request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("store returned HTTP {}", resp.status().as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("store response parse failed: {e}"))?;
    Ok(map_detail(&body))
}

/// `GET /extensions/{id}/download?version&platform` with the account Bearer, resolving the
/// artifact per the store contract's TWO shapes:
///
/// * **302 redirect** (preferred): the response carries a `Location` (the short-lived signed
///   URI) plus a JSON body echoing `{ sha256, signature }`; we read the integrity from the
///   body, then GET the signed URI for the `.zip` bytes.
/// * **direct stream** (v0 fallback): a `200` whose body IS the `.zip`, with integrity in the
///   `X-Koma-Sha256` / `X-Koma-Signature` headers.
///
/// Redirects are DISABLED on the first hop so we can read the 302 body + `Location` ourselves
/// (an auto-follow would swallow the integrity body). Returns `(zip_bytes, sha256,
/// signature)`; `signature` is `None` when the server advertised none (→ the caller's dev
/// unsigned fallback). A 401/402/404/… maps to a friendly error string.
async fn fetch_install_artifact(
    id: &str,
    version: Option<&str>,
    platform: &str,
    bearer: &str,
) -> std::result::Result<(Vec<u8>, String, Option<String>), String> {
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;

    let mut pairs: Vec<(&str, &str)> = vec![("platform", platform)];
    if let Some(v) = version {
        if !v.is_empty() {
            pairs.push(("version", v));
        }
    }
    let url = reqwest::Url::parse_with_params(&format!("{STORE_API_BASE}/{id}/download"), &pairs)
        .map_err(|e| format!("bad download url: {e}"))?;

    let resp = no_redirect
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| {
            let msg = format!("download request failed: {e}");
            store::append_global_error_log(
                "ext download",
                &format!("{id} (platform {platform}): {msg}"),
            );
            msg
        })?;
    let status = resp.status();

    if status.is_redirection() {
        // 302: Location → signed URI; body echoes the integrity fields.
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                let msg = "download redirect missing Location header".to_string();
                store::append_global_error_log(
                    "ext download",
                    &format!("{id} (platform {platform}): {msg}"),
                );
                msg
            })?;
        let body = resp.text().await.unwrap_or_default();
        let (sha256, signature) = parse_integrity_json(&body);

        // The signed URI is public (auth is in the query signature) — a plain follow.
        let zresp = http_client()
            .get(&location)
            .send()
            .await
            .map_err(|e| format!("signed download failed: {e}"))?;
        if !zresp.status().is_success() {
            let signed_status = zresp.status().as_u16();
            store::append_global_error_log(
                "ext download",
                &format!(
                    "{id} (platform {platform}): signed download returned HTTP {signed_status}"
                ),
            );
            return Err(format!("signed download returned HTTP {signed_status}"));
        }
        let bytes = zresp
            .bytes()
            .await
            .map_err(|e| format!("reading artifact failed: {e}"))?
            .to_vec();
        Ok((bytes, sha256, signature))
    } else if status.is_success() {
        // Direct stream: integrity in headers, body IS the zip.
        let sha256 = header_str(&resp, "x-koma-sha256").unwrap_or_default();
        let signature = header_str(&resp, "x-koma-signature").filter(|s| !s.is_empty());
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("reading artifact failed: {e}"))?
            .to_vec();
        Ok((bytes, sha256, signature))
    } else {
        let code = status.as_u16();
        let msg = match code {
            401 => "koma.run rejected the session — sign in again".to_string(),
            402 => "this extension needs an active koma.run entitlement".to_string(),
            404 => "extension not found for this version/platform".to_string(),
            429 => "koma.run is rate limiting — try again shortly".to_string(),
            other => format!("download failed (HTTP {other})"),
        };
        store::append_global_error_log(
            "ext download",
            &format!("{id} (platform {platform}): HTTP {code}: {msg}"),
        );
        Err(msg)
    }
}

/// Read a response header as a `String`, or `None` if absent / non-ASCII.
fn header_str(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Pull `{ sha256, signature }` out of a 302 integrity body (best-effort). A malformed /
/// empty body yields `(String::new(), None)` — the caller then treats it as unsigned.
fn parse_integrity_json(body: &str) -> (String, Option<String>) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (String::new(), None),
    };
    let sha = str_field(&v, "sha256");
    let sig = v
        .get("signature")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    (sha, sig)
}

/// Map one store `ExtensionSummary` JSON object to [`StoreItemWire`] (defensive — a missing
/// field degrades to empty rather than failing the whole list parse).
fn map_summary(v: &serde_json::Value) -> StoreItemWire {
    StoreItemWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
    }
}

/// Map one store `ExtensionDetail` JSON object to [`StoreDetailWire`] (defensive, like
/// [`map_summary`]).
fn map_detail(v: &serde_json::Value) -> StoreDetailWire {
    StoreDetailWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
        description_md: str_field(v, "description_md"),
        screenshots: arr_str(v, "screenshots"),
        contributes: map_contributes(v.get("contributes")),
        requires: arr_str(v, "requires"),
        versions: arr_str(v, "versions"),
    }
}

/// Collapse the detail's `contributes` object to per-kind COUNTS. Accepts both the array
/// shape (`{ models: [..], tools: [..] }` → counts) — a missing kind is 0.
fn map_contributes(v: Option<&serde_json::Value>) -> StoreContributesWire {
    let count = |key: &str| -> u32 {
        v.and_then(|c| c.get(key))
            .and_then(|x| x.as_array())
            .map(|a| a.len() as u32)
            .unwrap_or(0)
    };
    StoreContributesWire {
        models: count("models"),
        panels: count("panels"),
        tools: count("tools"),
        sub_agents: count("sub_agents"),
    }
}

/// A string field of a JSON object, or `""` if absent / not a string.
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// A `Vec<String>` field of a JSON object (its string elements), or empty.
fn arr_str(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The build host is always one of the v0 platforms (the test binary itself is one of
    /// them), so `detect_platform` must resolve to a `Some` in the advertised set.
    #[test]
    fn detect_platform_is_a_known_v0_token() {
        let plat = detect_platform().expect("build host must be a v0 store platform");
        assert!(
            [
                "linux-x64",
                "linux-arm64",
                "darwin-x64",
                "darwin-arm64",
                "windows-x64"
            ]
            .contains(&plat),
            "unexpected platform token: {plat}"
        );
    }

    /// The id-safety guard mirrors `install::validate_id`: reverse-DNS ids pass; path-escape
    /// / pure-punctuation ids are rejected (so the uninstall `remove_dir_all` can never
    /// escape `extensions/`).
    #[test]
    fn safe_ext_id_rejects_path_escapes() {
        assert!(is_safe_ext_id("run.koma.gateway"));
        assert!(is_safe_ext_id("run.koma.example.echo-tool_daemon"));
        assert!(!is_safe_ext_id(""));
        assert!(!is_safe_ext_id("."));
        assert!(!is_safe_ext_id(".."));
        assert!(!is_safe_ext_id("../etc"));
        assert!(!is_safe_ext_id("a/b"));
        assert!(!is_safe_ext_id(".hidden"));
    }

    /// A missing/never-installed manifest degrades to an empty panel list rather than
    /// failing — the id here is guaranteed to have no `extensions/<id>/manifest.json` on
    /// any test machine.
    #[test]
    fn read_ext_panels_degrades_to_empty_on_missing_manifest() {
        assert_eq!(
            read_ext_panels("run.koma.definitely-not-installed.test-fixture"),
            Vec::<PanelWire>::new()
        );
    }

    /// The summary mapping pulls exactly the wire fields from an `ExtensionSummary`-shaped
    /// object, degrading a missing field to empty rather than failing.
    #[test]
    fn map_summary_projects_summary_fields() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "Premium koma models, one endpoint.",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "https://cdn.koma.run/ext/run.koma.gateway/icon.png",
            "categories": ["models", "gateway"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z"
        });
        let item = map_summary(&v);
        assert_eq!(item.id, "run.koma.gateway");
        assert_eq!(item.name, "koma Gateway");
        assert_eq!(item.tier, "paid");
        assert_eq!(item.kind, "daemon");
        assert_eq!(item.latest_version, "0.3.1");
        assert_eq!(item.categories, vec!["models", "gateway"]);
        assert_eq!(item.author, "koma");
    }

    /// The detail mapping projects the long-form fields AND collapses `contributes` to
    /// per-kind counts + carries the `requires` grant list (the install card's inputs).
    #[test]
    fn map_detail_counts_contributions_and_reads_requires() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "one endpoint",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "",
            "categories": ["models"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z",
            "description_md": "# koma Gateway\n\nlong",
            "screenshots": ["https://cdn.koma.run/ext/run.koma.gateway/1.png"],
            "contributes": {
                "models": [{ "id": "a" }, { "id": "b" }],
                "panels": [],
                "tools": [{ "name": "t" }],
                "sub_agents": []
            },
            "requires": ["agents:read"],
            "versions": ["0.3.1", "0.3.0"]
        });
        let d = map_detail(&v);
        assert_eq!(d.description_md, "# koma Gateway\n\nlong");
        assert_eq!(d.screenshots.len(), 1);
        assert_eq!(d.contributes.models, 2);
        assert_eq!(d.contributes.panels, 0);
        assert_eq!(d.contributes.tools, 1);
        assert_eq!(d.contributes.sub_agents, 0);
        assert_eq!(d.requires, vec!["agents:read"]);
        assert_eq!(d.versions, vec!["0.3.1", "0.3.0"]);
    }

    /// A 302 integrity body yields `(sha, Some(sig))`; an empty / malformed body yields the
    /// unsigned shape `(empty, None)` — the caller's dev-unsigned trigger.
    #[test]
    fn parse_integrity_json_reads_or_degrades() {
        let (sha, sig) =
            parse_integrity_json(r#"{"sha256":"3b1f","signature":"MEUCIQ==","size":123}"#);
        assert_eq!(sha, "3b1f");
        assert_eq!(sig.as_deref(), Some("MEUCIQ=="));

        let (sha2, sig2) = parse_integrity_json("");
        assert!(sha2.is_empty());
        assert!(sig2.is_none());

        // Present-but-empty signature is treated as unsigned.
        let (_sha3, sig3) = parse_integrity_json(r#"{"sha256":"aa","signature":""}"#);
        assert!(sig3.is_none());
    }

    /// A registry fixture for the panel-start decision (only `enabled` + `kind` matter to it).
    fn ext_record(kind: &str, enabled: bool) -> InstalledExtension {
        InstalledExtension {
            id: "run.koma.test".to_string(),
            version: "0.0.1".to_string(),
            tier: "free".to_string(),
            granted: Vec::new(),
            enabled,
            kind: kind.to_string(),
            exec: "bin/x".to_string(),
        }
    }

    /// The panel-start decision (W8 auto-start) over every input combination: an already-running
    /// extension is invoked straight away; a not-running daemon+enabled one is started; a
    /// oneshot / disabled / missing one is a clean "extension not available" error.
    #[test]
    fn panel_start_decision_covers_all_cases() {
        // Already running → invoke straight away, regardless of the record (or its absence).
        assert_eq!(panel_start_decision(true, None), Ok(true));
        assert_eq!(
            panel_start_decision(true, Some(&ext_record("daemon", true))),
            Ok(true)
        );
        // Disabled but somehow running → still Ok(true): the enabled flag only gates auto-start.
        assert_eq!(
            panel_start_decision(true, Some(&ext_record("daemon", false))),
            Ok(true)
        );

        // Not running, daemon + enabled → auto-start (Ok(false)).
        assert_eq!(
            panel_start_decision(false, Some(&ext_record("daemon", true))),
            Ok(false)
        );
        // Not running, oneshot-kind → error (no persistent panel backend).
        assert_eq!(
            panel_start_decision(false, Some(&ext_record("oneshot", true))),
            Err("extension not available".to_string())
        );
        // Not running, disabled daemon → error (auto-start intentionally off).
        assert_eq!(
            panel_start_decision(false, Some(&ext_record("daemon", false))),
            Err("extension not available".to_string())
        );
        // Not running, not installed → error.
        assert_eq!(
            panel_start_decision(false, None),
            Err("extension not available".to_string())
        );
    }
}

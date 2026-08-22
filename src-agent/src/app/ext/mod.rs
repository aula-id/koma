//! Extension host CORE.
//!
//! koma installs, verifies, spawns, and talks to a signed extension. This module is
//! the Rust-side host manager, keyed PER-EXTENSION (unlike the single security daemon
//! it is cloned from — [`crate::app::sec`]). For each running daemon-kind extension it
//! owns a child process and a DUPLEX unix-socket connection: koma `Invoke`s the
//! extension (the "contributes" side) and the extension `Call`s back into koma (the
//! "requires" side) over the same link.
//!
//! ## Transport (host ⇄ extension must agree exactly)
//!
//! koma binds a per-extension unix socket at `~/.koma/run/ext-<id>.sock`, then spawns
//! the extension child with env `KOMA_EXT_SOCKET=<that path>` and
//! `KOMA_EXT_TOKEN=<per-boot uuid>`. The child CONNECTS in and sends
//! `ExtMsg::Hello { protocol, token, manifest }`; koma validates `token` matches and
//! `protocol == PROTOCOL_VERSION`, then replies `KomaMsg::Welcome { protocol,
//! koma_version, granted }` (grants = the manifest `requires` echoed back — enforcement
//! is a later wave) or `KomaMsg::Reject`. After that, newline-delimited JSON `ExtMsg`/
//! `KomaMsg` frames flow BOTH ways on that one connection.
//!
//! ## Concurrency model (KEPT verbatim from the security-daemon template)
//!
//! Per running extension: a **writer task** owns the socket write half and receives
//! frames over an `mpsc::UnboundedSender<String>`; a **reader task** owns the read half,
//! routes `Result` frames to the matching `oneshot` in a shared `pending` map, answers
//! ext→koma `Call`s, and notes `Health`. A `generation: u64` counter (bumped by every
//! start/stop and re-checked under the lock after the slow spawn) makes a stopped
//! extension un-resurrectable by a slow start. The child is spawned `kill_on_drop(true)`.
//! The sync→async dispatch bridge blocks the calling thread on a
//! `std::sync::mpsc::recv_timeout` — NEVER `Handle::block_on` (which panics inside a
//! runtime).
//!
//! NOTE: a building block — the tool-system / model-provider / panel wiring that
//! consumes `invoke` lands in follow-up waves — so the module carries
//! `#![allow(dead_code)]` to stay clippy-clean until then, matching the sibling `sec`
//! module.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Result};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use koma_extension::protocol::{Grant, KomaMsg};

use crate::model::app_config::InstalledExtension;
use crate::model::store;

pub mod broker;
pub mod dev_cli;
pub mod events;
pub mod install;
pub mod register;
pub(crate) mod screen;
// Named `ext_store` (not `store`) so this module doesn't collide with the pervasive
// `use crate::model::store;` (the app-data-dir module) every `app::ext` file — this one
// included — imports under the plain name `store`.
pub(crate) mod ext_store;
pub(crate) mod store_api;
pub mod uninstall;
mod wire;
pub use broker::{ExtAgentRegistry, ExtCallRequest, ExtNotify};
pub use dev_cli::{print_ext_usage, run_install_dev as run_ext_install_dev};
use wire::{connect_and_handshake, reader_task, writer_task, Handshaked};

/// A reply to a koma→ext `Invoke`: the extension's `result` value, or an error string
/// (connection dropped / stopped). Carried through the `pending` map.
type Reply = Result<serde_json::Value, String>;

/// In-flight `Invoke`s awaiting a reply, keyed by request id. Shared between the
/// dispatch path (inserts) and the reader task (removes + fulfils).
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>;

/// How long bind + spawn + accept + the `Hello`/`Welcome` handshake may take before
/// [`ExtHostManager::ensure_started`] gives up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one koma→ext `Invoke` round-trip may take before [`ExtHostManager::invoke`]
/// gives up and returns an error.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Per-extension connection state, guarded by the manager's map mutex. Written by the
/// start/stop paths and the dispatch path; the mutex is only ever held for cheap
/// synchronous work — never across an `.await`.
#[derive(Default)]
struct ExtEntry {
    /// `true` once the handshake succeeded and the reader/writer tasks are live.
    running: bool,
    /// Monotonic generation, bumped by every start/stop for this extension. A start
    /// task captures it BEFORE the spawn+handshake await and re-checks it AFTER (under
    /// the lock, before storing): a mismatch means a stop/restart superseded this
    /// attempt, so its child is killed and nothing is stored.
    generation: u64,
    /// Sender into the writer task that owns the socket write half. `Some` only while a
    /// child is live; dropping it (on stop) closes the writer task and the connection.
    writer: Option<mpsc::UnboundedSender<String>>,
    /// In-flight `Invoke`s awaiting a reply. Shared with the reader task via the `Arc`.
    pending: PendingMap,
    /// Monotonic source of request ids handed out by [`ExtHostManager::invoke`].
    next_id: u64,
    /// Handle to the live child, kept so `stop` can `start_kill` it (and `kill_on_drop`
    /// reaps it if the manager is dropped). `None` when no child is running.
    child: Option<tokio::process::Child>,
    /// Last `Health{ok}` reported by the extension (advisory liveness hint).
    last_health_ok: bool,
    /// The scopes koma granted this extension (parsed from `InstalledExtension.granted`
    /// at [`ExtHostManager::ensure_started_at`]). Read by the reader task via
    /// [`ExtHostManager::granted_for`] when packaging an `agents.*` `Call` into an
    /// [`ExtCallRequest`], so the grant broker gates against exactly what was extended
    /// to THIS extension.
    granted: Vec<Grant>,
    /// The koma->ext event names this extension declared under `contributes.events`
    /// (parsed from its on-disk `manifest.json` at [`ExtHostManager::ensure_started_at`],
    /// lowercased + deduped). Read by [`ExtHostManager::subscribers`] to decide which
    /// running extensions should receive a given `notify`.
    events: Vec<String>,
}

/// The extension host manager. Holds the runtime [`Handle`] (so async socket work can be
/// spawned from synchronous code) and a mutex-guarded map of per-extension
/// [`ExtEntry`]s keyed by extension id. Constructed inert via [`Self::new`]; a child is
/// only spawned by [`Self::ensure_started`].
pub struct ExtHostManager {
    handle: Handle,
    inner: Mutex<HashMap<String, ExtEntry>>,
    /// Sender into the event loop's `ext_call_rx` drain, used by every reader task
    /// to hand an `agents.*` `Call` off to the grant broker (the reader task has no
    /// `AppState` access). Set once at startup via [`Self::set_ext_call_tx`]; `None`
    /// until then (and in unit tests that never drive `agents.*` calls), in which
    /// case the reader answers such a call with a "broker not initialized" error
    /// rather than hanging the extension.
    ext_call_tx: Mutex<Option<mpsc::UnboundedSender<ExtCallRequest>>>,
    /// Sender into the event loop's `ext_notify_rx` drain, used by every reader
    /// task to hand an ext->koma `Notify` off to the event loop (the reader task
    /// has no `AppState` access, same reason as `ext_call_tx`). Set once at
    /// startup via [`Self::set_ext_notify_tx`]; `None` until then (and in unit
    /// tests that never drive a `Notify`), in which case the reader silently
    /// drops the frame — `Notify` is fire-and-forget, so there is nothing to
    /// fail back to the extension either way.
    ext_notify_tx: Mutex<Option<mpsc::UnboundedSender<ExtNotify>>>,
}

impl ExtHostManager {
    /// Build an inert manager: no extensions running, nothing spawned.
    pub fn new(handle: &Handle) -> Arc<Self> {
        Arc::new(Self {
            handle: handle.clone(),
            inner: Mutex::new(HashMap::new()),
            ext_call_tx: Mutex::new(None),
            ext_notify_tx: Mutex::new(None),
        })
    }

    /// Wire the event-loop grant-broker channel into the manager (called once at
    /// startup with a clone of `AppStateRest::ext_call_tx`). Every reader task reads
    /// it via [`Self::ext_call_tx`] to forward an `agents.*` `Call` to the broker.
    pub fn set_ext_call_tx(&self, tx: mpsc::UnboundedSender<ExtCallRequest>) {
        *self.ext_call_tx.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    }

    /// A clone of the grant-broker sender, or `None` if it was never wired
    /// ([`Self::set_ext_call_tx`] not yet called). Consulted by the reader task per
    /// `agents.*` `Call`.
    pub(crate) fn ext_call_tx(&self) -> Option<mpsc::UnboundedSender<ExtCallRequest>> {
        self.ext_call_tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Wire the event-loop notify channel into the manager (called once at
    /// startup with a clone of `AppStateRest::ext_notify_tx`). Every reader task
    /// reads it via [`Self::ext_notify_tx`] to forward an ext->koma `Notify` to
    /// the event loop.
    pub fn set_ext_notify_tx(&self, tx: mpsc::UnboundedSender<ExtNotify>) {
        *self.ext_notify_tx.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    }

    /// A clone of the notify sender, or `None` if it was never wired
    /// ([`Self::set_ext_notify_tx`] not yet called). Consulted by the reader task
    /// per ext->koma `Notify`.
    pub(crate) fn ext_notify_tx(&self) -> Option<mpsc::UnboundedSender<ExtNotify>> {
        self.ext_notify_tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The scopes granted to the running extension `ext_id` (empty if unknown/not
    /// running). Read by the reader task when packaging an `agents.*` `Call`.
    pub(crate) fn granted_for(&self, ext_id: &str) -> Vec<Grant> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(ext_id)
            .map(|e| e.granted.clone())
            .unwrap_or_default()
    }

    /// The ids of every RUNNING extension whose `contributes.events` (lowercased at
    /// start, see [`read_events_best_effort`]) contains `event` (also lowercased
    /// here, so callers need not normalise it themselves). Used by a future fan-out
    /// wave to decide who should receive a given koma->ext [`Self::notify`].
    pub fn subscribers(&self, event: &str) -> Vec<String> {
        let needle = event.to_lowercase();
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(_, e)| e.running && e.events.contains(&needle))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// koma->ext fire-and-forget `Event`: serialize and queue `KomaMsg::Event { name,
    /// params }` on `ext_id`'s writer channel. NON-BLOCKING — grabs the writer under a
    /// brief lock, drops the lock, then sends onto the (unbounded) writer channel;
    /// never awaits, so it is safe to call directly from the event loop. Returns
    /// `false` (frame dropped, nothing queued) if `ext_id` is not running; `true` once
    /// the frame is handed to the writer task (delivery itself is still best-effort,
    /// same as every other write onto that channel).
    pub fn notify(&self, ext_id: &str, name: &str, params: serde_json::Value) -> bool {
        let writer = {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let entry = match inner.get(ext_id) {
                Some(e) if e.running => e,
                _ => return false,
            };
            match &entry.writer {
                Some(w) => w.clone(),
                None => return false,
            }
        };

        let event = KomaMsg::Event {
            name: name.to_string(),
            params,
        };
        let mut frame = match serde_json::to_string(&event) {
            Ok(f) => f,
            Err(_) => return false,
        };
        frame.push('\n');
        writer.send(frame).is_ok()
    }

    /// Spawn + handshake a daemon-kind extension if it is not already running (a no-op
    /// for an already-running one). Blocks the calling thread until the handshake
    /// completes or fails, so a subsequent [`Self::invoke`] is guaranteed to reach a live
    /// child. oneshot-kind extensions are NOT started here (they are spawned per-invoke).
    ///
    /// The install directory is `~/.koma/extensions/<id>/` and the executable is its
    /// `exec` resolved against that dir; [`Self::ensure_started_at`] takes the install
    /// dir explicitly for tests.
    pub fn ensure_started(self: &Arc<Self>, ext: &InstalledExtension) -> Result<()> {
        let install_dir = store::extensions_dir()?.join(&ext.id);
        self.ensure_started_at(ext, &install_dir)
    }

    /// [`Self::ensure_started`] with an explicit `install_dir` (the unpacked package
    /// root). Public-in-crate so the integration test can start an extension unpacked
    /// into a temp dir rather than the real `extensions/` root.
    pub(crate) fn ensure_started_at(
        self: &Arc<Self>,
        ext: &InstalledExtension,
        install_dir: &Path,
    ) -> Result<()> {
        if ext.kind != "daemon" {
            bail!(
                "ensure_started is for daemon extensions only (got kind '{}')",
                ext.kind
            );
        }

        // Fast path: already live → no-op.
        {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if inner.get(&ext.id).map(|e| e.running).unwrap_or(false) {
                return Ok(());
            }
        }

        // Bump this extension's generation under the lock and capture it. Any in-flight
        // start for a previous generation will discard itself at the store step. Also
        // (re-)record this extension's granted scopes here — parsed from the persisted
        // wire strings — so the reader task spawned by this start gates `agents.*` calls
        // against exactly what koma extended to it (refreshed on every start/restart).
        // `events` is likewise refreshed here (best-effort read of the on-disk
        // manifest — see [`read_events_best_effort`]) so [`Self::subscribers`] always
        // reflects the CURRENT manifest rather than whatever was true at install time.
        let granted = broker::parse_grants(&ext.granted);
        let events = read_events_best_effort(install_dir);
        let gen_at_start = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let entry = inner.entry(ext.id.clone()).or_default();
            entry.generation = entry.generation.wrapping_add(1);
            entry.granted = granted;
            entry.events = events;
            entry.generation
        };

        let sock_path = store::ext_sock_path(&ext.id)?;
        let token = uuid::Uuid::new_v4().to_string();

        // Bridge sync→async: spawn the bind+spawn+handshake, block on its result. We do
        // NOT use `Handle::block_on` (the caller may already be inside the runtime, where
        // it panics); we spawn onto the runtime and block this thread on a std mpsc.
        let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
        let mgr = Arc::clone(self);
        let ext_id = ext.id.clone();
        let exec = ext.exec.clone();
        let install_dir = install_dir.to_path_buf();
        self.handle.spawn(async move {
            let r = mgr
                .connect_install(
                    &ext_id,
                    &sock_path,
                    &install_dir,
                    &exec,
                    &token,
                    gen_at_start,
                )
                .await;
            let _ = tx.send(r);
        });

        match rx.recv_timeout(CONNECT_TIMEOUT + Duration::from_secs(2)) {
            Ok(r) => r,
            Err(_) => bail!("extension '{}' start timed out", ext.id),
        }
    }

    /// Async spawn+handshake, then — only if the generation still matches — commit the
    /// live child + writer + pending map and spawn the reader/writer tasks. On a
    /// generation mismatch the freshly-spawned child is killed and nothing is stored, so
    /// a slow start can never resurrect an extension the caller just stopped.
    async fn connect_install(
        self: &Arc<Self>,
        ext_id: &str,
        sock_path: &Path,
        install_dir: &Path,
        exec: &str,
        token: &str,
        gen: u64,
    ) -> Result<()> {
        let Handshaked {
            mut child,
            reader,
            write_half,
        } = connect_and_handshake(sock_path, install_dir, exec, token).await?;

        let (wtx, wrx) = mpsc::unbounded_channel::<String>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        // Commit only if this attempt is still current.
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let entry = inner.entry(ext_id.to_string()).or_default();
            if entry.generation != gen {
                // Superseded mid-handshake (a stop/restart bumped the generation). Kill
                // the just-spawned child, unlink the socket, store nothing.
                let _ = child.start_kill();
                let _ = std::fs::remove_file(sock_path);
                bail!("extension '{ext_id}' start superseded");
            }
            entry.running = true;
            entry.writer = Some(wtx.clone());
            entry.pending = Arc::clone(&pending);
            entry.child = Some(child);
            entry.last_health_ok = true;
        }

        // Now committed under the matching generation: spawn the two long-lived tasks.
        // The reader gets a writer clone so it can answer ext→koma `Call`s.
        self.handle.spawn(writer_task(write_half, wrx));
        self.handle.spawn(reader_task(
            reader,
            pending,
            wtx,
            Arc::clone(self),
            ext_id.to_string(),
            gen,
        ));
        Ok(())
    }

    /// koma→ext `Invoke`: send `method`/`params` to the running extension and block
    /// until its `Result` (or [`CALL_TIMEOUT`]) lands, returning the `result` value.
    /// Thin wrapper over [`Self::invoke_with_timeout`] using the default timeout.
    pub fn invoke(
        &self,
        ext_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.invoke_with_timeout(ext_id, method, params, CALL_TIMEOUT)
    }

    /// [`Self::invoke`] with an explicit round-trip `timeout` instead of the default
    /// [`CALL_TIMEOUT`].
    ///
    /// THE SYNC→ASYNC BRIDGE (mirrors `SecDaemonManager::execute_blocking`): under a
    /// brief lock it grabs the writer + a fresh id + a clone of the `pending` map, drops
    /// the lock, registers a `oneshot`, spawns the write, and bridges the `oneshot` to a
    /// `std::sync::mpsc` this thread blocks on with `recv_timeout`.
    pub fn invoke_with_timeout(
        &self,
        ext_id: &str,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let (writer, id, pending, gen) = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let entry = match inner.get_mut(ext_id) {
                Some(e) if e.running => e,
                _ => bail!("extension '{ext_id}' not running"),
            };
            let writer = match &entry.writer {
                Some(w) => w.clone(),
                None => bail!("extension '{ext_id}' not running"),
            };
            let id = entry.next_id;
            entry.next_id = entry.next_id.wrapping_add(1);
            (writer, id, Arc::clone(&entry.pending), entry.generation)
        };

        let invoke = KomaMsg::Invoke {
            id,
            method: method.to_string(),
            params,
        };
        let mut frame =
            serde_json::to_string(&invoke).map_err(|e| anyhow::anyhow!("serialize invoke: {e}"))?;
        frame.push('\n');

        // Register the oneshot under this id — but ONLY if `ext_id`'s generation is
        // still the one snapshotted above, checked+inserted ATOMICALLY under the same
        // manager lock `stop()` takes before it bumps the generation. Without this, a
        // concurrent `stop()` racing between the snapshot above and here would orphan
        // this oneshot: `stop()` drains and replaces `entry.pending` with a fresh empty
        // map, so an insert into the OLD (captured) map that lands after `stop()` has
        // already run — and after the reader task has already done its own final
        // drain-on-EOF — would never be observed by anything again, silently waiting
        // out the full `CALL_TIMEOUT` instead of failing fast. Re-checking here closes
        // that window: if `stop()` already bumped the generation, fail immediately.
        let (otx, orx) = oneshot::channel::<Reply>();
        {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let still_current = inner.get(ext_id).map(|e| e.generation) == Some(gen);
            if !still_current {
                bail!("extension '{ext_id}' not running");
            }
            pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(id, otx);
        }

        // Bridge the async oneshot to a sync mpsc this thread blocks on.
        let (tx, rx) = std::sync::mpsc::channel::<Reply>();
        let pending_for_task = Arc::clone(&pending);
        let method_owned = method.to_string();
        self.handle.spawn(async move {
            if writer.send(frame).is_err() {
                pending_for_task
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                let _ = tx.send(Err("extension not running".to_string()));
                return;
            }
            let r = match orx.await {
                Ok(r) => r,
                Err(_) => Err("extension stopped before reply".to_string()),
            };
            let _ = tx.send(r);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => bail!("{e}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                bail!("extension '{ext_id}' invoke '{method_owned}' timed out");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                bail!("extension '{ext_id}' invoke '{method_owned}' task dropped before result");
            }
        }
    }

    /// Stop one extension: bump its generation (superseding any in-flight start and the
    /// live tasks), fail every pending caller, drop the writer, `start_kill` the child,
    /// and unlink its socket. Idempotent and non-blocking (an absent extension is a
    /// no-op). The generation bump means the reader task's EOF `mark_stopped` won't
    /// clobber a later start.
    pub fn stop(&self, ext_id: &str) {
        let (mut child, pending) = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let Some(entry) = inner.get_mut(ext_id) else {
                return;
            };
            entry.generation = entry.generation.wrapping_add(1);
            entry.running = false;
            entry.writer = None; // drop → writer task ends → connection closes
            let pending = std::mem::take(&mut entry.pending);
            let child = entry.child.take();
            (child, pending)
        };

        // Fail every in-flight caller so their recv_timeout returns promptly.
        {
            let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
            for (_id, tx) in map.drain() {
                let _ = tx.send(Err("extension stopped".to_string()));
            }
        }

        if let Some(child) = child.as_mut() {
            let _ = child.start_kill();
        }
        if let Ok(sock) = store::ext_sock_path(ext_id) {
            let _ = std::fs::remove_file(sock);
        }
    }

    /// Stop every running extension (used at shutdown). Snapshots the id set under the
    /// lock, then stops each.
    pub fn stop_all(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.keys().cloned().collect()
        };
        for id in ids {
            self.stop(&id);
        }
    }

    /// `true` once the handshake for `ext_id` succeeded and it is live.
    pub fn is_running(&self, ext_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(ext_id)
            .map(|e| e.running)
            .unwrap_or(false)
    }

    /// Mark `ext_id` stopped, but ONLY if the generation still matches (so a fresh start
    /// that bumped the generation is never clobbered by a stale reader's EOF). Called by
    /// the reader task on connection close.
    pub(crate) fn mark_stopped(&self, ext_id: &str, gen: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = inner.get_mut(ext_id) {
            if entry.generation != gen {
                return;
            }
            entry.running = false;
            entry.writer = None;
        }
    }

    /// Record the extension's last `Health{ok}` (generation-guarded). Advisory only.
    pub(crate) fn note_health(&self, ext_id: &str, ok: bool, gen: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = inner.get_mut(ext_id) {
            if entry.generation == gen {
                entry.last_health_ok = ok;
            }
        }
    }
}

/// Best-effort read of `<install_dir>/manifest.json`'s `contributes.events`,
/// lowercased and deduped. A missing/unparsable manifest yields an empty list
/// (never fails the start it's called from) — event subscription is advisory,
/// unlike `granted` which gates security and is parsed from the persisted
/// registry instead of the on-disk manifest.
fn read_events_best_effort(install_dir: &Path) -> Vec<String> {
    let path = install_dir.join("manifest.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let Ok(manifest) =
        serde_json::from_slice::<koma_extension::protocol::ExtensionManifest>(&bytes)
    else {
        return Vec::new();
    };
    let mut events: Vec<String> = manifest
        .contributes
        .events
        .into_iter()
        .map(|e| e.to_lowercase())
        .collect();
    events.sort();
    events.dedup();
    events
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

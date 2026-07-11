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

use koma_extension::protocol::KomaMsg;

use crate::model::app_config::InstalledExtension;
use crate::model::store;

pub mod install;
pub mod register;
mod wire;
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
}

/// The extension host manager. Holds the runtime [`Handle`] (so async socket work can be
/// spawned from synchronous code) and a mutex-guarded map of per-extension
/// [`ExtEntry`]s keyed by extension id. Constructed inert via [`Self::new`]; a child is
/// only spawned by [`Self::ensure_started`].
pub struct ExtHostManager {
    handle: Handle,
    inner: Mutex<HashMap<String, ExtEntry>>,
}

impl ExtHostManager {
    /// Build an inert manager: no extensions running, nothing spawned.
    pub fn new(handle: &Handle) -> Arc<Self> {
        Arc::new(Self {
            handle: handle.clone(),
            inner: Mutex::new(HashMap::new()),
        })
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
            bail!("ensure_started is for daemon extensions only (got kind '{}')", ext.kind);
        }

        // Fast path: already live → no-op.
        {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if inner.get(&ext.id).map(|e| e.running).unwrap_or(false) {
                return Ok(());
            }
        }

        // Bump this extension's generation under the lock and capture it. Any in-flight
        // start for a previous generation will discard itself at the store step.
        let gen_at_start = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let entry = inner.entry(ext.id.clone()).or_default();
            entry.generation = entry.generation.wrapping_add(1);
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
                .connect_install(&ext_id, &sock_path, &install_dir, &exec, &token, gen_at_start)
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
    ///
    /// THE SYNC→ASYNC BRIDGE (mirrors `SecDaemonManager::execute_blocking`): under a
    /// brief lock it grabs the writer + a fresh id + a clone of the `pending` map, drops
    /// the lock, registers a `oneshot`, spawns the write, and bridges the `oneshot` to a
    /// `std::sync::mpsc` this thread blocks on with `recv_timeout`.
    pub fn invoke(
        &self,
        ext_id: &str,
        method: &str,
        params: serde_json::Value,
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
        let mut frame = serde_json::to_string(&invoke)
            .map_err(|e| anyhow::anyhow!("serialize invoke: {e}"))?;
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

        match rx.recv_timeout(CALL_TIMEOUT) {
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

#[cfg(test)]
mod tests {
    use super::wire;
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::path::PathBuf;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Sign `zip_bytes` with a deterministic test keypair (seeded by `seed`) and
    /// return `(pubkey_b64, sha_hex, sig_b64)` — the three arguments
    /// [`install::install_from_zip_to`] needs, computed the same way it verifies them.
    fn sign(zip_bytes: &[u8], seed: u8) -> (String, String, String) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let pubkey_b64 = b64(&signing.verifying_key().to_bytes());
        let digest = Sha256::digest(zip_bytes);
        let sha_hex = install::hex_encode(&digest);
        let sig_b64 = b64(&signing.sign(digest.as_slice()).to_bytes());
        (pubkey_b64, sha_hex, sig_b64)
    }

    /// A minimal but schema-complete manifest (all fields `ExtensionManifest` requires,
    /// no `contributes`/`description` since those have `#[serde(default)]`), with `id`
    /// and `runtime.exec` parameterised so the id-whitelist and exec-escape tests can
    /// drive both without needing a real executable.
    fn minimal_manifest_json(id: &str, exec: &str) -> String {
        serde_json::json!({
            "schema": "koma-extension/v0",
            "id": id,
            "name": "test-ext",
            "version": "0.0.1",
            "tier": "free",
            "kind": "daemon",
            "runtime": { "exec": exec, "args": [] },
            "requires": []
        })
        .to_string()
    }

    /// Pack a zip containing ONLY `manifest.json` — enough to drive the id-whitelist
    /// and exec-escape guards, both of which reject before (or without needing) an
    /// exec entry to actually exist in the archive.
    fn pack_manifest_only_zip(manifest_json: &str) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("manifest.json", opts).unwrap();
            zw.write_all(manifest_json.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        cursor.into_inner()
    }

    /// The freshly-built echo sample binary (built by `cargo build --workspace
    /// --release` in `src-extension/`). Skipping when absent keeps the test green on a
    /// checkout that hasn't built the samples yet.
    fn sample_binary() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src-extension")
            .join("target")
            .join("release")
            .join("echo-tool-daemon")
    }

    /// The echo sample's manifest, with `runtime.exec` rewritten to `bin/echo-tool-daemon`
    /// exactly as `pack.sh` does for the packaged form.
    fn sample_manifest_json() -> String {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src-extension")
            .join("example")
            .join("echo-tool-daemon")
            .join("manifest.json");
        let mut v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&src).expect("read sample manifest"))
                .expect("parse sample manifest");
        v["runtime"]["exec"] = serde_json::Value::String("bin/echo-tool-daemon".to_string());
        serde_json::to_string_pretty(&v).unwrap()
    }

    /// Pack a `manifest.json` + `bin/echo-tool-daemon` zip in memory (stored, no
    /// compression — reading it exercises the real unzip path all the same).
    fn pack_zip(binary: &Path, manifest_json: &str) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            zw.start_file("manifest.json", opts).unwrap();
            zw.write_all(manifest_json.as_bytes()).unwrap();
            zw.add_directory("bin", opts).unwrap();
            zw.start_file("bin/echo-tool-daemon", opts).unwrap();
            zw.write_all(&std::fs::read(binary).unwrap()).unwrap();
            zw.finish().unwrap();
        }
        cursor.into_inner()
    }

    /// Proves the load-bearing path end to end WITHOUT the full koma daemon:
    /// real signature-verify + unzip install, a tamper rejection, then spawn +
    /// handshake + the echo `Invoke` roundtrip over the unix socket, then reap.
    #[test]
    fn install_verify_and_echo_roundtrip() {
        let binary = sample_binary();
        if !binary.exists() {
            eprintln!(
                "SKIP install_verify_and_echo_roundtrip: {} missing \
                 (run `cargo build --workspace --release` in src-extension/)",
                binary.display()
            );
            return;
        }

        // Freshly pack the echo sample (its binary now speaks host mode).
        let zip_bytes = pack_zip(&binary, &sample_manifest_json());

        // Deterministic test keypair; sign the zip's 32-byte SHA-256 digest.
        let signing = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey_b64 = b64(&signing.verifying_key().to_bytes());
        let digest = Sha256::digest(&zip_bytes);
        let sha_hex = install::hex_encode(&digest);
        let sig_b64 = b64(&signing.sign(digest.as_slice()).to_bytes());

        // Install through the REAL verify + unpack pipeline into a temp dir.
        let tmp = std::env::temp_dir().join(format!("koma-ext-test-{}", uuid::Uuid::new_v4()));
        let installed =
            install::install_from_zip_to(&zip_bytes, &sha_hex, &sig_b64, &pubkey_b64, &tmp)
                .expect("signed install should succeed");
        assert_eq!(installed.id, "run.koma.example.echo-tool-daemon");
        assert_eq!(installed.kind, "daemon");
        assert_eq!(installed.exec, "bin/echo-tool-daemon");
        assert_eq!(installed.tier, "free");

        // Tamper: flip a byte → SHA-256 mismatch → hard reject (nothing installed).
        let mut bad = zip_bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xff;
        assert!(
            install::install_from_zip_to(&bad, &sha_hex, &sig_b64, &pubkey_b64, &tmp).is_err(),
            "tampered zip must be rejected"
        );

        // Start it, invoke echo, assert the roundtrip, then stop + assert reaped.
        let _ = store::ensure_dirs();
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mgr = ExtHostManager::new(rt.handle());
        let install_dir = tmp.join(&installed.id);

        mgr.ensure_started_at(&installed, &install_dir)
            .expect("ensure_started should hand-shake");
        assert!(mgr.is_running(&installed.id), "extension should be running");

        let out = mgr
            .invoke(
                &installed.id,
                "tool.call",
                serde_json::json!({ "name": "echo", "args": { "text": "ping" } }),
            )
            .expect("invoke echo");
        assert_eq!(out, serde_json::json!({ "output": "ping" }));

        mgr.stop(&installed.id);
        assert!(!mgr.is_running(&installed.id), "extension should be stopped");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Wave B: an extension's `contributes.tools` (the echo sample declares one
    /// `echo` tool) registers as `mcp__<sanitized-id>__echo` on a live
    /// [`crate::app::mcp::McpManager`], and a call through
    /// `McpManager::execute_blocking` — the SAME dispatch path a model's tool
    /// call actually takes, unlike `install_verify_and_echo_roundtrip` above
    /// which calls `ExtHostManager::invoke` directly — routes end to end through
    /// `ExtHostManager::invoke` and returns the echoed output. Also proves
    /// `purge_extension_tools` removes it again (the uninstall/disable shape).
    #[test]
    fn extension_tool_registers_and_routes_through_mcp_manager() {
        let binary = sample_binary();
        if !binary.exists() {
            eprintln!(
                "SKIP extension_tool_registers_and_routes_through_mcp_manager: {} missing \
                 (run `cargo build --workspace --release` in src-extension/)",
                binary.display()
            );
            return;
        }

        let zip_bytes = pack_zip(&binary, &sample_manifest_json());
        let signing = SigningKey::from_bytes(&[77u8; 32]);
        let pubkey_b64 = b64(&signing.verifying_key().to_bytes());
        let digest = Sha256::digest(&zip_bytes);
        let sha_hex = install::hex_encode(&digest);
        let sig_b64 = b64(&signing.sign(digest.as_slice()).to_bytes());

        let tmp = std::env::temp_dir().join(format!("koma-ext-mcp-test-{}", uuid::Uuid::new_v4()));
        let installed =
            install::install_from_zip_to(&zip_bytes, &sha_hex, &sig_b64, &pubkey_b64, &tmp)
                .expect("signed install should succeed");

        let _ = store::ensure_dirs();
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let ext_mgr = ExtHostManager::new(rt.handle());
        let install_dir = tmp.join(&installed.id);
        ext_mgr
            .ensure_started_at(&installed, &install_dir)
            .expect("ensure_started should hand-shake");

        // Read the manifest back exactly as `register::register_contributions`
        // does, to get `contributes.tools`.
        let manifest_bytes =
            std::fs::read(install_dir.join("manifest.json")).expect("read manifest");
        let manifest: koma_extension::protocol::ExtensionManifest =
            serde_json::from_slice(&manifest_bytes).expect("parse manifest");
        assert_eq!(manifest.contributes.tools.len(), 1, "sample declares one tool");

        let mcp = crate::app::mcp::McpManager::connect_all(rt.handle(), &[]);
        mcp.register_extension_tools(
            &installed.id,
            &manifest.contributes.tools,
            std::sync::Arc::clone(&ext_mgr),
        );

        // Advertised alongside regular MCP tools, namespaced mcp__<ext>__<tool>.
        let names = mcp.tool_names();
        let namespaced = names
            .iter()
            .find(|n| n.ends_with("__echo"))
            .cloned()
            .expect("echo tool should be advertised");
        assert!(namespaced.starts_with("mcp__"), "namespaced as mcp__<ext>__echo");
        assert!(
            mcp.tool_defs().iter().any(|d| d.function.name == namespaced),
            "the same namespaced tool must appear in tool_defs()"
        );

        // Call it through the model-facing dispatch path — NOT ExtHostManager
        // directly — and confirm it reaches the extension and returns "ping".
        let result = mcp
            .execute_blocking(&namespaced, &serde_json::json!({ "text": "ping" }))
            .expect("extension tool call should succeed");
        assert_eq!(result, "ping");

        // Lifecycle: purge (uninstall/disable) removes it again.
        mcp.purge_extension_tools(&installed.id);
        assert!(
            mcp.tool_names().is_empty(),
            "purge_extension_tools should remove every tool for this ext_id"
        );

        ext_mgr.stop(&installed.id);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// CRITICAL regression test: a manifest with `id = "."` must be rejected, and —
    /// the actual bug — must NOT wipe a sibling extension directory. Pre-fix,
    /// `validate_id`'s blacklist let `"."` through; `dest_root.join(".")` resolves to
    /// `dest_root` ITSELF, so `unpack`'s "clean reinstall" `remove_dir_all(&dest)`
    /// would delete every already-installed extension in one shot. The whitelist in
    /// `validate_id` now rejects `"."` outright (no alphanumeric char), before any
    /// path is even built from the id, so a decoy sibling planted in the same
    /// `dest_root` must survive the failed install untouched.
    #[test]
    fn install_rejects_dot_id_without_deleting_siblings() {
        let tmp =
            std::env::temp_dir().join(format!("koma-ext-test-dotid-{}", uuid::Uuid::new_v4()));
        let decoy = tmp.join("some.other.extension");
        std::fs::create_dir_all(decoy.join("bin")).expect("create decoy dir");
        let marker = decoy.join("bin").join("marker");
        std::fs::write(&marker, b"decoy").expect("write decoy marker");

        let manifest_json = minimal_manifest_json(".", "bin/tool");
        let zip_bytes = pack_manifest_only_zip(&manifest_json);
        let (pubkey_b64, sha_hex, sig_b64) = sign(&zip_bytes, 45);

        let result =
            install::install_from_zip_to(&zip_bytes, &sha_hex, &sig_b64, &pubkey_b64, &tmp);
        assert!(result.is_err(), "id = \".\" must be rejected");
        assert!(
            marker.exists(),
            "a rejected id=\".\" install must not touch a sibling extension directory"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// MAJOR regression test: `runtime.exec` bypasses the zip-entry `safe_rel_path`
    /// guard (it's read from `manifest.json`, not a zip entry name), so
    /// `install::safe_exec_rel` must independently reject both an absolute exec path
    /// (which would REPLACE the install dir in `Path::join`, escaping it entirely) and
    /// a `..`-relative one (which would climb out of it).
    #[test]
    fn install_rejects_absolute_and_escaping_exec() {
        let tmp = std::env::temp_dir()
            .join(format!("koma-ext-test-execguard-{}", uuid::Uuid::new_v4()));

        for bad_exec in ["/etc/passwd", "../escape"] {
            let manifest_json = minimal_manifest_json("com.koma.test.execguard", bad_exec);
            let zip_bytes = pack_manifest_only_zip(&manifest_json);
            let (pubkey_b64, sha_hex, sig_b64) = sign(&zip_bytes, 46);

            let result =
                install::install_from_zip_to(&zip_bytes, &sha_hex, &sig_b64, &pubkey_b64, &tmp);
            assert!(
                result.is_err(),
                "runtime.exec = {bad_exec:?} must be rejected"
            );
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// MAJOR regression test: the frame-size cap on the wire read path. Drives
    /// `wire::read_capped_line` directly over an in-memory `tokio::io::duplex` pipe
    /// (a full spawned-extension handshake is unnecessary weight for this) and checks
    /// all three shapes: a normal small frame is unaffected, a frame landing EXACTLY
    /// on the cap boundary still parses, and one byte over the cap is rejected
    /// (`FrameReadError::TooLarge`) rather than buffered without bound.
    #[test]
    fn read_capped_line_rejects_oversized_frame() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        rt.block_on(async {
            let cap = 16usize;

            // A normal small frame, well under the cap, is unaffected.
            let (mut tx, mut rx) = tokio::io::duplex(1024);
            tokio::io::AsyncWriteExt::write_all(&mut tx, b"hi\n")
                .await
                .unwrap();
            drop(tx);
            let got = wire::read_capped_line(&mut rx, cap)
                .await
                .expect("small frame should parse");
            assert_eq!(got, Some("hi".to_string()));

            // Boundary: exactly `cap` bytes of content + newline must still parse.
            let (mut tx, mut rx) = tokio::io::duplex(1024);
            let ok_line = "a".repeat(cap);
            tokio::io::AsyncWriteExt::write_all(&mut tx, format!("{ok_line}\n").as_bytes())
                .await
                .unwrap();
            drop(tx);
            let got = wire::read_capped_line(&mut rx, cap)
                .await
                .expect("boundary frame should parse");
            assert_eq!(got, Some(ok_line));

            // One byte over the cap must be rejected outright, not silently buffered —
            // the whole point of the guard (a memory-DoS defense).
            let (mut tx, mut rx) = tokio::io::duplex(1024);
            let bad_line = "a".repeat(cap + 1);
            tokio::io::AsyncWriteExt::write_all(&mut tx, format!("{bad_line}\n").as_bytes())
                .await
                .unwrap();
            drop(tx);
            let err = wire::read_capped_line(&mut rx, cap)
                .await
                .expect_err("oversized frame must be rejected");
            assert!(matches!(err, wire::FrameReadError::TooLarge));
        });
    }
}

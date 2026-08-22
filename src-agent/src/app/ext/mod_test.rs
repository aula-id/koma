#![allow(clippy::unwrap_used, clippy::expect_used)]
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
/// exactly as `pack.sh` does for the packaged form, and `id` overridden to `id` — the
/// four subprocess-spawning tests below each pass a UNIQUE id here so they never
/// collide on `store::ext_sock_path`'s fixed `~/.koma/run/ext-<id>.sock` path when run
/// concurrently (a shared id let concurrent tests steal each other's listener, causing
/// a flaky "extension did not connect within 10s").
fn sample_manifest_json(id: &str) -> String {
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
    v["id"] = serde_json::Value::String(id.to_string());
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

    // Freshly pack the echo sample (its binary now speaks host mode). Unique id
    // (see `sample_manifest_json` doc) so this test's socket path never collides
    // with the other three subprocess-spawning tests in this module.
    let ext_id = "run.koma.example.echo-tool-daemon-roundtrip";
    let zip_bytes = pack_zip(&binary, &sample_manifest_json(ext_id));

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
    assert_eq!(installed.id, ext_id);
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
    assert!(
        !mgr.is_running(&installed.id),
        "extension should be stopped"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Wave 2: `ExtHostManager::notify` reaches a running extension's `on_event`
/// (koma->ext `KomaMsg::Event`). Fires `notify`, then `invoke`s the echo
/// sample's `debug.last_event` test hook (which `on_event` populates) — the
/// writer channel is FIFO, so the notify frame is guaranteed to land before
/// the invoke frame queued after it, making the ordering deterministic
/// without a sleep/poll.
#[test]
fn notify_reaches_extension_on_event() {
    let binary = sample_binary();
    if !binary.exists() {
        eprintln!(
            "SKIP notify_reaches_extension_on_event: {} missing \
             (run `cargo build --workspace --release` in src-extension/)",
            binary.display()
        );
        return;
    }

    // Unique id (see `sample_manifest_json` doc) so this test's socket path never
    // collides with the other three subprocess-spawning tests in this module.
    let zip_bytes = pack_zip(
        &binary,
        &sample_manifest_json("run.koma.example.echo-tool-daemon-notify"),
    );
    let signing = SigningKey::from_bytes(&[91u8; 32]);
    let pubkey_b64 = b64(&signing.verifying_key().to_bytes());
    let digest = Sha256::digest(&zip_bytes);
    let sha_hex = install::hex_encode(&digest);
    let sig_b64 = b64(&signing.sign(digest.as_slice()).to_bytes());

    let tmp =
        std::env::temp_dir().join(format!("koma-ext-notify-test-{}", uuid::Uuid::new_v4()));
    let installed =
        install::install_from_zip_to(&zip_bytes, &sha_hex, &sig_b64, &pubkey_b64, &tmp)
            .expect("signed install should succeed");

    let _ = store::ensure_dirs();
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mgr = ExtHostManager::new(rt.handle());
    let install_dir = tmp.join(&installed.id);
    mgr.ensure_started_at(&installed, &install_dir)
        .expect("ensure_started should hand-shake");

    assert!(
        mgr.notify(&installed.id, "test.evt", serde_json::json!({ "x": 1 })),
        "notify to a running extension should queue the frame"
    );

    let out = mgr
        .invoke(&installed.id, "debug.last_event", serde_json::json!({}))
        .expect("invoke debug.last_event");
    assert_eq!(out["name"], serde_json::json!("test.evt"));
    assert_eq!(out["params"], serde_json::json!({ "x": 1 }));

    mgr.stop(&installed.id);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Wave 2: `notify` on an extension that was never started must fail closed
/// (return `false`) rather than panic or hang.
#[test]
fn notify_to_stopped_extension_returns_false() {
    let _ = store::ensure_dirs();
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mgr = ExtHostManager::new(rt.handle());
    let ok = mgr.notify(
        "run.koma.example.never-started",
        "test.evt",
        serde_json::json!({}),
    );
    assert!(
        !ok,
        "notify on a never-started extension must return false, not panic"
    );
}

/// Wave 2: an ext->koma `Notify` (the echo sample's `drive` hook fires
/// `Koma::panel_push` once per connection) is routed onto `ext_notify_tx`
/// and observable on the event-loop side, with the correct `ext_id` and
/// `name`. Wires a plain test channel via `set_ext_notify_tx` BEFORE
/// starting the extension so the driver's push — which fires shortly after
/// the handshake completes, on its own side thread — is guaranteed to be
/// captured.
#[test]
fn ext_notify_routes_to_channel() {
    let binary = sample_binary();
    if !binary.exists() {
        eprintln!(
            "SKIP ext_notify_routes_to_channel: {} missing \
             (run `cargo build --workspace --release` in src-extension/)",
            binary.display()
        );
        return;
    }

    // Unique id (see `sample_manifest_json` doc) so this test's socket path never
    // collides with the other three subprocess-spawning tests in this module.
    let zip_bytes = pack_zip(
        &binary,
        &sample_manifest_json("run.koma.example.echo-tool-daemon-notify-route"),
    );
    let signing = SigningKey::from_bytes(&[92u8; 32]);
    let pubkey_b64 = b64(&signing.verifying_key().to_bytes());
    let digest = Sha256::digest(&zip_bytes);
    let sha_hex = install::hex_encode(&digest);
    let sig_b64 = b64(&signing.sign(digest.as_slice()).to_bytes());

    let tmp = std::env::temp_dir().join(format!(
        "koma-ext-notify-route-test-{}",
        uuid::Uuid::new_v4()
    ));
    let installed =
        install::install_from_zip_to(&zip_bytes, &sha_hex, &sig_b64, &pubkey_b64, &tmp)
            .expect("signed install should succeed");

    let _ = store::ensure_dirs();
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mgr = ExtHostManager::new(rt.handle());

    let (tx, mut rx) = mpsc::unbounded_channel::<ExtNotify>();
    mgr.set_ext_notify_tx(tx);

    let install_dir = tmp.join(&installed.id);
    mgr.ensure_started_at(&installed, &install_dir)
        .expect("ensure_started should hand-shake");

    let notify = rt
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await
        })
        .expect("driver's panel_push should arrive within 10s")
        .expect("channel should not close");

    assert_eq!(notify.ext_id, installed.id);
    assert_eq!(notify.name, "panel.push");
    assert_eq!(notify.params["panelId"], serde_json::json!("p1"));
    assert_eq!(
        notify.params["payload"],
        serde_json::json!({ "hello": true })
    );

    mgr.stop(&installed.id);
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

    // Unique id (see `sample_manifest_json` doc) so this test's socket path never
    // collides with the other three subprocess-spawning tests in this module.
    let zip_bytes = pack_zip(
        &binary,
        &sample_manifest_json("run.koma.example.echo-tool-daemon-mcp"),
    );
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
    assert_eq!(
        manifest.contributes.tools.len(),
        1,
        "sample declares one tool"
    );

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
    assert!(
        namespaced.starts_with("mcp__"),
        "namespaced as mcp__<ext>__echo"
    );
    assert!(
        mcp.tool_defs()
            .iter()
            .any(|d| d.function.name == namespaced),
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
    let tmp =
        std::env::temp_dir().join(format!("koma-ext-test-execguard-{}", uuid::Uuid::new_v4()));

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

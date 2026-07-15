//! Extension install + verify.
//!
//! An extension ships as a signed zip whose layout is `manifest.json` + `bin/<exec>`
//! (see `src-extension/pack.sh`). Installing one is a strict, fail-closed pipeline:
//!
//! 1. **Integrity + signature FIRST, before any disk write.** The SHA-256 of the raw
//!    zip must equal `expected_sha256`, and an Ed25519 signature over the 32 RAW
//!    digest bytes must verify against the pinned release key
//!    [`KOMA_EXT_SIGNING_PUBKEY`]. Either check failing is a hard stop — nothing
//!    touches disk.
//! 2. **Unpack** into `extensions/<id>/` (id parsed from the zip's `manifest.json`)
//!    with a zip-slip guard: no absolute paths, no `..` components. Every entry's
//!    unix permission bits are restored from the zip (masked to `0o777` — never
//!    setuid/setgid) so ANY executable an extension ships keeps its exec bit, not
//!    just the declared `runtime.exec` (e.g. a daemon that itself spawns a second,
//!    unmentioned binary under `bin/`, like a standalone MCP stdio server).
//! 3. **`chmod +x`** the `bin/<exec>` on unix — belt-and-suspenders on top of step 2
//!    for the one path the manifest actually declares.
//!
//! The signed [`install_from_zip`] is the production path. [`install_dev_unsigned`]
//! (debug-only) skips step 1 for local testing against the unsigned
//! `src-extension/dist/*.zip`. Neither saves `config.json` — the caller upserts the
//! returned [`InstalledExtension`] and persists.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use koma_extension::protocol::ExtensionManifest;

use crate::model::app_config::InstalledExtension;
use crate::model::store;

/// koma extension-signing public key (Ed25519, 32-byte raw, base64). Verify-only.
/// The private key is held offline (deploy secret), never in the repo.
///
/// Used to verify the integrity and signature of signed extension packages.
/// Tests drive the real verify pipeline through [`install_from_zip_to`] with their
/// own generated keypair, so they are unaffected by this pinned production key.
pub const KOMA_EXT_SIGNING_PUBKEY: &str = "ZIMbgD12NrqrT7q0wA8B5/i76rI/DvdGkPgSU2K35Uc=";

/// Verify a signed extension zip and unpack it into `extensions/<id>/`, returning the
/// registry entry. Does NOT persist `config.json` (the caller upserts + saves).
///
/// Integrity (SHA-256) and the Ed25519 signature are checked BEFORE anything touches
/// disk; either failing is a hard stop. See the module docs for the full pipeline.
pub fn install_from_zip(
    zip_bytes: &[u8],
    expected_sha256: &str,
    signature_b64: &str,
) -> Result<InstalledExtension> {
    let dest_root = store::extensions_dir()?;
    install_from_zip_to(
        zip_bytes,
        expected_sha256,
        signature_b64,
        KOMA_EXT_SIGNING_PUBKEY,
        &dest_root,
    )
}

/// The core of [`install_from_zip`], parameterised by the verifying key and the
/// destination root so tests can drive the REAL verify+unpack pipeline with a
/// generated keypair into a temp dir. Production callers use [`install_from_zip`],
/// which pins [`KOMA_EXT_SIGNING_PUBKEY`] + [`store::extensions_dir`].
pub(crate) fn install_from_zip_to(
    zip_bytes: &[u8],
    expected_sha256: &str,
    signature_b64: &str,
    pubkey_b64: &str,
    dest_root: &Path,
) -> Result<InstalledExtension> {
    // Integrity + signature FIRST — reject before any disk write.
    verify_integrity(zip_bytes, expected_sha256, signature_b64, pubkey_b64)?;
    unpack(zip_bytes, dest_root)
}

/// DEV-ONLY: install an UNSIGNED zip (skips integrity + signature verification) into
/// `extensions/<id>/`, for local testing against the unsigned `src-extension/dist/*.zip`.
/// Compiled out of release builds so the signature gate can never be bypassed in
/// production.
#[cfg(debug_assertions)]
pub fn install_dev_unsigned(zip_path: &Path) -> Result<InstalledExtension> {
    let bytes = std::fs::read(zip_path)
        .with_context(|| format!("read extension zip {}", zip_path.display()))?;
    let dest_root = store::extensions_dir()?;
    unpack(&bytes, &dest_root)
}

/// CLI dev-sideload (`koma ext install --dev <path>`): install an UNSIGNED zip from
/// local disk, skipping integrity/signature verification exactly like
/// [`install_dev_unsigned`]. Unlike that function this is reachable in EVERY build
/// (not just `debug_assertions`) — it's an explicit, local, user-typed CLI action
/// (the developer already has the file and typed `--dev`), not a silent fallback for
/// an untrusted network response, so the release-build signature gate around the
/// store path is unaffected.
pub fn install_dev_zip(zip_path: &Path) -> Result<InstalledExtension> {
    let bytes = std::fs::read(zip_path)
        .with_context(|| format!("read extension zip {}", zip_path.display()))?;
    let dest_root = store::extensions_dir()?;
    unpack(&bytes, &dest_root)
}

/// CLI dev-sideload (`koma ext install --dev <path>`) from an already-unpacked
/// directory (`manifest.json` + `bin/<exec>` etc. at its root): parse + validate the
/// manifest exactly like [`unpack`], then COPY (never symlink — Windows has no
/// reliable unprivileged symlink) the whole tree into `extensions/<id>/`. Same trust
/// model as [`install_dev_zip`]: no signature, no store.
pub fn install_dev_dir(src_dir: &Path) -> Result<InstalledExtension> {
    let manifest_path = src_dir.join("manifest.json");
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: ExtensionManifest =
        serde_json::from_slice(&bytes).context("parse manifest.json")?;
    validate_id(&manifest.id)?;

    let dest_root = store::extensions_dir()?;
    let dest = dest_root.join(&manifest.id);
    // Clean install, same rationale as `unpack`'s reinstall-clears-stale-files
    // behaviour: `validate_id` has already rejected any id (e.g. `.`) that could
    // make `dest` resolve to `dest_root` itself or an ancestor.
    if dest.exists() {
        std::fs::remove_dir_all(&dest).with_context(|| format!("clear {}", dest.display()))?;
    }
    copy_dir_all(src_dir, &dest)?;

    // Same exec-escape guard + existence check `unpack` applies to a zip install.
    let exec_path = safe_exec_rel(&manifest.runtime.exec, &dest)?;
    if !exec_path.is_file() {
        bail!(
            "extension manifest declares runtime.exec '{}' but it was not found under {} after copy",
            manifest.runtime.exec,
            dest.display()
        );
    }

    #[cfg(unix)]
    chmod_exec(&exec_path)?;

    Ok(InstalledExtension {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        tier: enum_wire(&manifest.tier),
        granted: manifest.requires.iter().map(enum_wire).collect(),
        enabled: true,
        kind: enum_wire(&manifest.kind),
        exec: manifest.runtime.exec.clone(),
    })
}

/// `chmod +x` a manifest-declared spawnable binary on unix. Shared by [`unpack`] (zip
/// install) and [`install_dev_dir`] (dir install) — belt-and-suspenders on top of the
/// zip-mode-preservation in [`unpack`]'s extraction loop, for the one path the
/// manifest actually declares today: `runtime.exec`.
///
/// No other `Contributes` kind declares a spawnable command path as of this writing —
/// `ToolDef`/`ModelDef`/`PanelDef`/`OAuthProviderDef` (see `src-extension/src/protocol.rs`)
/// carry no path field. A `contributes.tools` entry is invoked IN-PROCESS on the
/// extension's own `runtime.exec` daemon via `KomaMsg::Invoke`, not spawned as a
/// separate binary. A genuinely separate stdio MCP server an extension ships (e.g. the
/// Workflow extension's `bin/workflow-mcp`, shipped alongside `bin/office-daemon`) is
/// registered as its own `McpServerEntry` in `config.json` — via either of two routes:
/// UNDECLARED (bundled by `bin/`-directory convention only, hand-added by the user
/// through the MCP settings, `command` free to point ANYWHERE on disk), or DECLARED on
/// the manifest's `mcp_servers[]` field (`ManifestMcpServer`) and AUTO-registered at
/// install time by `app::ext::register::register_mcp_servers`, whose `exec` is resolved
/// through the SAME [`safe_exec_rel`] containment guard `runtime.exec` gets below —
/// scoped to this extension's own install dir. Either way, chmod'ing it from HERE would
/// be redundant at best (for a declared one) or unsafe overreach (for a hand-added one
/// pointing outside the install dir): that binary's exec bit is instead covered by the
/// zip-mode preservation below, which applies to every entry regardless of manifest
/// declaration.
///
/// Propagates any failure instead of swallowing it — a silent chmod failure would only
/// surface later as a confusing "permission denied" at first spawn.
#[cfg(unix)]
fn chmod_exec(exec_path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(exec_path)
        .with_context(|| format!("stat {}", exec_path.display()))?;
    let mut perms = meta.permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(exec_path, perms)
        .with_context(|| format!("chmod +x {}", exec_path.display()))?;
    Ok(())
}

/// Recursively copy `src`'s contents into `dst` (created if missing). Used by
/// [`install_dev_dir`]. REAL file copies only — symlinks are skipped rather than
/// followed-or-recreated, since a dev-staged extension directory has no business
/// containing one and Windows has no reliable unprivileged symlink story.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("read dir {}", src.display()))? {
        let entry = entry.with_context(|| format!("read dir entry under {}", src.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dst_path).with_context(|| {
                format!(
                    "copy {} -> {}",
                    entry.path().display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Reject a zip whose SHA-256 differs from `expected_sha256`, or whose Ed25519
/// `signature` over the 32 raw digest bytes does not verify against `pubkey_b64`.
/// The signed message is the raw digest bytes (NOT the hex string).
fn verify_integrity(
    zip_bytes: &[u8],
    expected_sha256: &str,
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<()> {
    // 1. SHA-256 integrity. Compare hex case-insensitively.
    let digest = Sha256::digest(zip_bytes);
    let got = hex_encode(&digest);
    if !got.eq_ignore_ascii_case(expected_sha256.trim()) {
        bail!("extension integrity check failed: sha256 mismatch");
    }

    // 2. Ed25519 signature over the 32 RAW digest bytes.
    let pk_bytes = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64.trim().as_bytes())
        .context("signing pubkey is not valid base64")?;
    let pk_arr: [u8; 32] = pk_bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("signing pubkey must be 32 bytes"))?;
    let vk =
        VerifyingKey::from_bytes(&pk_arr).map_err(|e| anyhow!("invalid signing pubkey: {e}"))?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim().as_bytes())
        .context("signature is not valid base64")?;
    let sig =
        Signature::from_slice(&sig_bytes).map_err(|e| anyhow!("invalid signature encoding: {e}"))?;

    vk.verify(digest.as_slice(), &sig)
        .map_err(|_| anyhow!("extension signature verification failed"))?;
    Ok(())
}

/// Parse `manifest.json`, validate the id, then extract the zip into
/// `dest_root/<id>/` with a zip-slip guard and `chmod +x` the runtime exec.
/// Assumes integrity/signature already checked (or intentionally skipped in dev).
fn unpack(zip_bytes: &[u8], dest_root: &Path) -> Result<InstalledExtension> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("open extension zip")?;

    // Parse manifest.json first to learn the id (scoped so the mutable borrow of
    // `archive` drops before the extraction loop re-borrows it).
    let manifest: ExtensionManifest = {
        let mut mf = archive
            .by_name("manifest.json")
            .context("extension zip is missing manifest.json")?;
        let mut buf = String::new();
        mf.read_to_string(&mut buf).context("read manifest.json")?;
        serde_json::from_str(&buf).context("parse manifest.json")?
    };

    validate_id(&manifest.id)?;

    let dest = dest_root.join(&manifest.id);
    // Clean install: remove any prior copy so a downgrade/reinstall leaves no stale
    // files behind. Safe now that `validate_id` has already rejected anything (e.g.
    // `.`) that could make `dest` resolve to `dest_root` itself or an ancestor.
    if dest.exists() {
        std::fs::remove_dir_all(&dest).with_context(|| format!("clear {}", dest.display()))?;
    }
    std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;

    // Zip-bomb guard: bounds both a single entry's uncompressed size and the total
    // unpacked across the whole archive. Defense in depth — today only signed (or
    // dev-unsigned) zips ever reach `unpack`, but this stays cheap insurance if that
    // trust boundary ever widens.
    let mut total_unpacked: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("read zip entry {i}"))?;
        // zip-slip guard: reject absolute paths and any `..`/root component.
        let rel = safe_rel_path(entry.name())
            .ok_or_else(|| anyhow!("unsafe path in extension zip: {}", entry.name()))?;
        if rel.as_os_str().is_empty() {
            continue; // the root entry, or a pure "./" — nothing to write
        }
        let out_path = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .with_context(|| format!("create dir {}", out_path.display()))?;
        } else {
            // Reject an entry whose DECLARED size already exceeds the cap (cheap,
            // though the value comes from the zip's central directory and is
            // technically attacker-controlled) — the real enforcement is the bounded
            // copy below, which never trusts the declared size.
            if entry.size() > MAX_ENTRY_BYTES {
                bail!(
                    "extension zip entry '{}' declares {} bytes, exceeding the {}-byte cap",
                    entry.name(),
                    entry.size(),
                    MAX_ENTRY_BYTES
                );
            }
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create dir {}", parent.display()))?;
            }
            let mut out = std::fs::File::create(&out_path)
                .with_context(|| format!("create {}", out_path.display()))?;
            // Bound the ACTUAL copy at cap+1 bytes so a mismatched/lying declared size
            // can never write more than the cap onto disk before we notice.
            let mut limited = (&mut entry).take(MAX_ENTRY_BYTES + 1);
            let copied = std::io::copy(&mut limited, &mut out)
                .with_context(|| format!("write {}", out_path.display()))?;
            if copied > MAX_ENTRY_BYTES {
                let _ = std::fs::remove_file(&out_path);
                bail!(
                    "extension zip entry '{}' exceeds the {}-byte cap while extracting",
                    entry.name(),
                    MAX_ENTRY_BYTES
                );
            }
            total_unpacked = total_unpacked.saturating_add(copied);
            if total_unpacked > MAX_TOTAL_UNPACKED_BYTES {
                bail!(
                    "extension zip exceeds the total unpacked-size cap ({} bytes)",
                    MAX_TOTAL_UNPACKED_BYTES
                );
            }

            // Restore the entry's unix permission bits from the zip, masked to the
            // standard rwx bits (never setuid/setgid/sticky). This is what actually
            // fixes a SECOND binary an extension ships alongside its declared
            // `runtime.exec` (e.g. a daemon that itself spawns a standalone MCP stdio
            // server under `bin/`) — `std::fs::File::create` above wrote it with a
            // plain umask-default mode, not the zip's recorded mode, so without this
            // it would never be executable regardless of what the manifest declares.
            // `unix_mode()` is `None` for a zip built on a non-unix host (e.g. plain
            // `zip`/`unzip` on Windows, which records no unix attributes) — a no-op
            // there, same as the runtime.exec chmod below.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    let mode = mode & 0o777;
                    if mode != 0 {
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))
                            .with_context(|| {
                                format!("restore permissions for {}", out_path.display())
                            })?;
                    }
                }
            }
        }
    }

    // Resolve + validate `runtime.exec` against the install dir. This is NOT covered
    // by `safe_rel_path` above (that guard only applies to zip ENTRY names) — `exec`
    // comes straight from `manifest.json`'s `runtime.exec` field, which is re-read at
    // every boot (see `wire::connect_and_handshake`), so an absolute path or a `..`
    // escape here would let a "clean" zip point the spawned executable anywhere on
    // disk. Fail the INSTALL (rather than silently succeeding and failing later at
    // first spawn) if the declared exec doesn't even exist after unpack.
    let exec_path = safe_exec_rel(&manifest.runtime.exec, &dest)?;
    if !exec_path.is_file() {
        bail!(
            "extension manifest declares runtime.exec '{}' but it was not found under {} after unpack",
            manifest.runtime.exec,
            dest.display()
        );
    }

    // Belt-and-suspenders chmod of the one path the manifest actually declares, on top
    // of the zip-mode preservation above.
    #[cfg(unix)]
    chmod_exec(&exec_path)?;

    Ok(InstalledExtension {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        // Store the serde WIRE strings so persistence never couples to the wire
        // crate's enum shape (see `InstalledExtension`'s docs).
        tier: enum_wire(&manifest.tier),
        granted: manifest.requires.iter().map(enum_wire).collect(),
        enabled: true,
        kind: enum_wire(&manifest.kind),
        exec: manifest.runtime.exec.clone(),
    })
}

/// Reject anything but a well-formed reverse-DNS id: a WHITELIST, not a blacklist.
///
/// A prior blacklist (reject `/`, `\`, `..`) let `id = "."` slip through — and
/// `dest_root.join(".")` resolves to `dest_root` ITSELF, so the caller's
/// `remove_dir_all(&dest)` on a reinstall would wipe every already-installed
/// extension. A whitelist closes that (and every other) shape of escape at once:
/// the id must be non-empty, contain only `[A-Za-z0-9._-]`, contain at least one
/// alphanumeric character (so a pure-punctuation id like `.`, `..`, `...`, or `-` is
/// rejected even though none of those chars are individually illegal), and not
/// start or end with `.`.
fn validate_id(id: &str) -> Result<()> {
    let all_allowed = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    let has_alnum = id.chars().any(|c| c.is_ascii_alphanumeric());
    let dot_wrapped = id.starts_with('.') || id.ends_with('.');
    if !all_allowed || !has_alnum || dot_wrapped {
        bail!("invalid extension id");
    }
    Ok(())
}

/// Per-entry uncompressed-size cap for a zip-bomb guard in [`unpack`]. Defense in
/// depth: today only a signature-verified (or dev-unsigned) zip ever reaches here.
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

/// Total-unpacked-size cap across every entry in one zip, same rationale as
/// [`MAX_ENTRY_BYTES`].
const MAX_TOTAL_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;

/// Resolve `exec` (the manifest's `runtime.exec`, e.g. `"bin/tool"`) against `base`
/// (the unpacked install dir), rejecting anything that could escape `base`: empty,
/// absolute, or containing any `..`/root/prefix/current-dir component. Mirrors
/// [`safe_rel_path`]'s zip-slip guard, but `runtime.exec` never goes through that
/// guard — it is read straight out of `manifest.json`, both at install time (here)
/// and at every boot (`wire::connect_and_handshake` re-reads it from the persisted
/// registry), so `Path::join`'s "an absolute joinee REPLACES the base" footgun would
/// otherwise let a manifest point the spawned executable anywhere on disk (e.g.
/// `"/etc/passwd"` or `"../../x"`).
///
/// After joining, confirms the RESULT is still a descendant of `base` by comparing
/// path COMPONENTS (not strings, which a trailing-slash / separator mismatch could
/// fool) — belt-and-suspenders on top of the component whitelist above.
pub(crate) fn safe_exec_rel(exec: &str, base: &Path) -> Result<PathBuf> {
    if exec.is_empty() {
        bail!("extension runtime.exec is empty");
    }
    let rel = Path::new(exec);
    if rel.is_absolute() {
        bail!("extension runtime.exec must be a relative path: {exec}");
    }
    for comp in rel.components() {
        match comp {
            std::path::Component::Normal(_) => {}
            _ => bail!("extension runtime.exec contains an illegal path component: {exec}"),
        }
    }
    let joined = base.join(rel);

    let base_components: Vec<_> = base.components().collect();
    let joined_components: Vec<_> = joined.components().collect();
    if joined_components.len() <= base_components.len()
        || joined_components[..base_components.len()] != base_components[..]
    {
        bail!("extension runtime.exec escapes the install dir: {exec}");
    }
    Ok(joined)
}

/// Turn a zip entry name (always forward-slash separated per the zip spec) into a
/// clean RELATIVE path, or `None` if it is unsafe (absolute, or contains a `..`
/// component). Empty and `.` components are dropped; a trailing slash yields the
/// directory path.
fn safe_rel_path(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') || name.starts_with('\\') {
        return None; // absolute
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None, // zip-slip
            _ => {
                if part.contains('\\') {
                    return None; // windows-style separator sneaking in
                }
                out.push(part);
            }
        }
    }
    Some(out)
}

/// The serde wire string for a manifest enum (`Tier`/`ExtensionKind`/`Grant`), which
/// all serialize to a plain JSON string. Guarantees the persisted registry strings
/// equal the wire forms without hardcoding them here (future variants ride through).
fn enum_wire<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Lowercase-hex encode. `pub(crate)` so the integration test can compute the
/// `expected_sha256` argument the same way this module compares it.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // `from_digit` is infallible for a nibble (0..16).
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    s
}

//! Secrets at rest: one machine key, sealed string values, owner-only files.
//!
//! API keys and OAuth tokens used to sit in `config.json` as plain text in a
//! world-readable file. They are now written as `enc:v1:<base64>` (AES-256-GCM
//! under `secrets/machine.key`), and the files that carry them are created
//! `0600`.
//!
//! Threat model: a copy of `config.json` on its own (a backup, a sync folder, a
//! bug-report attachment) no longer carries usable credentials, and other local
//! users cannot read the file. Someone who can read the whole data directory
//! can read `machine.key` too; this does not defend against that.
//!
//! Losing `machine.key` loses the sealed values: they load as empty and the UI
//! asks for the credential again. Nothing else in the config depends on the key.
//!
//! Plain values are still accepted on load, so an older config (or one edited by
//! hand) keeps working and is sealed on the next save.

use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use base64::Engine;
use rand::RngCore;

pub(crate) const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const PREFIX: &str = "enc:v1:";
/// Ties a sealed value to this use, so ciphertext from another koma store that
/// shares the machine key cannot be pasted in and accepted.
const AAD: &[u8] = b"koma-config-secret-v1";

pub(crate) type Key = [u8; KEY_LEN];

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD_NO_PAD;

pub(crate) fn is_sealed(value: &str) -> bool {
    value.starts_with(PREFIX)
}

pub(crate) fn seal_with(key: &Key, plain: &str) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).context("init aes-gcm")?;
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain.as_bytes(),
                aad: AAD,
            },
        )
        .map_err(|error| anyhow::anyhow!("seal secret: {error}"))?;
    let mut blob = Vec::with_capacity(NONCE_LEN + sealed.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&sealed);
    Ok(format!("{PREFIX}{}", B64.encode(blob)))
}

/// `None` when `token` is not a sealed value, was sealed under another key, or
/// has been altered.
pub(crate) fn open_with(key: &Key, token: &str) -> Option<String> {
    let blob = B64.decode(token.strip_prefix(PREFIX)?).ok()?;
    if blob.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, sealed) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plain = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: AAD,
            },
        )
        .ok()?;
    String::from_utf8(plain).ok()
}

/// What opening a document's secrets found, so the caller can decide whether to
/// rewrite the file and what to report.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Opened {
    /// Non-empty values that were stored in plain text.
    pub plain: usize,
    /// Sealed values that could not be opened; they are now empty.
    pub lost: usize,
}

/// Seal one field in place. Empty and already-sealed values are left alone.
pub(crate) fn seal_field(key: &Key, field: &mut String) -> Result<()> {
    if !field.is_empty() && !is_sealed(field) {
        *field = seal_with(key, field)?;
    }
    Ok(())
}

/// Open one field in place. A sealed value that cannot be opened becomes empty,
/// which every caller already treats as "not configured".
pub(crate) fn open_field(key: Option<&Key>, field: &mut String, opened: &mut Opened) {
    if field.is_empty() {
        return;
    }
    if !is_sealed(field) {
        opened.plain += 1;
        return;
    }
    match key.and_then(|key| open_with(key, field)) {
        Some(plain) => *field = plain,
        None => {
            field.clear();
            opened.lost += 1;
        }
    }
}

// ── machine key ────────────────────────────────────────────────────────────

pub(crate) fn secrets_dir() -> Result<PathBuf> {
    Ok(crate::model::store::base_dir()?.join("secrets"))
}

pub(crate) fn key_path() -> Result<PathBuf> {
    Ok(secrets_dir()?.join("machine.key"))
}

pub(crate) fn load_key_at(path: &Path) -> Option<Key> {
    std::fs::read(path).ok()?.try_into().ok()
}

/// Read the key at `path`, creating it when absent. A key of the wrong length is
/// an error rather than a reason to mint a new one: replacing it would silently
/// orphan everything sealed under the old key.
pub(crate) fn load_or_create_key_at(path: &Path) -> Result<Key> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
        set_mode(dir, 0o700)?;
    }
    if let Some(key) = read_existing_key(path)? {
        return Ok(key);
    }
    let mut key = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    let tmp = temp_sibling(path);
    write_new_private(&tmp, &key).with_context(|| format!("write {}", tmp.display()))?;
    // Two processes can reach this point together. `hard_link` refuses an
    // existing target, so exactly one key wins and the loser adopts it instead
    // of sealing under a key that is about to be overwritten.
    let linked = std::fs::hard_link(&tmp, path);
    let outcome = match linked {
        Ok(()) => Ok(key),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            read_existing_key(path)?.context("machine key vanished while it was being created")
        }
        // No hard links on this filesystem: fall back to a plain rename.
        Err(_) => std::fs::rename(&tmp, path)
            .map(|()| key)
            .with_context(|| format!("rename {}", path.display())),
    };
    let _ = std::fs::remove_file(&tmp);
    outcome
}

fn read_existing_key(path: &Path) -> Result<Option<Key>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let length = bytes.len();
            let key = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("machine key has invalid length {length}"))?;
            Ok(Some(key))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Fixed key for unit tests, so saving a config in a test never touches the
/// real data directory and never races a test that swaps `HOME`.
#[cfg(test)]
const TEST_KEY: Key = [0x4b; KEY_LEN];

/// The key for reading. `None` when no key exists yet (nothing was ever sealed)
/// or it cannot be read.
pub(crate) fn key_for_read() -> Option<Key> {
    #[cfg(test)]
    {
        Some(TEST_KEY)
    }
    #[cfg(not(test))]
    {
        load_key_at(&key_path().ok()?)
    }
}

/// The key for writing, created on first use.
pub(crate) fn key_for_write() -> Result<Key> {
    #[cfg(test)]
    {
        Ok(TEST_KEY)
    }
    #[cfg(not(test))]
    {
        load_or_create_key_at(&key_path()?)
    }
}

// ── owner-only files ───────────────────────────────────────────────────────

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {:o} {}", mode, path.display()))
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    path.with_file_name(name)
}

/// Create `path` (which must not exist) readable by its owner only. The mode is
/// set at creation, so the bytes are never on disk under a wider one.
fn write_new_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    // Windows: the data directory already inherits a per-user ACL. Narrowing it
    // further is best effort; a failure must not block saving the config.
    #[cfg(windows)]
    let _ = crate::model::win_acl::restrict_owner_only(path);
    Ok(())
}

/// Atomically replace `path` with `bytes`, leaving it readable by its owner
/// only. Replacing (rather than rewriting) also fixes a file that was created
/// under an older, wider mode.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = temp_sibling(path);
    let written = write_new_private(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Whether anyone but the owner can read or write `path`. `None` when the file
/// is absent or the platform has no mode bits.
pub(crate) fn is_exposed(path: &Path) -> Option<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).ok()?.permissions().mode();
        Some(mode & 0o077 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
#[path = "secret_box_test.rs"]
mod tests;

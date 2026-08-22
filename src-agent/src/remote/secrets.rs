//! Encrypted-at-rest SSH passwords for saved remote hosts.
//!
//! Layout under the koma data root:
//! ```text
//! secrets/                 # 0o700
//!   machine.key            # 32 random bytes, 0o600
//!   remote-passwords.json  # host_id → {nonce, ct} AES-256-GCM
//! ```
//!
//! Threat model: protects casual readout and accidental sync of host metadata.
//! A full home-directory attacker can still read `machine.key`. If the machine
//! key is missing, reset, or corrupt, lookups fail open to `None` and the UI
//! re-prompts — passwords are never required to decrypt to connect.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

const STORE_VERSION: u32 = 1;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PasswordStore {
    version: u32,
    #[serde(default)]
    entries: HashMap<String, StoreEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreEntry {
    nonce: String,
    ct: String,
}

fn secrets_dir() -> Result<PathBuf> {
    Ok(crate::model::store::base_dir()?.join("secrets"))
}

fn machine_key_path() -> Result<PathBuf> {
    Ok(secrets_dir()?.join("machine.key"))
}

fn passwords_path() -> Result<PathBuf> {
    Ok(secrets_dir()?.join("remote-passwords.json"))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {:o} {}", mode, path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn ensure_secrets_dir() -> Result<PathBuf> {
    let dir = secrets_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    set_mode(&dir, 0o700)?;
    Ok(dir)
}

fn read_or_create_machine_key() -> Result<[u8; KEY_LEN]> {
    ensure_secrets_dir()?;
    let path = machine_key_path()?;
    if path.exists() {
        let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        if bytes.len() == KEY_LEN {
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        // Corrupt/truncated key → treat as reset (caller may recreate on set).
        anyhow::bail!("machine key has invalid length {}", bytes.len());
    }
    let mut key = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    atomic_write(&path, &key, 0o600)?;
    Ok(key)
}

fn load_machine_key_for_get() -> Option<[u8; KEY_LEN]> {
    let path = machine_key_path().ok()?;
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() != KEY_LEN {
        return None;
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes);
    Some(key)
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    // Unique tmp next to the target.
    let tmp = path.parent().unwrap_or_else(|| Path::new(".")).join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("secret"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    set_mode(&tmp, mode)?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename {}", path.display()))?;
    set_mode(path, mode)?;
    Ok(())
}

fn load_store() -> PasswordStore {
    let path = match passwords_path() {
        Ok(p) => p,
        Err(_) => return PasswordStore::default(),
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return PasswordStore::default(),
    };
    match serde_json::from_slice::<PasswordStore>(&bytes) {
        Ok(s) if s.version == STORE_VERSION => s,
        _ => PasswordStore::default(),
    }
}

fn save_store(store: &PasswordStore) -> Result<()> {
    ensure_secrets_dir()?;
    let path = passwords_path()?;
    let json = serde_json::to_vec_pretty(store).context("serialise remote password store")?;
    atomic_write(&path, &json, 0o600)
}

fn encrypt(key: &[u8; KEY_LEN], host_id: &str, password: &str) -> Result<StoreEntry> {
    let cipher = Aes256Gcm::new_from_slice(key).context("init aes-gcm")?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: password.as_bytes(),
                aad: host_id.as_bytes(),
            },
        )
        .map_err(|e| anyhow::anyhow!("encrypt password: {e}"))?;
    Ok(StoreEntry {
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
        ct: base64::engine::general_purpose::STANDARD.encode(ct),
    })
}

fn decrypt(key: &[u8; KEY_LEN], host_id: &str, entry: &StoreEntry) -> Option<String> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce_bytes = base64::engine::general_purpose::STANDARD
        .decode(&entry.nonce)
        .ok()?;
    if nonce_bytes.len() != NONCE_LEN {
        return None;
    }
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = base64::engine::general_purpose::STANDARD
        .decode(&entry.ct)
        .ok()?;
    let plain = cipher
        .decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: &ct,
                aad: host_id.as_bytes(),
            },
        )
        .ok()?;
    String::from_utf8(plain).ok()
}

/// Look up a stored SSH password for `host_id`. Returns `None` on any failure
/// (missing key/file, decrypt error, version mismatch).
pub fn get_remote_password(host_id: &str) -> Option<String> {
    if host_id.is_empty() {
        return None;
    }
    let key = load_machine_key_for_get()?;
    let store = load_store();
    let entry = store.entries.get(host_id)?;
    decrypt(&key, host_id, entry)
}

/// Encrypt and persist an SSH password for `host_id`. Creates the machine key
/// lazily on first write.
pub fn set_remote_password(host_id: &str, password: &str) -> Result<()> {
    if host_id.is_empty() {
        anyhow::bail!("host_id must not be empty");
    }
    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }
    let key = read_or_create_machine_key()?;
    let entry = encrypt(&key, host_id, password)?;
    let mut store = load_store();
    store.version = STORE_VERSION;
    store.entries.insert(host_id.to_string(), entry);
    save_store(&store)
}

/// Remove a stored password for `host_id` (host delete / auth failure).
pub fn delete_remote_password(host_id: &str) -> Result<()> {
    if host_id.is_empty() {
        return Ok(());
    }
    let mut store = load_store();
    if store.entries.remove(host_id).is_none() {
        return Ok(());
    }
    save_store(&store)
}

/// Resolve a saved host id from a user/host/port triple (for CLI `koma remote`).
pub fn host_id_for_address(user: &str, host: &str, port: Option<u16>) -> Option<String> {
    let port = port.unwrap_or(22);
    let hosts = super::hosts::load_hosts();
    hosts
        .hosts
        .into_iter()
        .find(|h| h.user == user && h.host == host && h.port == port)
        .map(|h| h.id)
}

#[cfg(test)]
#[path = "secrets_test.rs"]
mod tests;

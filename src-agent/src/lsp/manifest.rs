//! On-disk install manifest for koma-managed language servers.
//!
//! Lives at `~/.koma/lsp/manifest.json`. Tracks which catalogue ids are
//! installed, their version string, install source, and timestamp. Uninstall
//! removes the id's directory AND drops it from this file. Never touches PATH
//! copies.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::store::base_dir;

/// Root of koma-managed language servers: `~/.koma/lsp/`.
pub fn lsp_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("lsp"))
}

/// Per-server install root: `~/.koma/lsp/<id>/`.
pub fn server_dir(id: &str) -> Result<PathBuf> {
    Ok(lsp_dir()?.join(id))
}

/// Path to the shared manifest file.
pub fn manifest_path() -> Result<PathBuf> {
    Ok(lsp_dir()?.join("manifest.json"))
}

/// One installed server's record inside the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub id: String,
    pub version: String,
    /// Where the binary came from (`github`, `npm`, `pip`, `go`, `path-copy`, …).
    pub source: String,
    /// Unix epoch seconds when installed/updated.
    pub installed_at: u64,
    /// Relative path under `~/.koma/lsp/<id>/` to the primary binary, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_rel: Option<String>,
}

/// Full manifest document.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Schema version for future migrations.
    #[serde(default = "default_schema")]
    pub schema: u32,
    /// Installed servers keyed by catalogue id.
    #[serde(default)]
    pub servers: BTreeMap<String, ManifestEntry>,
}

fn default_schema() -> u32 {
    1
}

impl Manifest {
    /// Load from disk, or return an empty manifest if the file is missing.
    pub fn load() -> Result<Self> {
        let path = manifest_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let m: Manifest = serde_json::from_str(&raw)
            .with_context(|| format!("parse {}", path.display()))?;
        Ok(m)
    }

    /// Atomic-ish write: write to `.tmp` then rename.
    pub fn save(&self) -> Result<()> {
        let dir = lsp_dir()?;
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let path = manifest_path()?;
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(self).context("serialize lsp manifest")?;
        std::fs::write(&tmp, raw.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ManifestEntry> {
        self.servers.get(id)
    }

    pub fn upsert(&mut self, entry: ManifestEntry) {
        self.servers.insert(entry.id.clone(), entry);
    }

    pub fn remove(&mut self, id: &str) -> Option<ManifestEntry> {
        self.servers.remove(id)
    }
}

/// Resolve the absolute path of a managed binary for `id`, if the install looks
/// complete. Prefers `binary_rel` from the manifest; falls back to
/// `bin/<binary>` and bare `<binary>` under the server dir.
pub fn managed_binary_path(id: &str, binary: &str) -> Option<PathBuf> {
    let dir = server_dir(id).ok()?;
    if !dir.is_dir() {
        return None;
    }
    if let Ok(m) = Manifest::load() {
        if let Some(entry) = m.get(id) {
            if let Some(rel) = entry.binary_rel.as_deref() {
                let p = dir.join(rel);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    // Common layouts (also try .js / Windows .cmd — npm package bins).
    let names = managed_binary_names(binary);
    let mut candidates: Vec<PathBuf> = Vec::new();
    for name in &names {
        candidates.push(dir.join("bin").join(name));
        candidates.push(dir.join(name));
        candidates.push(dir.join("node_modules").join(".bin").join(name));
        candidates.push(dir.join("venv").join(venv_bin_dir()).join(name));
    }
    if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    let base = binary.to_string();
    let js = format!("{base}.js");
    // Slow path: look under lib/node_modules and node_modules for the bin name.
    for root in [
        dir.join("lib").join("node_modules"),
        dir.join("node_modules"),
    ] {
        if !root.is_dir() {
            continue;
        }
        if let Some(found) = find_file_named(&root, &base).or_else(|| find_file_named(&root, &js)) {
            return Some(found);
        }
    }
    None
}

fn find_file_named(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_dir() {
                // Skip bulky trees we never need for bin lookup.
                if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                    if n == "node_modules" && dir != root {
                        // Still descend top-level package node_modules? No —
                        // package bins live in the package's own bin/, not deps.
                        continue;
                    }
                }
                stack.push(p);
            } else if p.is_file() && p.file_name().and_then(|s| s.to_str()) == Some(name) {
                return Some(p);
            }
        }
    }
    None
}

/// Candidate basenames for a managed binary on this platform.
fn managed_binary_names(binary: &str) -> Vec<String> {
    let mut names = vec![binary.to_string()];
    if !binary.ends_with(".js") {
        names.push(format!("{binary}.js"));
    }
    #[cfg(windows)]
    {
        if !binary.ends_with(".cmd") {
            names.push(format!("{binary}.cmd"));
        }
        if !binary.ends_with(".exe") {
            names.push(format!("{binary}.exe"));
        }
        if !binary.ends_with(".bat") {
            names.push(format!("{binary}.bat"));
        }
    }
    names
}

#[cfg(windows)]
fn venv_bin_dir() -> &'static str {
    "Scripts"
}

#[cfg(not(windows))]
fn venv_bin_dir() -> &'static str {
    "bin"
}

/// Current unix epoch seconds (best-effort; 0 on clock failure).
pub fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ensure `path` is executable on Unix (no-op on Windows).
pub fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?;
        let mut perms = meta.permissions();
        let mode = perms.mode();
        // u+rwx, g+rx, o+rx
        perms.set_mode(mode | 0o755);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("chmod +x {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip_empty() {
        let m = Manifest::default();
        let raw = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.servers.len(), 0);
    }

    #[test]
    fn manifest_upsert_remove() {
        let mut m = Manifest::default();
        m.upsert(ManifestEntry {
            id: "taplo".into(),
            version: "0.10.0".into(),
            source: "github".into(),
            installed_at: 1,
            binary_rel: Some("bin/taplo".into()),
        });
        assert!(m.get("taplo").is_some());
        assert!(m.remove("taplo").is_some());
        assert!(m.get("taplo").is_none());
    }
}

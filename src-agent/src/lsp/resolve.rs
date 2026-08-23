//! Resolve a language server to managed install, PATH, or missing.
//!
//! Resolution order (matches the plan):
//! 1. koma-managed binary under `~/.koma/lsp/<id>/`
//! 2. same basename on `PATH`
//! 3. missing → Monarch-only + banner

use std::path::PathBuf;
use std::process::Command;

use super::catalog::{self, ServerSpec};
use super::manifest::{self, Manifest};

/// Where a resolved server binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Installed under `~/.koma/lsp/<id>/`.
    Managed,
    /// Found on the process PATH.
    Path,
    /// Not found anywhere.
    Missing,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Managed => "managed",
            Source::Path => "path",
            Source::Missing => "missing",
        }
    }
}

/// Fully resolved status for one catalogue entry.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub id: String,
    pub name: String,
    pub binary: String,
    pub source: Source,
    /// Absolute path to the executable when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Version string when known (manifest, or `--version` probe).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub extensions: Vec<String>,
    /// Install recipe kind label for Settings (`github` / `npm` / `pip` / `go`).
    pub install_kind: String,
    pub package: String,
}

/// Resolve every first-wave server.
pub fn status_all() -> Vec<ServerStatus> {
    let manifest = Manifest::load().unwrap_or_default();
    catalog::CATALOG
        .iter()
        .map(|spec| resolve_one(spec, &manifest))
        .collect()
}

/// Resolve one server by catalogue id. Returns `None` if id is unknown.
pub fn status_one(id: &str) -> Option<ServerStatus> {
    let spec = catalog::find(id)?;
    let manifest = Manifest::load().unwrap_or_default();
    Some(resolve_one(spec, &manifest))
}

/// Resolve the server that owns `ext` (no leading dot), if any.
#[allow(dead_code)]
pub fn status_for_extension(ext: &str) -> Option<ServerStatus> {
    let spec = catalog::find_by_extension(ext)?;
    let manifest = Manifest::load().unwrap_or_default();
    Some(resolve_one(spec, &manifest))
}

fn resolve_one(spec: &ServerSpec, manifest: &Manifest) -> ServerStatus {
    let install_kind = match spec.kind {
        catalog::InstallKind::GithubGz | catalog::InstallKind::GithubZip => "github",
        catalog::InstallKind::Npm => "npm",
        catalog::InstallKind::PipVenv => "pip",
        catalog::InstallKind::GoInstall => "go",
    }
    .to_string();

    let extensions = spec
        .extensions
        .iter()
        .map(|e| (*e).to_string())
        .collect::<Vec<_>>();

    // 1. Managed.
    if let Some(path) = manifest::managed_binary_path(spec.id, spec.binary) {
        let version = manifest
            .get(spec.id)
            .map(|e| e.version.clone())
            .or_else(|| probe_version(&path));
        return ServerStatus {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            binary: spec.binary.to_string(),
            source: Source::Managed,
            path: Some(path.display().to_string()),
            version,
            extensions,
            install_kind,
            package: spec.package.to_string(),
        };
    }

    // 2. PATH.
    if let Some(path) = find_on_path(spec.binary) {
        let version = probe_version(&path);
        return ServerStatus {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            binary: spec.binary.to_string(),
            source: Source::Path,
            path: Some(path.display().to_string()),
            version,
            extensions,
            install_kind,
            package: spec.package.to_string(),
        };
    }

    // 3. Missing.
    ServerStatus {
        id: spec.id.to_string(),
        name: spec.name.to_string(),
        binary: spec.binary.to_string(),
        source: Source::Missing,
        path: None,
        version: None,
        extensions,
        install_kind,
        package: spec.package.to_string(),
    }
}

/// Locate `name` on PATH (cross-platform). Returns the first hit.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    which(name)
}

fn which(name: &str) -> Option<PathBuf> {
    // Prefer a real `which`/`where` only as fallback — walk PATH ourselves so
    // tests and restricted envs don't depend on an external binary.
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        #[cfg(windows)]
        {
            for ext in ["", ".exe", ".cmd", ".bat"] {
                let candidate = if ext.is_empty() {
                    dir.join(name)
                } else if name.ends_with(ext) {
                    dir.join(name)
                } else {
                    dir.join(format!("{name}{ext}"))
                };
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        #[cfg(not(windows))]
        {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Best-effort `--version` / `version` probe. Returns the first non-empty line
/// of stdout/stderr, truncated. Never panics; network-free; short timeout via
/// process completion (caller is already off the UI thread for install).
fn probe_version(bin: &std::path::Path) -> Option<String> {
    for args in [["--version"].as_slice(), ["version"].as_slice(), ["-V"].as_slice()] {
        let output = Command::new(bin).args(args.iter().copied()).output().ok()?;
        let text = if !output.stdout.is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            String::from_utf8_lossy(&output.stderr)
        };
        let line = text.lines().next().unwrap_or("").trim();
        if !line.is_empty() {
            // Cap length so a chatty binary can't flood Settings.
            let capped: String = line.chars().take(120).collect();
            return Some(capped);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_all_covers_catalog() {
        let rows = status_all();
        assert_eq!(rows.len(), catalog::CATALOG.len());
        for (row, spec) in rows.iter().zip(catalog::CATALOG.iter()) {
            assert_eq!(row.id, spec.id);
        }
    }

    #[test]
    fn missing_server_has_no_path() {
        // A nonsense binary name won't be on PATH; craft via catalog entry that
        // is extremely unlikely to be installed in CI. We just assert the
        // shape for whatever resolve returns for taplo if missing.
        let row = status_one("taplo").expect("taplo in catalog");
        assert_eq!(row.id, "taplo");
        match row.source {
            Source::Missing => {
                assert!(row.path.is_none());
            }
            Source::Managed | Source::Path => {
                assert!(row.path.is_some());
            }
        }
    }
}

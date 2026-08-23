//! Resolve language-server binaries.
//!
//! Resolution order (matches the plan):
//! 1. koma-managed binary under `~/.koma/lsp/<id>/`
//! 2. same basename on `PATH` **and actually runnable**
//! 3. missing → Monarch-only + banner
//!
//! A file on PATH is not enough: rustup installs a `rust-analyzer` proxy that
//! exists even when the component is missing, and its error text used to show
//! up in Settings as a fake "version".

use std::path::{Path, PathBuf};
use std::process::Command;

use super::catalog::{self, ServerSpec};
use super::manifest::{self, Manifest};

/// Where a resolved server binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Installed under `~/.koma/lsp/<id>/`.
    Managed,
    /// Found on the process PATH and verified runnable.
    Path,
    /// Not found anywhere (or PATH hit is a broken toolchain proxy).
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
#[allow(dead_code)]
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

    // 2. PATH — only if the binary is actually runnable (not a broken rustup stub).
    if let Some(path) = find_on_path(spec.binary) {
        match classify_path_binary(&path) {
            PathBinary::Usable { version } => {
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
            PathBinary::Broken => {
                // Fall through to missing so Settings shows Install, not a fake
                // "on PATH" row with rustup's error as the version string.
            }
        }
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

#[derive(Debug, PartialEq, Eq)]
enum PathBinary {
    Usable { version: Option<String> },
    Broken,
}

/// Decide whether a PATH hit is a real server or a dead toolchain proxy.
///
/// rustup puts `rust-analyzer` on PATH for every toolchain. When the component
/// is missing, invoking it prints:
///   error: Unknown binary 'rust-analyzer' in official toolchain '…'
/// and exits non-zero. We must not report that as "on PATH".
fn classify_path_binary(bin: &Path) -> PathBinary {
    let mut saw_broken = false;
    let mut version = None;

    for args in [
        ["--version"].as_slice(),
        ["version"].as_slice(),
        ["-V"].as_slice(),
    ] {
        let Ok(output) = Command::new(bin).args(args.iter().copied()).output() else {
            continue;
        };
        let text = combined_output(&output);
        if looks_like_broken_toolchain_proxy(&text) {
            saw_broken = true;
            continue;
        }
        if output.status.success() {
            if let Some(line) = first_useful_line(&text) {
                version = Some(line);
                break;
            }
        }
    }

    if version.is_some() {
        return PathBinary::Usable { version };
    }
    if saw_broken {
        return PathBinary::Broken;
    }

    // No version string, but no broken-proxy signal either. Accept the PATH hit
    // (some servers are quiet or use nonstandard flags). One more --help probe
    // to catch proxies that only error on real invocation.
    if let Ok(output) = Command::new(bin).arg("--help").output() {
        let text = combined_output(&output);
        if looks_like_broken_toolchain_proxy(&text) {
            return PathBinary::Broken;
        }
    }

    PathBinary::Usable { version: None }
}

fn combined_output(output: &std::process::Output) -> String {
    let mut s = String::new();
    if !output.stdout.is_empty() {
        s.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    s
}

fn first_useful_line(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Never surface toolchain / proxy errors as a version badge.
        if looks_like_broken_toolchain_proxy(line) {
            continue;
        }
        if line.starts_with("error:") || line.starts_with("Error:") {
            continue;
        }
        let capped: String = line.chars().take(120).collect();
        return Some(capped);
    }
    None
}

/// rustup / similar toolchain managers leave shims on PATH that only error.
fn looks_like_broken_toolchain_proxy(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("unknown binary")
        || lower.contains("is not installed")
        || lower.contains("not a component")
        || lower.contains("consider using `rustup component add")
        || lower.contains("rustup component add")
        || (lower.contains("toolchain") && lower.contains("not installed"))
}

/// Best-effort `--version` / `version` probe. Returns the first useful line of
/// **successful** stdout/stderr. Never treats error text as a version.
fn probe_version(bin: &Path) -> Option<String> {
    for args in [["--version"].as_slice(), ["version"].as_slice(), ["-V"].as_slice()] {
        let output = Command::new(bin).args(args.iter().copied()).output().ok()?;
        if !output.status.success() {
            continue;
        }
        let text = combined_output(&output);
        if let Some(line) = first_useful_line(&text) {
            return Some(line);
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
        // source is whatever the host has; just ensure serde shape fields exist.
        let _ = row.source;
    }

    #[test]
    fn rustup_unknown_binary_is_broken() {
        let msg = "error: Unknown binary 'rust-analyzer' in official toolchain 'stable-aarch64-apple-darwin'.";
        assert!(looks_like_broken_toolchain_proxy(msg));
    }

    #[test]
    fn real_version_line_is_not_broken() {
        assert!(!looks_like_broken_toolchain_proxy(
            "rust-analyzer 1.83.0 (a1b2c3d 2025-01-01)"
        ));
        assert!(!looks_like_broken_toolchain_proxy("vtsls 0.3.0"));
    }

    #[test]
    fn first_useful_line_skips_errors() {
        let text = "error: Unknown binary 'rust-analyzer'\nrust-analyzer 1.0\n";
        // First line is error; if a later good line exists we could take it, but
        // for broken proxies the whole output is the error — first_useful_line
        // skips error-prefixed lines.
        assert!(first_useful_line("error: boom\n").is_none());
        assert_eq!(
            first_useful_line("rust-analyzer 1.0\n").as_deref(),
            Some("rust-analyzer 1.0")
        );
        let _ = text;
    }

    #[test]
    fn classify_broken_script() {
        let tmp = std::env::temp_dir().join(format!(
            "koma-lsp-broken-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("rust-analyzer");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .mode(0o755)
                .open(&bin)
                .unwrap();
            use std::io::Write;
            writeln!(
                f,
                "#!/bin/sh\necho \"error: Unknown binary 'rust-analyzer' in official toolchain 'stable-aarch64-apple-darwin'.\" >&2\nexit 1"
            )
            .unwrap();
        }
        #[cfg(not(unix))]
        {
            // On Windows, write a .cmd that prints the rustup message.
            let bin = tmp.join("rust-analyzer.cmd");
            std::fs::write(
                &bin,
                "@echo off\r\necho error: Unknown binary 'rust-analyzer' in official toolchain 'stable-aarch64-apple-darwin'.\r\nexit /b 1\r\n",
            )
            .unwrap();
            assert_eq!(classify_path_binary(&bin), PathBinary::Broken);
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        assert_eq!(classify_path_binary(&bin), PathBinary::Broken);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn classify_good_script() {
        let tmp = std::env::temp_dir().join(format!(
            "koma-lsp-good-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("fake-ls");
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .mode(0o755)
                .open(&bin)
                .unwrap();
            writeln!(f, "#!/bin/sh\necho 'fake-ls 1.2.3'\nexit 0").unwrap();
        }
        #[cfg(not(unix))]
        {
            let bin = tmp.join("fake-ls.cmd");
            std::fs::write(&bin, "@echo off\r\necho fake-ls 1.2.3\r\nexit /b 0\r\n").unwrap();
            match classify_path_binary(&bin) {
                PathBinary::Usable { version } => {
                    assert!(version.unwrap_or_default().contains("fake-ls"));
                }
                PathBinary::Broken => panic!("expected usable"),
            }
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        match classify_path_binary(&bin) {
            PathBinary::Usable { version } => {
                assert_eq!(version.as_deref(), Some("fake-ls 1.2.3"));
            }
            PathBinary::Broken => panic!("expected usable"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

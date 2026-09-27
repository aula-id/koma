//! Curated standalone coding components. No extension host is loaded.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn tools_dir() -> Result<PathBuf> {
    Ok(crate::model::store::base_dir()?.join("coding/tools"))
}
pub(crate) fn component_binary(id: &str) -> Option<PathBuf> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let base = tools_dir().ok()?.join(id);
    let record: Value =
        serde_json::from_slice(&std::fs::read(base.join("current.json")).ok()?).ok()?;
    let path = base.join(record.get("binary")?.as_str()?);
    if path.is_file()
        && path
            .canonicalize()
            .ok()?
            .starts_with(base.canonicalize().ok()?)
    {
        Some(path)
    } else {
        None
    }
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .https_only(true)
        .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(300))
        .build()?)
}
fn download(url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = client()?.get(url).send()?.error_for_status()?;
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= limit),
        "Component download exceeds size limit"
    );
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "Component download exceeds size limit"
    );
    Ok(bytes)
}
fn json_url(url: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&download(url, 4 * 1024 * 1024)?)?)
}
fn safe_name(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}
fn extract(bytes: &[u8], name: &str, dest: &Path) -> Result<()> {
    let mut total = 0u64;
    if name.ends_with(".zip") || name.ends_with(".vsix") {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        anyhow::ensure!(archive.len() <= 50000, "Too many archive entries");
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let path = entry.enclosed_name().context("Unsafe archive path")?;
            anyhow::ensure!(safe_name(&path), "Unsafe archive path");
            anyhow::ensure!(
                entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
                "Archive symlinks are not accepted"
            );
            total += entry.size();
            anyhow::ensure!(
                total <= 1024 * 1024 * 1024,
                "Extracted component exceeds 1 GiB"
            );
            let target = dest.join(path);
            if entry.is_dir() {
                std::fs::create_dir_all(&target)?;
                continue;
            }
            std::fs::create_dir_all(target.parent().unwrap())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            std::io::copy(&mut entry, &mut file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
                }
            }
        }
    } else {
        let reader: Box<dyn Read + '_> = if name.ends_with(".tar.gz") {
            Box::new(flate2::read::GzDecoder::new(bytes))
        } else if name.ends_with(".tar.xz") {
            Box::new(xz2::read::XzDecoder::new(bytes))
        } else {
            anyhow::bail!("Unsupported component archive: {name}")
        };
        let mut archive = tar::Archive::new(reader);
        for (index, entry) in archive.entries()?.enumerate() {
            anyhow::ensure!(index < 50000, "Too many archive entries");
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            anyhow::ensure!(safe_name(&path), "Unsafe archive path");
            let kind = entry.header().entry_type();
            anyhow::ensure!(
                kind.is_file() || kind.is_dir(),
                "Archive links and devices are not accepted"
            );
            total += entry.size();
            anyhow::ensure!(
                total <= 1024 * 1024 * 1024,
                "Extracted component exceeds 1 GiB"
            );
            anyhow::ensure!(entry.unpack_in(dest)?, "Archive entry escaped destination");
        }
    }
    Ok(())
}
fn find_file(root: &Path, name: &str) -> Result<PathBuf> {
    for entry in ignore::WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .max_depth(Some(16))
        .build()
        .flatten()
    {
        if entry.file_type().is_some_and(|t| t.is_file())
            && entry.file_name().to_string_lossy() == name
        {
            return Ok(entry.into_path());
        }
    }
    anyhow::bail!("Installed component does not contain {name}")
}
fn run(command: &str, args: &[String], cwd: &Path, env: &[(&str, String)]) -> Result<()> {
    let status = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (*k, v)))
        .stdin(std::process::Stdio::null())
        .status()
        .with_context(|| format!("Start {command}"))?;
    anyhow::ensure!(status.success(), "{command} failed with {status}");
    Ok(())
}
fn github_asset(id: &str, dest: &Path) -> Result<(PathBuf, String, String)> {
    let repo = match id {
        "lua-language-server" => "LuaLS/lua-language-server",
        "zls" => "zigtools/zls",
        "js-debug" => "microsoft/vscode-js-debug",
        _ => anyhow::bail!("Unknown release component"),
    };
    let release = json_url(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let matches = |name: &str| {
        if id == "js-debug" {
            return name.starts_with("js-debug-dap-") && name.ends_with(".tar.gz");
        }
        let os_name = if id == "lua-language-server" {
            match os {
                "macos" => "darwin",
                "windows" => "win32",
                other => other,
            }
        } else {
            os
        };
        let arch_name = if id == "lua-language-server" {
            match arch {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                other => other,
            }
        } else {
            arch
        };
        name.contains(os_name)
            && name.contains(arch_name)
            && !name.contains("musl")
            && (name.ends_with(".tar.gz") || name.ends_with(".tar.xz") || name.ends_with(".zip"))
    };
    let assets: Vec<_> = release["assets"]
        .as_array()
        .context("Release has no assets")?
        .iter()
        .filter(|a| a["name"].as_str().is_some_and(matches))
        .collect();
    anyhow::ensure!(
        assets.len() == 1,
        "No unambiguous {id} release for {os}/{arch}; select a compatible external executable"
    );
    let asset = assets[0];
    let name = asset["name"].as_str().unwrap();
    let url = asset["browser_download_url"]
        .as_str()
        .context("Missing release URL")?;
    println!("Downloading {repo}: {name}");
    let bytes = download(url, 256 * 1024 * 1024)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    if let Some(expected) = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
    {
        anyhow::ensure!(hash == expected, "Release digest mismatch");
    }
    extract(&bytes, name, dest)?;
    let binary = if id == "js-debug" {
        "dapDebugServer.js".to_string()
    } else {
        format!("{id}{}", if cfg!(windows) { ".exe" } else { "" })
    };
    let path = find_file(dest, &binary)?;
    if id != "js-debug" {
        crate::lsp::manifest::ensure_executable(&path)?;
    }
    Ok((
        path,
        release["tag_name"].as_str().unwrap_or("release").into(),
        hash,
    ))
}
fn vsix(id: &str, dest: &Path) -> Result<(PathBuf, String, String)> {
    let (publisher, extension, file) = match id {
        "php-debug" => ("xdebug", "php-debug", "phpDebug.js"),
        "bash-debug" => ("rogalmic", "bash-debug", "bashDebug.js"),
        "lua-debug" => ("tomblind", "local-lua-debugger-vscode", "debugAdapter.js"),
        _ => anyhow::bail!("Unknown debugger component"),
    };
    let metadata = json_url(&format!(
        "https://open-vsx.org/api/{publisher}/{extension}/latest"
    ))?;
    let url = metadata
        .pointer("/files/download")
        .and_then(Value::as_str)
        .context("Debugger package is unavailable from Open VSX")?;
    let bytes = download(url, 256 * 1024 * 1024)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    extract(&bytes, "adapter.vsix", dest)?;
    Ok((
        find_file(dest, file)?,
        metadata["version"].as_str().unwrap_or("release").into(),
        hash,
    ))
}
pub(crate) fn install(id: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(
            id,
            "lua-language-server"
                | "zls"
                | "js-debug"
                | "debugpy"
                | "delve"
                | "nil"
                | "phpactor"
                | "php-debug"
                | "bash-debug"
                | "lua-debug"
        ),
        "Unknown coding component"
    );
    let base = tools_dir()?.join(id);
    std::fs::create_dir_all(&base)?;
    let version_dir = base.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&version_dir)?;
    // The previous version stays selected until all installation steps succeed.
    let (binary, version, digest) = match id {
        "lua-language-server" | "zls" | "js-debug" => github_asset(id, &version_dir)?,
        "php-debug" | "bash-debug" | "lua-debug" => vsix(id, &version_dir)?,
        "debugpy" => {
            let python = if cfg!(windows) { "python" } else { "python3" };
            run(
                python,
                &[
                    "-m".into(),
                    "venv".into(),
                    version_dir.to_string_lossy().into(),
                ],
                &version_dir,
                &[],
            )?;
            let binary = version_dir.join(if cfg!(windows) {
                "Scripts/python.exe"
            } else {
                "bin/python"
            });
            run(
                &binary.to_string_lossy(),
                &[
                    "-m".into(),
                    "pip".into(),
                    "install".into(),
                    "--disable-pip-version-check".into(),
                    "debugpy".into(),
                    "pytest".into(),
                ],
                &version_dir,
                &[],
            )?;
            (binary, "pip-resolved".into(), String::new())
        }
        "delve" => {
            run(
                "go",
                &[
                    "install".into(),
                    "github.com/go-delve/delve/cmd/dlv@latest".into(),
                ],
                &version_dir,
                &[("GOBIN", version_dir.to_string_lossy().into())],
            )?;
            (
                version_dir.join(if cfg!(windows) { "dlv.exe" } else { "dlv" }),
                "go-resolved".into(),
                String::new(),
            )
        }
        "nil" => {
            run(
                "cargo",
                &[
                    "install".into(),
                    "--locked".into(),
                    "--git".into(),
                    "https://github.com/oxalica/nil".into(),
                    "--root".into(),
                    version_dir.to_string_lossy().into(),
                    "nil".into(),
                ],
                &version_dir,
                &[],
            )?;
            (
                version_dir.join(if cfg!(windows) {
                    "bin/nil.exe"
                } else {
                    "bin/nil"
                }),
                "cargo-resolved".into(),
                String::new(),
            )
        }
        "phpactor" => {
            run(
                "composer",
                &[
                    "create-project".into(),
                    "--no-interaction".into(),
                    "--no-dev".into(),
                    "phpactor/phpactor".into(),
                    "phpactor".into(),
                ],
                &version_dir,
                &[],
            )?;
            (
                version_dir.join("phpactor/bin/phpactor"),
                "composer-resolved".into(),
                String::new(),
            )
        }
        _ => unreachable!(),
    };
    anyhow::ensure!(
        binary.is_file(),
        "Installed component executable is missing"
    );
    let record = json!({"version":version,"binary":binary.strip_prefix(&base)?.to_string_lossy(),"sha256":digest,"installed":crate::lsp::manifest::now_epoch()});
    super::persistence::atomic_write(
        &base.join("current.json"),
        &serde_json::to_vec_pretty(&record)?,
    )?;
    println!("Installed {id}: {}", binary.display());
    Ok(())
}

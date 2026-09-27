//! Project-selected executables and environment; no process-global mutation.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn language(program: &str) -> &str {
    match Path::new(program)
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or(program)
    {
        "python" | "python3" | "pytest" | "basedpyright-langserver" => "python",
        "node" | "npm" | "npx" | "vtsls" => "javascript",
        "cargo" | "rustc" | "rust-analyzer" => "rust",
        "go" | "gopls" | "dlv" => "go",
        "clang" | "clang++" | "clangd" | "gcc" | "g++" | "lldb-dap" => "cpp",
        "php" | "phpactor" | "intelephense" => "php",
        "lua" | "lua-language-server" => "lua",
        "zig" | "zls" => "zig",
        "nix" | "nil" => "nix",
        "bash" | "bash-language-server" => "bash",
        _ => "",
    }
}
pub(crate) fn variables(root: &Path, program: &str) -> Result<BTreeMap<String, String>> {
    let config = super::workspace::read_config(root)?;
    let mut env: BTreeMap<String, String> =
        serde_json::from_value(config.get("environment").cloned().unwrap_or(json!({})))?;
    let key = language(program);
    let tool = config.get("toolchains").and_then(|v| v.get(key));
    if let Some(extra) = tool.and_then(|v| v.get("environment")) {
        env.extend(serde_json::from_value::<BTreeMap<String, String>>(
            extra.clone(),
        )?);
    }
    env.entry("PATH".into())
        .or_insert_with(|| host_path().to_string_lossy().into_owned());
    let selected = tool
        .and_then(|v| v.get("executable"))
        .and_then(Value::as_str)
        .map(|v| resolve_selected(root, v))
        .or_else(|| {
            if key == "python" {
                project_python(root)
            } else {
                None
            }
        });
    if let Some(path) = selected {
        if let Some(parent) = path.parent() {
            let joined = std::env::join_paths(
                std::iter::once(parent.to_path_buf())
                    .chain(std::env::split_paths(env.get("PATH").unwrap())),
            )?;
            env.insert("PATH".into(), joined.to_string_lossy().into_owned());
            if key == "python"
                && parent
                    .parent()
                    .is_some_and(|p| p.join("pyvenv.cfg").is_file())
            {
                env.insert(
                    "VIRTUAL_ENV".into(),
                    parent.parent().unwrap().to_string_lossy().into_owned(),
                );
            }
        }
    }
    anyhow::ensure!(
        env.len() <= 200
            && env
                .iter()
                .all(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')),
        "Invalid project environment"
    );
    Ok(env)
}
fn resolve_selected(root: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else if value.contains(['/', '\\']) {
        root.join(value)
    } else {
        crate::lsp::resolve::find_on_path(value).unwrap_or_else(|| p.to_path_buf())
    }
}
pub(crate) fn executable(root: &Path, program: &str) -> Result<PathBuf> {
    let config = super::workspace::read_config(root)?;
    let key = language(program);
    // Only replace the runtime/compiler itself, never its language server.
    let is_runtime = matches!(
        program,
        "python"
            | "python3"
            | "node"
            | "go"
            | "cargo"
            | "php"
            | "lua"
            | "zig"
            | "nix"
            | "bash"
            | "clang"
    );
    if is_runtime {
        if let Some(value) = config
            .get("toolchains")
            .and_then(|v| v.get(key))
            .and_then(|v| v.get("executable"))
            .and_then(Value::as_str)
        {
            return Ok(resolve_selected(root, value));
        }
    }
    if matches!(program, "python" | "python3") {
        if let Some(path) = project_python(root) {
            return Ok(path);
        }
    }
    Ok(PathBuf::from(program))
}
pub(super) fn fingerprint(config: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(config)?)))
}
pub(super) fn select(
    root: &Path,
    language: &str,
    executable: &str,
    expected: &str,
    server: Option<&str>,
) -> Result<Value> {
    anyhow::ensure!(
        matches!(
            language,
            "python"
                | "javascript"
                | "rust"
                | "go"
                | "cpp"
                | "php"
                | "lua"
                | "zig"
                | "bash"
                | "nix"
        ),
        "Unknown toolchain language"
    );
    anyhow::ensure!(
        executable.len() <= 32768 && !executable.contains('\0'),
        "Invalid executable path"
    );
    if !executable.is_empty() {
        anyhow::ensure!(
            resolve_selected(root, executable).is_file(),
            "Executable does not exist on this host"
        );
    }
    let _guard = crate::app::runtime::client::file_ops::FILE_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("File mutation lock failed"))?;
    if let Some(server) = server {
        let spec = crate::lsp::catalog::find(server).context("Unknown language server")?;
        anyhow::ensure!(
            self::language(spec.binary) == language,
            "Language server does not match the selected toolchain"
        );
        anyhow::ensure!(
            !(cfg!(windows) && server == "phpactor"),
            "Managed Phpactor is supported on Unix hosts"
        );
    }
    let mut config = super::workspace::read_config(root)?;
    anyhow::ensure!(
        fingerprint(&config)? == expected,
        "Coding settings changed. Refresh before selecting an environment."
    );
    if config.get("toolchains").is_none() {
        config["toolchains"] = json!({});
    }
    if config["toolchains"].get(language).is_none() {
        config["toolchains"][language] = json!({});
    }
    anyhow::ensure!(
        config["toolchains"][language].is_object(),
        "Toolchain settings must be an object"
    );
    if executable.is_empty() {
        config["toolchains"][language]
            .as_object_mut()
            .unwrap()
            .remove("executable");
    } else {
        config["toolchains"][language]["executable"] = json!(executable);
    }
    if let Some(server) = server {
        config["toolchains"][language]["languageServer"] = json!(server);
    }
    let dir = root.join(".koma");
    std::fs::create_dir_all(&dir)?;
    anyhow::ensure!(
        dir.canonicalize()?.starts_with(root),
        "Coding directory is outside workspace"
    );
    let target = dir.join("coding.json");
    if target.exists() {
        anyhow::ensure!(
            target.canonicalize()?.starts_with(root),
            "Configuration is outside workspace"
        );
    }
    super::persistence::atomic_write(&target, &serde_json::to_vec_pretty(&config)?)
        .context("Save environment selection")?;
    Ok(config)
}

fn project_python(root: &Path) -> Option<PathBuf> {
    [".venv", "venv"]
        .iter()
        .map(|name| {
            root.join(name).join(if cfg!(windows) {
                "Scripts/python.exe"
            } else {
                "bin/python"
            })
        })
        .find(|path| path.is_file())
}
pub(super) fn test_python(root: &Path) -> Result<PathBuf> {
    let program = if cfg!(windows) { "python" } else { "python3" };
    let selected = executable(root, program)?;
    let config = super::workspace::read_config(root)?;
    if selected != PathBuf::from(program)
        || config["toolchains"]["python"]["executable"].is_string()
    {
        return Ok(selected);
    }
    Ok(super::provision::component_binary("debugpy").unwrap_or(selected))
}
pub(crate) fn language_server(
    root: &Path,
    extension: &str,
) -> Option<&'static crate::lsp::catalog::ServerSpec> {
    let default = crate::lsp::catalog::find_by_extension(extension)?;
    let key = language(default.binary);
    let config = super::workspace::read_config(root).ok()?;
    let id = config["toolchains"][key]["languageServer"]
        .as_str()
        .unwrap_or(default.id);
    crate::lsp::catalog::find(id).filter(|spec| {
        spec.extensions
            .iter()
            .any(|ext| ext.eq_ignore_ascii_case(extension))
    })
}
pub(crate) fn host_path() -> std::ffi::OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(not(windows))]
    {
        current
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{
            RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ,
            RRF_RT_REG_SZ,
        };
        let mut paths: Vec<PathBuf> = std::env::split_paths(&current).collect();
        for (key, name) in [
            (HKEY_CURRENT_USER, "Environment"),
            (
                HKEY_LOCAL_MACHINE,
                "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            ),
        ] {
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let value: Vec<u16> = "Path".encode_utf16().chain(Some(0)).collect();
            let mut bytes = 65536u32;
            let mut buffer = vec![0u16; 32768];
            let result = unsafe {
                RegGetValueW(
                    key,
                    name.as_ptr(),
                    value.as_ptr(),
                    RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                    std::ptr::null_mut(),
                    buffer.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            if result == 0 {
                let count = (bytes as usize / 2).min(buffer.len());
                let path = String::from_utf16_lossy(&buffer[..count])
                    .trim_end_matches('\0')
                    .to_string();
                for part in std::env::split_paths(&path) {
                    if !paths.contains(&part) {
                        paths.push(part);
                    }
                }
            }
        }
        std::env::join_paths(paths).unwrap_or(current)
    }
}

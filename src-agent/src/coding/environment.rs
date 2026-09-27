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
    if let Some(tool) = config.get("toolchains").and_then(|v| v.get(key)) {
        if let Some(extra) = tool.get("environment") {
            env.extend(serde_json::from_value::<BTreeMap<String, String>>(
                extra.clone(),
            )?);
        }
        if let Some(executable) = tool.get("executable").and_then(Value::as_str) {
            let path = resolve_selected(root, executable);
            if let Some(parent) = path.parent() {
                let base = env
                    .get("PATH")
                    .cloned()
                    .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
                let joined = std::env::join_paths(
                    std::iter::once(parent.to_path_buf()).chain(std::env::split_paths(&base)),
                )?;
                env.insert("PATH".into(), joined.to_string_lossy().into_owned());
                if key == "python"
                    && parent
                        .parent()
                        .is_some_and(|p| p.join("pyvenv.cfg").is_file())
                {
                    env.insert(
                        "VIRTUAL_ENV".into(),
                        parent
                            .parent()
                            .unwrap_or(parent)
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
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

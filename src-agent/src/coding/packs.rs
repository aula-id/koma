//! Reviewable, host-bound provisioning recipes executed by the task supervisor.
use super::WorkspaceRef;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
struct Pack {
    id: &'static str,
    label: &'static str,
    runtime: &'static str,
    lsp: &'static str,
    adapter: &'static str,
    test: &'static str,
}
const PACKS: &[Pack] = &[
    Pack {
        id: "rust",
        label: "Rust",
        runtime: "cargo",
        lsp: "rust-analyzer",
        adapter: "lldb-dap",
        test: "cargo test",
    },
    Pack {
        id: "javascript",
        label: "JavaScript / TypeScript",
        runtime: "node",
        lsp: "vtsls",
        adapter: "js-debug",
        test: "node / project scripts",
    },
    Pack {
        id: "python",
        label: "Python",
        runtime: "python3",
        lsp: "basedpyright",
        adapter: "debugpy",
        test: "pytest",
    },
    Pack {
        id: "go",
        label: "Go",
        runtime: "go",
        lsp: "gopls",
        adapter: "delve",
        test: "go test",
    },
    Pack {
        id: "cpp",
        label: "C / C++",
        runtime: "clang",
        lsp: "clangd",
        adapter: "lldb-dap",
        test: "CTest",
    },
    Pack {
        id: "php",
        label: "PHP",
        runtime: "php",
        lsp: "intelephense",
        adapter: "php-debug",
        test: "PHPUnit (project dependency)",
    },
    Pack {
        id: "lua",
        label: "Lua",
        runtime: "lua",
        lsp: "lua-language-server",
        adapter: "lua-debug",
        test: "Busted (project dependency)",
    },
    Pack {
        id: "zig",
        label: "Zig",
        runtime: "zig",
        lsp: "zls",
        adapter: "lldb-dap",
        test: "zig test",
    },
    Pack {
        id: "bash",
        label: "Bash",
        runtime: "bash",
        lsp: "bash-language-server",
        adapter: "bash-debug",
        test: "Bats (project dependency)",
    },
    Pack {
        id: "nix",
        label: "Nix",
        runtime: "nix",
        lsp: "nil",
        adapter: "",
        test: "nix flake check",
    },
    Pack {
        id: "web",
        label: "HTML / CSS / JSON",
        runtime: "node",
        lsp: "vscode-langservers",
        adapter: "",
        test: "project scripts",
    },
    Pack {
        id: "toml",
        label: "TOML",
        runtime: "",
        lsp: "taplo",
        adapter: "",
        test: "",
    },
];
fn available(id: &str) -> Option<std::path::PathBuf> {
    super::provision::component_binary(id).or_else(|| crate::lsp::resolve::find_on_path(id))
}
pub(super) fn status(root: &Path) -> Result<Value> {
    let config = super::workspace::read_config(root)?;
    let packs: Vec<Value> = PACKS.iter().map(|p| {
        let spec = crate::lsp::catalog::find(p.lsp).unwrap();
        let lsp = crate::lsp::manifest::managed_binary_path(spec.id,spec.binary).or_else(||crate::lsp::resolve::find_on_path(spec.binary));
        let selected = config["toolchains"][p.id]["executable"].as_str().unwrap_or("");
        let runtime = super::environment::executable(root,p.runtime).ok().and_then(|v| if v.is_file() {Some(v)} else {crate::lsp::resolve::find_on_path(v.to_str()?) });
        let mut candidates = Vec::new();
        if let Some(path) = &runtime { candidates.push(path.to_string_lossy().into_owned()); }
        if p.id == "python" { for name in [".venv", "venv"] { let path = root.join(name).join(if cfg!(windows) {"Scripts/python.exe"} else {"bin/python"}); if path.is_file() { candidates.push(path.to_string_lossy().into_owned()); } } }
        json!({"id":p.id,"label":p.label,"runtime":runtime,"runtimeName":p.runtime,"selected":selected,"candidates":candidates,"lsp":lsp,"server":p.lsp,"adapter":if p.adapter.is_empty(){None}else{available(p.adapter)},"adapterName":p.adapter,"test":p.test})
    }).collect();
    Ok(
        json!({"packs":packs,"fingerprint":super::environment::fingerprint(&config)?,"platform":std::env::consts::OS}),
    )
}
struct Plan {
    id: String,
    workspace: WorkspaceRef,
    pack: String,
    commands: Vec<Value>,
    at: Instant,
}
static PLANS: OnceLock<Mutex<VecDeque<Plan>>> = OnceLock::new();
fn plans() -> &'static Mutex<VecDeque<Plan>> {
    PLANS.get_or_init(Default::default)
}
fn command(command: &str, args: Vec<String>) -> Value {
    json!({"command":command,"args":args,"timeoutMs":3_600_000})
}
fn runtime_command(p: &Pack) -> Result<Value> {
    // Installation is explicit. Elevation is delegated to the platform's own UI.
    let packages: &[&str] = match p.id {
        "rust" => &["cargo","rustc","lldb"], "javascript"|"web" => &["nodejs","npm"],
        "python" => &["python3","python3-venv","python3-pip"], "go"=> &["golang-go"],
        "cpp"=> &["clang","lldb","cmake"], "php"=> &["php-cli","composer","nodejs","npm","php-xdebug"],
        "lua"=> &["lua5.4","nodejs","npm"], "bash"=> &["bash","nodejs","npm"],
        _=> anyhow::bail!("Runtime installation for {} is not available through this host's package manager. Select an existing executable.",p.label),
    };
    #[cfg(target_os = "linux")]
    if crate::lsp::resolve::find_on_path("apt-get").is_some() {
        let apt = crate::lsp::resolve::find_on_path("apt-get").unwrap();
        let mut args = vec![
            apt.to_string_lossy().into_owned(),
            "install".into(),
            "-y".into(),
        ];
        args.extend(packages.iter().map(|s| s.to_string()));
        if unsafe { libc::geteuid() } == 0 {
            return Ok(command(&args.remove(0), args));
        }
        anyhow::ensure!(crate::lsp::resolve::find_on_path("pkexec").is_some(),"Install pkexec or select a preinstalled runtime; the installer requires a desktop elevation prompt");
        return Ok(command("pkexec", args));
    }
    #[cfg(target_os = "macos")]
    if crate::lsp::resolve::find_on_path("brew").is_some() {
        let names: &[&str] = match p.id {
            "rust" => &["rust", "llvm"],
            "javascript" | "web" => &["node"],
            "python" => &["python"],
            "go" => &["go"],
            "cpp" => &["llvm", "cmake"],
            "php" => &["php", "composer", "node"],
            "lua" => &["lua", "node"],
            "bash" => &["bash", "node"],
            _ => &[],
        };
        let mut args = vec!["install".into()];
        args.extend(names.iter().map(|s| s.to_string()));
        return Ok(command("brew", args));
    }
    anyhow::bail!(
        "No supported runtime package manager on this host. Select an existing runtime executable."
    )
}
pub(super) fn plan(
    workspace: &WorkspaceRef,
    _root: &Path,
    pack_id: &str,
    runtime: bool,
) -> Result<Value> {
    let p = PACKS
        .iter()
        .find(|p| p.id == pack_id)
        .context("Unknown language pack")?;
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let mut commands = Vec::new();
    if runtime {
        commands.push(runtime_command(p)?);
    }
    let provision_lsp = matches!(p.lsp, "lua-language-server" | "zls" | "nil");
    commands.push(command(
        &exe,
        if provision_lsp {
            vec!["coding-provision".into(), p.lsp.into()]
        } else {
            vec![
                "lsp".into(),
                "install".into(),
                p.lsp.into(),
                "--force".into(),
            ]
        },
    ));
    if !p.adapter.is_empty() && p.adapter != "lldb-dap" {
        commands.push(command(
            &exe,
            vec!["coding-provision".into(), p.adapter.into()],
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let result = json!({"id":id,"label":p.label,"commands":commands,"notes": if p.adapter=="lldb-dap" {"LLVM's lldb-dap must be on PATH. Some distributions package it separately. Project test dependencies remain project-owned."} else {"Project test dependencies remain project-owned. PHP debugging requires Xdebug; Bash debugging requires bashdb."}});
    let mut registry = plans().lock().unwrap();
    registry.retain(|p| p.at.elapsed() < Duration::from_secs(600));
    while registry.len() >= 32 {
        registry.pop_front();
    }
    registry.push_back(Plan {
        id,
        workspace: workspace.clone(),
        pack: p.id.into(),
        commands,
        at: Instant::now(),
    });
    Ok(result)
}
pub(super) fn apply(workspace: &WorkspaceRef, root: &Path, plan_id: &str) -> Result<Value> {
    let mut registry = plans().lock().unwrap();
    let index = registry
        .iter()
        .position(|p| p.id == plan_id && &p.workspace == workspace)
        .context("Installation plan expired; review again")?;
    let plan = registry.remove(index).unwrap();
    drop(registry);
    anyhow::ensure!(
        plan.at.elapsed() < Duration::from_secs(600),
        "Installation plan expired; review again"
    );
    super::tasks::run_commands(
        workspace,
        root,
        &format!("pack:{}", plan.pack),
        &format!("Install / update {}", plan.pack),
        plan.commands,
    )
}

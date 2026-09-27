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
        let server = config["toolchains"][p.id]["languageServer"].as_str().unwrap_or(p.lsp);
        let spec = crate::lsp::catalog::find(server).unwrap_or_else(||crate::lsp::catalog::find(p.lsp).unwrap());
        let lsp = crate::lsp::manifest::managed_binary_path(spec.id,spec.binary).or_else(||crate::lsp::resolve::find_on_path(spec.binary));
        let selected = config["toolchains"][p.id]["executable"].as_str().unwrap_or("");
        let runtime = super::environment::executable(root,p.runtime).ok().and_then(|v| if v.is_file() {Some(v)} else {crate::lsp::resolve::find_on_path(v.to_str()?) });
        let mut candidates = Vec::new();
        if let Some(path) = &runtime { candidates.push(path.to_string_lossy().into_owned()); }
        if p.id == "python" { for name in [".venv", "venv"] { let path = root.join(name).join(if cfg!(windows) {"Scripts/python.exe"} else {"bin/python"}); if path.is_file() { candidates.push(path.to_string_lossy().into_owned()); } } }
        json!({"id":p.id,"label":p.label,"runtime":runtime,"runtimeName":p.runtime,"selected":selected,"candidates":candidates,"lsp":lsp,"server":spec.id,"serverOptions":if p.id=="php"&&!cfg!(windows){vec!["intelephense","phpactor"]}else{vec![p.lsp]},"adapter":if p.adapter.is_empty(){None}else{available(p.adapter)},"adapterName":p.adapter,"test":p.test})
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
fn runtime_commands(p: &Pack) -> Result<Vec<Value>> {
    #[cfg(target_os = "linux")]
    {
        let manager = ["apt-get", "dnf", "pacman", "zypper"]
            .into_iter()
            .find(|name| crate::lsp::resolve::find_on_path(name).is_some());
        if let Some(manager) = manager {
            let packages: &[&str] = match (manager, p.id) {
                ("apt-get", "rust") => &["cargo", "rustc", "lldb", "build-essential", "pkg-config"],
                ("apt-get", "python") => &["python3", "python3-venv", "python3-pip"],
                ("apt-get", "go") => &["golang-go"],
                ("apt-get", "php") => &[
                    "php-cli",
                    "php-mbstring",
                    "php-xml",
                    "composer",
                    "nodejs",
                    "npm",
                    "php-xdebug",
                ],
                ("apt-get", "lua") => &["lua5.4", "nodejs", "npm"],
                ("apt-get", "zig" | "nix") => anyhow::bail!(
                    "Select an existing {} runtime; no portable apt recipe is available",
                    p.label
                ),
                ("pacman", "rust") => &["rust", "lldb"],
                ("pacman", "python") => &["python", "python-pip"],
                ("pacman", "go") => &["go"],
                ("pacman", "php") => &["php", "composer", "nodejs", "npm", "xdebug"],
                ("zypper", "go") => &["go"],
                (_, "rust") => &["cargo", "rust", "lldb"],
                (_, "python") => &["python3", "python3-pip"],
                (_, "go") => &["golang"],
                (_, "php") => &["php", "composer", "nodejs", "npm"],
                (_, "lua") => &["lua", "nodejs", "npm"],
                (_, "javascript" | "web") => &["nodejs", "npm"],
                (_, "cpp") => &["clang", "lldb", "cmake"],
                (_, "bash") => &["bash", "bashdb", "nodejs", "npm"],
                (_, "zig") => &["zig"],
                _ => anyhow::bail!("Select an existing {} runtime on this host", p.label),
            };
            let binary = crate::lsp::resolve::find_on_path(manager)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let mut args: Vec<String> = match manager {
                "pacman" => vec!["-S".into(), "--needed".into(), "--noconfirm".into()],
                "zypper" => vec!["--non-interactive".into(), "install".into()],
                _ => vec!["install".into(), "-y".into()],
            };
            args.extend(packages.iter().map(|s| s.to_string()));
            if unsafe { libc::geteuid() } == 0 {
                return Ok(vec![command(&binary, args)]);
            }
            let desktop = std::env::var_os("DISPLAY").is_some()
                || std::env::var_os("WAYLAND_DISPLAY").is_some();
            args.insert(0, binary);
            if desktop && crate::lsp::resolve::find_on_path("pkexec").is_some() {
                return Ok(vec![command("pkexec", args)]);
            }
            anyhow::ensure!(
                crate::lsp::resolve::find_on_path("sudo").is_some(),
                "Install sudo or select an existing runtime"
            );
            let mut step = command("sudo", args);
            step["interactive"] = json!(true);
            return Ok(vec![step]);
        }
    }
    #[cfg(target_os = "macos")]
    if crate::lsp::resolve::find_on_path("brew").is_some() {
        let packages: &[&str] = match p.id {
            "rust" => &["rust", "llvm"],
            "javascript" | "web" => &["node"],
            "python" => &["python"],
            "go" => &["go"],
            "cpp" => &["llvm", "cmake"],
            "php" => &["php", "composer", "node"],
            "lua" => &["lua", "node"],
            "bash" => &["bash", "bashdb", "node"],
            "zig" => &["zig", "llvm"],
            _ => anyhow::bail!("Select an existing {} runtime on this host", p.label),
        };
        let mut args = vec!["install".into()];
        args.extend(packages.iter().map(|s| s.to_string()));
        return Ok(vec![command("brew", args)]);
    }
    #[cfg(windows)]
    if crate::lsp::resolve::find_on_path("winget").is_some() {
        let packages: &[&str] = match p.id {
            "rust" => &[
                "Microsoft.VisualStudio.2022.BuildTools",
                "Rustlang.Rustup",
                "LLVM.LLVM",
            ],
            "javascript" | "web" => &["OpenJS.NodeJS.LTS"],
            "python" => &["Python.Python.3.13"],
            "go" => &["GoLang.Go"],
            "cpp" => &[
                "Microsoft.VisualStudio.2022.BuildTools",
                "LLVM.LLVM",
                "Kitware.CMake",
            ],
            "php" => &["PHP.PHP.8.4", "OpenJS.NodeJS.LTS"],
            "zig" => &["zig.zig", "LLVM.LLVM"],
            _ => anyhow::bail!("Select an existing {} runtime on Windows", p.label),
        };
        return Ok(packages
            .iter()
            .map(|id| {
                let mut args = vec![
                    "install".into(),
                    "--id".into(),
                    id.to_string(),
                    "--exact".into(),
                    "--accept-package-agreements".into(),
                    "--accept-source-agreements".into(),
                ];
                if *id == "Python.Python.3.13" {
                    args.extend(["--custom".into(), "PrependPath=1".into()]);
                }
                if *id=="Microsoft.VisualStudio.2022.BuildTools" {args.extend(["--override".into(),"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended".into()]);}
                command("winget", args)
            })
            .collect());
    }
    anyhow::bail!(
        "No supported runtime package manager on this host. Select an existing executable."
    )
}
pub(super) fn plan(
    workspace: &WorkspaceRef,
    root: &Path,
    pack_id: &str,
    runtime: bool,
    server: Option<&str>,
) -> Result<Value> {
    let p = PACKS
        .iter()
        .find(|p| p.id == pack_id)
        .context("Unknown language pack")?;
    let server = server.unwrap_or(p.lsp);
    anyhow::ensure!(
        server == p.lsp || (p.id == "php" && server == "phpactor" && !cfg!(windows)),
        "Unsupported language server for this pack and host"
    );
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let mut commands = Vec::new();
    if runtime {
        commands.extend(runtime_commands(p)?);
    }
    let provision_lsp = matches!(server, "lua-language-server" | "zls" | "nil");
    commands.push(command(
        &exe,
        if provision_lsp {
            vec!["coding-provision".into(), server.into()]
        } else {
            vec![
                "lsp".into(),
                "install".into(),
                server.into(),
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
    for step in &mut commands {
        if step["command"] == exe {
            let mut env = super::environment::variables(root, p.runtime)?;
            if p.id == "zig" {
                env.insert(
                    "KOMA_ZIG_EXECUTABLE".into(),
                    super::environment::executable(root, "zig")?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            if p.id == "python" {
                env.insert(
                    "KOMA_PYTHON_EXECUTABLE".into(),
                    super::environment::executable(
                        root,
                        if cfg!(windows) { "python" } else { "python3" },
                    )?
                    .to_string_lossy()
                    .into_owned(),
                );
            }
            step["env"] = json!(env);
        }
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

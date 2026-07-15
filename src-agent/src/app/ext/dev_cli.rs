//! `koma ext install --dev <path>` — the CLI dev-sideload verb.
//!
//! Today sideloading an extension for local development means either waiting on
//! koma.run sign-in (the store path, `requests_ext.rs::install_extension`) or
//! hand-editing `~/.koma/config.json`. This gives it a real, blessed verb: point at
//! an unsigned `.zip` (from `src-extension/pack.sh`) or an already-staged directory
//! (`manifest.json` + `bin/<exec>` at its root) and it installs straight into
//! `~/.koma/extensions/<id>/`, fully offline, no koma.run account needed.
//!
//! This runs PRE-daemon, straight out of `main` (see `cli::ExtCli` / the `ext` short
//! circuit in `main.rs`), so plain terminal `println!`/`eprintln!` here is correct —
//! the "never print from runtime/daemon-owning code" rule is for TUI/daemon-owning
//! code, not this pre-TUI CLI path.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::app::ext::install;
use crate::model::app_config::AppConfig;

/// Print `koma ext` / `koma ext install` usage to stderr and return the exit code
/// `main` should use. Mirrors `print_daemon_usage`'s shape (see `app::runtime::manage`).
pub fn print_ext_usage() -> i32 {
    eprintln!("usage: koma ext install --dev <zip|dir>");
    eprintln!();
    eprintln!("  only --dev is supported from the CLI; use the in-app store otherwise.");
    1
}

/// Sideload an extension from `path` (a `.zip` file or an unpacked directory) with
/// NO signature/store/signin checks — purely local, for iterating on an extension
/// under development. Overwrites any existing install of the same id (so an
/// install -> test -> reinstall loop just works), auto-grants every manifest
/// `requires` entry (printing each one so the developer sees the surface), tags the
/// registry entry with the `"dev"` tier marker, and enables it immediately.
pub fn run_install_dev(path: &str) -> Result<()> {
    let p = Path::new(path);
    if !p.exists() {
        bail!("no such file or directory: {path}");
    }

    println!("[dev] unsigned sideload");
    let mut ext = if p.is_dir() {
        install::install_dev_dir(p)?
    } else {
        install::install_dev_zip(p)?
    };

    // Dev marker: `InstalledExtension.tier` is a raw wire string, not tied to the
    // manifest's closed `Tier` enum (which is only `free`/`paid`) — so this sidesteps
    // adding a variant there. See docs/EXTENSIONS.md's "Dev install" section.
    ext.tier = "dev".to_string();
    // Dev installs are for immediate testing.
    ext.enabled = true;

    let mut config = AppConfig::load();
    if let Some(existing) = config.installed_extensions.iter().find(|e| e.id == ext.id) {
        println!("[dev] replacing existing {} v{}", existing.id, existing.version);
    }

    for grant in &ext.granted {
        println!("[dev] granting {grant} ...");
    }

    let id = ext.id.clone();
    let version = ext.version.clone();
    config.upsert_extension(ext.clone());

    // Auto-register any manifest-declared bundled MCP servers (same as the store install
    // paths) so a dev sideload doesn't need a hand-added McpServerEntry either.
    let mcp_registered = crate::app::ext::register::register_mcp_servers(&ext, &mut config)
        .context("register manifest mcp_servers")?;
    if mcp_registered > 0 {
        println!("[dev] registered {mcp_registered} mcp server(s)");
    }

    config.save().context("save ~/.koma/config.json")?;

    println!(
        "installed {id} v{version} (dev). New sessions load it immediately; \
         restart any running koma sessions to pick it up."
    );
    Ok(())
}

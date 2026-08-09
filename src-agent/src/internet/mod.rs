//! Internet full-mode provisioner — "full" tier installer.
//!
//! The "full" internet tier upgrades the `web_fetch` tool to a browser backend:
//! a vendored Python package (`scrapion_agent`) that drives a Firefox via
//! Playwright (renders JS, beats Cloudflare). That package lives in
//! `src-internet/` (sibling of this crate) and is embedded verbatim into the
//! binary at compile time via [`include_dir!`].
//!
//! # Public surface
//!
//! | Function | Purpose |
//! |---|---|
//! | [`internet_dir`] | `~/.koma/internet/` — install root |
//! | [`venv_python`] | path to the venv Python; used as "installed" marker |
//! | [`is_installed`] | non-panicking predicate consumed by Stage 6 gating |
//! | [`install`] | provisions the environment (CLI mode, prints progress) |
//! | [`uninstall`] | removes the install root |

use anyhow::{anyhow, Context, Result};
use include_dir::{include_dir, Dir};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::model::store::base_dir;

/// Embedded snapshot of `src-internet/` baked in at compile time.
///
/// The macro path is relative to `$CARGO_MANIFEST_DIR` (i.e. `src-agent/`),
/// so `../src-internet` resolves to the sibling directory that holds the
/// vendored `scrapion_agent` package, `requirements.txt`, and metadata files.
static INTERNET_ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../src-internet");

/// Returns `~/.koma/internet/` — the root of the internet install.
pub fn internet_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("internet"))
}

/// Returns the path to the venv Python binary:
/// `~/.koma/internet/venv/bin/python`.
///
/// The presence of this file is the canonical "installed" marker used by both
/// [`is_installed`] and the Stage 6 gating logic.
pub fn venv_python() -> Result<PathBuf> {
    Ok(internet_dir()?.join("venv").join("bin").join("python"))
}

/// Non-panicking predicate: `true` iff the venv Python binary exists on disk.
///
/// Intended to be called from the TUI event loop (Stage 6 gating). Any error
/// resolving the path is treated as "not installed" rather than propagated.
pub fn is_installed() -> bool {
    venv_python().map(|p| p.exists()).unwrap_or(false)
}

/// Extract the embedded `src-internet/` assets into `dest`, overwriting
/// existing files.
///
/// Uses a hand-rolled recursive extractor because [`Dir::extract`] from
/// `include_dir 0.7` refuses to overwrite existing files (returns an `Err` if
/// a file already exists). Walking entries manually lets us call
/// [`std::fs::write`] unconditionally, which is idempotent.
fn extract_assets(dir: &Dir, dest: &PathBuf) -> Result<()> {
    use include_dir::DirEntry;

    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(sub) => {
                let sub_dest = dest.join(sub.path());
                std::fs::create_dir_all(&sub_dest)
                    .with_context(|| format!("create dir {}", sub_dest.display()))?;
                extract_assets(sub, dest)?;
            }
            DirEntry::File(f) => {
                let file_dest = dest.join(f.path());
                if let Some(parent) = file_dest.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create dir {}", parent.display()))?;
                }
                std::fs::write(&file_dest, f.contents())
                    .with_context(|| format!("write {}", file_dest.display()))?;
            }
        }
    }
    Ok(())
}

/// Provision the Python research environment.
///
/// This runs in CLI command mode (before the TUI starts), so progress is
/// printed to stdout/stderr via [`println!`] / [`eprintln!`].
///
/// # Steps
/// 1. Confirm `python3` is available.
/// 2. Early-exit if already installed and `force` is false.
/// 3. On `force`, remove the existing `venv/` so pip rebuilds it cleanly,
///    then overwrite the Python source files.
/// 4. Extract embedded assets into `internet_dir()`.
/// 5. Create the venv.
/// 6. Install Python deps from `requirements.txt`.
/// 7. Install Firefox via Playwright.
#[allow(unreachable_code)]
pub fn install(force: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = force;
        anyhow::bail!("full internet / research mode is not supported on Windows");
    }

    // Step 1: verify python3 is available.
    let py3_ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !py3_ok {
        return Err(anyhow!(
            "python3 not found — install Python 3.8+ and re-run `koma --internet-fullmode-install`"
        ));
    }

    let dest = internet_dir()?;

    // Step 2: idempotency guard.
    if dest.exists() && is_installed() && !force {
        println!(
            "internet research already installed at {} (use --force to reinstall)",
            dest.display()
        );
        return Ok(());
    }

    // Step 3: on force, remove the venv so it is rebuilt from scratch.
    // We keep the Python source files (they will be overwritten in step 4).
    if force {
        let venv = dest.join("venv");
        if venv.exists() {
            println!("removing existing venv for reinstall...");
            std::fs::remove_dir_all(&venv).with_context(|| format!("remove {}", venv.display()))?;
        }
    }

    // Step 4: extract embedded assets (creates dest if needed).
    println!("extracting internet assets to {}...", dest.display());
    std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
    extract_assets(&INTERNET_ASSETS, &dest)?;

    // Step 5: create the virtual environment.
    println!("creating Python venv...");
    let status = std::process::Command::new("python3")
        .args(["-m", "venv", dest.join("venv").to_str().unwrap_or("venv")])
        .status()
        .context("failed to launch `python3 -m venv`")?;
    if !status.success() {
        return Err(anyhow!("`python3 -m venv` exited with status {}", status));
    }

    // Step 6: install Python dependencies.
    let pip = dest.join("venv").join("bin").join("pip");
    let requirements = dest.join("requirements.txt");
    println!(
        "installing Python dependencies from {}...",
        requirements.display()
    );
    let status = std::process::Command::new(&pip)
        .args([
            "install",
            "-r",
            requirements.to_str().unwrap_or("requirements.txt"),
        ])
        .status()
        .context("failed to launch pip install")?;
    if !status.success() {
        return Err(anyhow!("pip install exited with status {}", status));
    }

    // Step 7: install Firefox for Playwright (~80 MB download).
    // Prefer the `playwright` console script; fall back to `python -m playwright`
    // if the script is not executable (some environments strip it).
    let playwright_script = dest.join("venv").join("bin").join("playwright");
    let python_bin = dest.join("venv").join("bin").join("python");

    println!("installing Firefox for Playwright (this downloads ~80 MB)...");
    let pw_status = if playwright_script.exists() {
        std::process::Command::new(&playwright_script)
            .args(["install", "firefox"])
            .status()
            .context("failed to launch playwright install")?
    } else {
        std::process::Command::new(&python_bin)
            .args(["-m", "playwright", "install", "firefox"])
            .status()
            .context("failed to launch `python -m playwright install`")?
    };
    if !pw_status.success() {
        return Err(anyhow!(
            "playwright install firefox exited with status {}",
            pw_status
        ));
    }

    println!("internet research installed at {}", dest.display());
    println!("set internet mode to `full` in /settings or via `/internet full`");
    Ok(())
}

/// Remove the internet install directory entirely.
///
/// Prints a confirmation line on success. A missing directory is not an error.
pub fn uninstall() -> Result<()> {
    let dest = internet_dir()?;
    if dest.exists() {
        std::fs::remove_dir_all(&dest).with_context(|| format!("remove {}", dest.display()))?;
        println!(
            "internet research environment removed from {}",
            dest.display()
        );
    } else {
        println!("nothing to remove (internet research was not installed)");
    }
    Ok(())
}

/// Maximum time to wait for a scrapion subprocess.
const SCRAPION_TIMEOUT_SECS: u64 = 150;

/// Run a scrapion subcommand in a dedicated OS thread and return its stdout.
///
/// Spawns `python -m scrapion_agent <args>` via the installed venv Python,
/// collects stdout, and enforces a [`SCRAPION_TIMEOUT_SECS`] timeout. The
/// subprocess runs in its own thread to avoid blocking the tokio runtime.
pub fn scrapion_run(args: &[&str]) -> Result<String, String> {
    let python = venv_python().map_err(|e| format!("internet dir error: {e}"))?;
    if !python.exists() {
        return Err(
            "internet research environment not installed — run `koma --internet-fullmode-install`"
                .into(),
        );
    }

    let mut cmd = std::process::Command::new(&python);
    cmd.arg("-m").arg("scrapion_agent");
    cmd.args(args);

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    std::thread::spawn(move || {
        let output = cmd.output();
        let _ = tx.send(output);
    });

    let timeout = Duration::from_secs(SCRAPION_TIMEOUT_SECS);
    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!(
                    "scrapion exited with status {}: {}",
                    output.status,
                    stderr.trim()
                ))
            } else {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
        }
        Ok(Err(e)) => Err(format!("failed to spawn scrapion: {e}")),
        Err(_) => Err(format!("scrapion timed out after {SCRAPION_TIMEOUT_SECS}s")),
    }
}

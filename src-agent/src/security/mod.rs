//! Security daemon provisioner — M1 installer.
//!
//! Bundles the Python security daemon (`koma_sec_daemon`) that provides
//! pwntools/requests-based security tooling. The package lives in
//! `src-security/` (sibling of this crate) and is embedded verbatim into the
//! binary at compile time via [`include_dir!`].
//!
//! # Public surface
//!
//! | Function | Purpose |
//! |---|---|
//! | [`security_dir`] | `~/.koma/security/` — install root |
//! | [`venv_python`] | path to the venv Python; used as "installed" marker |
//! | [`is_installed`] | non-panicking predicate consumed by gating logic |
//! | [`install`] | provisions the environment (CLI mode, prints progress) |

use std::path::PathBuf;
use anyhow::{anyhow, Context, Result};
use include_dir::{include_dir, Dir};

use crate::model::store::base_dir;

/// Embedded snapshot of `src-security/` baked in at compile time.
///
/// The macro path is relative to `$CARGO_MANIFEST_DIR` (i.e. `src-agent/`),
/// so `../src-security` resolves to the sibling directory that holds the
/// vendored `koma_sec_daemon` package and `requirements.txt`.
static SECURITY_ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../src-security");

/// Returns `~/.koma/security/` — the root of the security daemon install.
pub fn security_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("security"))
}

/// Returns the path to the venv Python binary:
/// `~/.koma/security/venv/bin/python`.
///
/// The presence of this file is the canonical "installed" marker.
pub fn venv_python() -> Result<PathBuf> {
    Ok(security_dir()?.join("venv").join("bin").join("python"))
}

/// Non-panicking predicate: `true` iff the venv Python binary exists on disk.
///
/// Any error resolving the path is treated as "not installed" rather than
/// propagated.
pub fn is_installed() -> bool {
    venv_python().map(|p| p.exists()).unwrap_or(false)
}

/// Extract the embedded `src-security/` assets into `dest`, overwriting
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

/// Provision the Python security daemon environment.
///
/// This runs in CLI command mode (before the TUI starts), so progress is
/// printed to stdout/stderr via [`println!`] / [`eprintln!`].
///
/// # Steps
/// 1. Confirm `python3` is available.
/// 2. Early-exit if already installed and `force` is false.
/// 3. On `force`, remove the existing `venv/` so pip rebuilds it cleanly,
///    then overwrite the Python source files.
/// 4. Extract embedded assets into `security_dir()`.
/// 5. Create the venv.
/// 6. Install Python deps from `requirements.txt`.
pub fn install(force: bool) -> Result<()> {
    // Step 1: verify python3 is available.
    let py3_ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !py3_ok {
        return Err(anyhow!(
            "python3 not found — install Python 3.8+ and re-run `koma --security-install`"
        ));
    }

    let dest = security_dir()?;

    // Step 2: idempotency guard.
    if dest.exists() && is_installed() && !force {
        println!(
            "security daemon already installed at {} (use --force to reinstall)",
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
            std::fs::remove_dir_all(&venv)
                .with_context(|| format!("remove {}", venv.display()))?;
        }
    }

    // Step 4: extract embedded assets (creates dest if needed).
    println!("extracting security assets to {}...", dest.display());
    std::fs::create_dir_all(&dest)
        .with_context(|| format!("create {}", dest.display()))?;
    extract_assets(&SECURITY_ASSETS, &dest)?;

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
    println!("installing Python dependencies from {}...", requirements.display());
    let status = std::process::Command::new(&pip)
        .args(["install", "-r", requirements.to_str().unwrap_or("requirements.txt")])
        .status()
        .context("failed to launch pip install")?;
    if !status.success() {
        return Err(anyhow!("pip install exited with status {}", status));
    }

    println!(
        "security daemon installed at {}",
        dest.display()
    );
    Ok(())
}

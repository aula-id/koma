//! `koma update` — stop the daemon then run the official installer to fetch the
//! latest release binary.
//!
//! # NO hotswap
//!
//! This does NOT swap the running process in place. It stops the daemon first
//! (so the on-disk binary is no longer held open by a running process), then
//! shells out to the installer which overwrites the binary file on disk. The
//! next `koma` launch picks up the new binary. The user is told to re-run
//! `koma` afterward.

use anyhow::{anyhow, Result};

use crate::cli::DaemonSub;

/// Stop any running daemon, then run the official installer to replace the
/// on-disk binary with the latest release. Prints progress to stdout and
/// inherits the installer's stdout/stderr so the user sees download progress.
///
/// Returns `Ok(())` on success. A non-zero installer exit or a missing
/// downloader (`curl`/`wget`) is surfaced as `Err`.
pub fn run_update() -> Result<()> {
    // 1. Stop the daemon (graceful → SIGTERM → SIGKILL) via the same public
    //    path that `koma daemon kill` uses. A "no daemon running" outcome is
    //    fine — cmd_kill prints "no daemon running" and returns Ok(()).
    println!("koma update: stopping daemon…");
    // Ignore an Err from kill (e.g. unexpected socket I/O failure): the update
    // should proceed regardless — worst case the installer overwrites the binary
    // while the daemon is still running from its in-memory image.
    let _ = super::run_daemon_subcommand(DaemonSub::Kill);

    // 2. Fetch + run the installer.
    println!("koma update: fetching latest installer…");

    // Prefer curl; fall back to wget; hard error if neither is found.
    let sh_cmd = if which("curl") {
        "curl -fsSL https://koma.run/install.sh | sh"
    } else if which("wget") {
        "wget -qO- https://koma.run/install.sh | sh"
    } else {
        return Err(anyhow!(
            "neither curl nor wget found; install one and retry"
        ));
    };

    // Inherit stdout/stderr so the installer's progress is visible in the
    // terminal. stdin is also inherited (some installers prompt for sudo).
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(sh_cmd)
        .status()
        .map_err(|e| anyhow!("failed to launch installer: {e}"))?;

    if !status.success() {
        return Err(anyhow!(
            "installer exited with status {}",
            status.code().unwrap_or(-1)
        ));
    }

    // 3. Done.
    println!("koma updated. Run 'koma' to start.");
    Ok(())
}

/// Return `true` if `name` is found on `$PATH` (best-effort — a missing `PATH`
/// or a permission error returns `false`).
fn which(name: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

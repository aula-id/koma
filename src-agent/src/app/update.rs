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

/// Windows self-update: stop the daemon, then launch the official `install.ps1`
/// via PowerShell with a two-URL fallback chain.
///
/// The script downloads the latest `koma-x64.msi` from GitHub and runs
/// `msiexec /i` with a full UI (so UAC elevation can pop). WiX's
/// `MajorUpgrade` element handles in-place upgrades.
#[cfg(windows)]
pub fn run_update() -> Result<()> {
    // 1. Stop the daemon — same logic as the unix path.
    println!("koma update: stopping daemon…");
    let _ = super::run_daemon_subcommand(DaemonSub::Kill);

    // 2. Fetch + run the PowerShell installer via two URL candidates.
    println!("koma update: fetching latest installer…");

    let ps_urls = [
        "https://koma.run/install.ps1",
        "https://raw.githubusercontent.com/aula-id/koma/main/install.ps1",
    ];

    let mut last_err = None;
    for url in &ps_urls {
        let ps_cmd = format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; irm '{}' | iex",
            url
        );
        match std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps_cmd])
            .status()
        {
            Ok(status) if status.success() => {
                super::migrate_legacy_daemon();
                println!("koma updated. Run 'koma' to start.");
                return Ok(());
            }
            Ok(status) => {
                last_err = Some(format!("installer exited with status {}", status.code().unwrap_or(-1)));
            }
            Err(e) => {
                last_err = Some(format!("failed to launch PowerShell: {e}"));
            }
        }
    }

    Err(anyhow!(
        "could not download or run the installer from any source.\n\
         {}\n\n\
         You can update manually:\n\
         1. Download koma-x64.msi from https://github.com/aula-id/koma/releases/latest\n\
         2. Run the MSI installer",
        last_err.as_deref().unwrap_or("unknown error")
    ))
}

/// Stop any running daemon, then run the official installer to replace the
/// on-disk binary with the latest release. Prints progress to stdout and
/// inherits the installer's stdout/stderr so the user sees download progress.
///
/// Returns `Ok(())` on success. A non-zero installer exit or a missing
/// downloader (`curl`/`wget`) is surfaced as `Err`.
#[cfg(not(windows))]
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
    let status = crate::tool::shell::os_shell_command(sh_cmd)
        .status()
        .map_err(|e| anyhow!("failed to launch installer: {e}"))?;

    if !status.success() {
        return Err(anyhow!(
            "installer exited with status {}",
            status.code().unwrap_or(-1)
        ));
    }

    // 3. Reap any surviving daemons — covers a pre-0.2.0 global daemon that was running
    //    alongside 0.2.0 daemons (or a first upgrade FROM 0.1.x where step 1 killed
    //    nothing). Best-effort: a reap failure must never block a successful update.
    super::migrate_legacy_daemon();

    // 4. Done.
    println!("koma updated. Run 'koma' to start.");
    Ok(())
}

/// Return `true` if `name` is found on `$PATH` (best-effort — a missing `PATH`
/// or a permission error returns `false`). Only used by the unix `run_update`
/// body above (the windows build has its own stub `run_update` and never
/// calls this), so it's gated the same way to avoid an unused-function warning.
#[cfg(not(windows))]
fn which(name: &str) -> bool {
    let cmd = format!("command -v {name}");

    crate::tool::shell::os_shell_command(&cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

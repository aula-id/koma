//! Thin stdio bridge for remote attach (`koma server`).
//!
//! SSH carries this process, not the agent. Flow:
//!
//! ```text
//! local thin-client ──SSH stdio──► koma server (bridge)
//!                                     │ ensure + dial
//!                                     ▼
//!                               remote session-daemon
//!                               ~/.koma/run/<id>.sock
//! ```
//!
//! The bridge ensures a durable `koma --daemon --session <id>` is accepting,
//! connects to its socket, and proxies the existing length-prefixed IPC frames
//! bidirectionally. It never calls [`super::lifecycle::install_daemon_session`]
//! or [`super::event_loop::daemon::daemon_loop`]. EOF closes the bridge only;
//! QuitDaemon is only forwarded when the thin client sends it.

use std::path::Path;

use anyhow::Result;

use super::stdio_bridge;

/// Entry point: ensure session-daemon + stdio↔sock frame proxy.
///
/// Requires `--session <id>` (matches the daemon's keyed socket). Optional
/// `--cwd` is the spawn workdir for a *missing* daemon only — a live daemon is
/// not chdir'd.
pub fn run_server(opts: crate::cli::Opts) -> Result<()> {
    // Ignore SIGPIPE — a broken-pipe write returns EPIPE instead of killing us.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let session_id = opts
        .session
        .clone()
        .ok_or_else(|| anyhow::anyhow!("koma server requires --session <id>"))?;
    if session_id.is_empty() || session_id.contains('\0') {
        anyhow::bail!("invalid session id");
    }

    // `--cwd` is only the spawn workdir for a *missing* daemon. A live daemon is
    // never chdir'd (`ensure_daemon_running` skips spawn when the sock accepts).
    let workdir: Option<&Path> = match opts.cwd.as_deref() {
        Some(cwd) => {
            if cwd.is_empty() || cwd.contains('\0') {
                anyhow::bail!("invalid remote working directory");
            }
            Some(Path::new(cwd))
        }
        None => None,
    };

    let rt = tokio::runtime::Runtime::new()?;
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    rt.block_on(stdio_bridge::run_stdio_sock_bridge(
        &session_id,
        workdir,
        stdin,
        stdout,
    ))
}

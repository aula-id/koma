//! Remote development over SSH (`koma remote user@host`).
//!
//! SSH-connects to a remote machine, auto-provisions koma if needed,
//! execs `koma server`, and bridges the SSH channel to a local TUI client.

pub(crate) mod auth;
pub(crate) mod bootstrap;
pub(crate) mod client;
pub(crate) mod hosts;
pub(crate) mod ssh;

use anyhow::Result;

/// Parsed remote target.
pub(crate) struct RemoteTarget {
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
    pub key: Option<String>,
}

/// Parse a target string like `user@host`, `user@host:22`, or `host`.
pub(crate) fn parse_target(target: &str) -> Result<RemoteTarget> {
    let (user_host, port) = if let Some((uh, p)) = target.rsplit_once(':') {
        let port: u16 = p.parse().map_err(|_| anyhow::anyhow!("invalid port: {p}"))?;
        (uh, Some(port))
    } else {
        (target, None)
    };

    let (user, host) = if let Some((u, h)) = user_host.split_once('@') {
        (u.to_string(), h.to_string())
    } else {
        // No user@ — use current user.
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
        (user, user_host.to_string())
    };

    Ok(RemoteTarget {
        user,
        host,
        port,
        key: None,
    })
}

/// Run a remote koma session over SSH.
#[allow(dead_code)] // superseded by client::run_remote_client_target
pub(crate) fn run_remote(target_str: &str, key: Option<&str>, port: Option<u16>) -> Result<()> {
    let mut target = parse_target(target_str)?;
    if let Some(k) = key {
        target.key = Some(k.to_string());
    }
    if let Some(p) = port {
        target.port = Some(p);
    }

    // 1. Check if koma is installed on remote.
    eprintln!("Connecting to {}@{}...", target.user, target.host);
    if !bootstrap::is_koma_installed(&target, None)? {
        eprintln!("koma not found on remote. Installing...");
        bootstrap::install_koma(&target, None)?;
        eprintln!("koma installed successfully.");
    }

    // 2. Generate a session id for the remote daemon.
    let session_id = uuid::Uuid::new_v4().to_string();

    // 3. SSH connect and exec `koma server`.
    eprintln!("Starting remote koma server (session: {session_id})...");
    let _session = ssh::connect(&target, &session_id, None)?;

    // 4. Bridge to local TUI client.
    // This is handled by the caller (main.rs routes to client::remote).
    // For now, return the session info.
    // Actually, the full bridge will be wired when Phase 1's server.rs is ready.

    eprintln!("Remote session established. Press Ctrl-D to disconnect.");

    // TODO: Bridge SSH session to local TUI
    // For now, just keep the connection alive and show output.
    // This will be completed when client/remote.rs is created.

    Ok(())
}

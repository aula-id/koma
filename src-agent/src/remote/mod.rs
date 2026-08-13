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

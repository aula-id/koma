//! Remote session discovery and lifecycle over SSH.
//!
//! - List: `koma sessions --json`
//! - Kill: `koma daemon kill --session <id>` (stops that remote session-daemon only)
//!
//! Used by the remote hub (TUI + GUI). Disconnecting from a host never calls kill —
//! only an explicit hub kill / QuitDaemon does.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A discovered remote session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSession {
    pub session_id: String,
    pub name: String,
    #[serde(default)]
    pub pwd: String,
    pub working: bool,
    #[serde(rename = "is_foreground")]
    pub is_foreground: bool,
}

/// Run `koma sessions --json` over SSH and parse the result.
///
/// The remote host must have `koma` installed (ensured by the bootstrap
/// phase). Best-effort: returns an empty list on any SSH/parse failure.
pub fn list_sessions_over_ssh(
    target: &super::RemoteTarget,
    auth: Option<&super::auth::SshAuth>,
) -> Result<Vec<DiscoveredSession>> {
    let path = super::ssh::find_koma(target, auth)?;
    let command = super::ssh::remote_command(&path, &["sessions", "--json"])?;
    let output = super::ssh::exec_remote(target, &command, auth)?;
    // Handle empty output (no sessions) gracefully.
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sessions: Vec<DiscoveredSession> = serde_json::from_str(&output)?;
    Ok(sessions)
}

/// Stop one remote session-daemon via SSH: `koma daemon kill --session <id>`.
///
/// This is the hub "kill session" primitive. It does **not** disconnect the
/// local client from the host — callers refresh the remote hub afterward.
/// Failures are returned so the caller can surface them; a missing session is
/// still `Ok` (remote kill is best-effort / already-dead).
pub fn kill_session_over_ssh(
    target: &super::RemoteTarget,
    auth: Option<&super::auth::SshAuth>,
    session_id: &str,
) -> Result<()> {
    if session_id.is_empty() || session_id.contains('\0') {
        anyhow::bail!("invalid remote session id");
    }
    let path = super::ssh::find_koma(target, auth)?;
    let command = super::ssh::remote_command(
        &path,
        &["daemon", "kill", "--session", session_id],
    )?;
    // exec_remote fails on non-zero exit. Remote kill is best-effort: treat a
    // successful SSH that printed "not running" as ok; surface real SSH errors.
    match super::ssh::exec_remote(target, &command, auth) {
        Ok(_) => Ok(()),
        Err(e) => {
            // Remote binary too old (no --session on kill) or already gone —
            // surface the error so the operator can upgrade / retry.
            Err(e)
        }
    }
}

/// Build remote-kill argv for tests / docs: `daemon kill --session <id>`.
#[cfg(test)]
pub(crate) fn kill_session_args(session_id: &str) -> Result<Vec<String>> {
    if session_id.is_empty() || session_id.contains('\0') {
        anyhow::bail!("invalid remote session id");
    }
    Ok(vec![
        "daemon".into(),
        "kill".into(),
        "--session".into(),
        session_id.into(),
    ])
}

#[cfg(test)]
#[path = "sessions_test.rs"]
mod tests;

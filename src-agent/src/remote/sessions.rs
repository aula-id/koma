//! Remote session discovery over SSH (`koma sessions --json`).
//!
//! SSHes into a remote host and runs `koma sessions --json` to discover
//! live koma sessions. Used by the GUI's remote-host panel to populate
//! the session list after connecting.

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

//! Remote session discovery and lifecycle over SSH.
//!
//! - List: `koma sessions --json` → `{ live, history }`
//! - Kill: `koma daemon kill --session <id>` (stops that remote session-daemon only)
//! - Delete: `koma daemon delete --session <id>` (on-disk history row only)
//!
//! Used by the remote hub (TUI + GUI). Disconnecting from a host never calls kill —
//! only an explicit hub kill / QuitDaemon does. History delete never touches the
//! laptop disk: it always SSHes `daemon delete` on the remote host.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A discovered remote LIVE session (daemon socket accepting).
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

/// A discovered remote on-disk session not currently live.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredHistorySession {
    pub session_id: String,
    pub name: String,
    #[serde(default)]
    pub pwd: String,
    /// Registry `updated_at` as unix seconds.
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub dir_label: String,
}

/// Full remote hub discovery payload: live daemons + on-disk history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveredSessions {
    #[serde(default)]
    pub live: Vec<DiscoveredSession>,
    #[serde(default)]
    pub history: Vec<DiscoveredHistorySession>,
}

/// Parse `koma sessions --json` output.
///
/// Steady state is the object form `{ "live": [...], "history": [...] }`.
/// Legacy remote binaries that still emit a bare live array are accepted as
/// live-only + empty history (bootstrap upgrade race).
pub fn parse_sessions_json(output: &str) -> Result<DiscoveredSessions> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(DiscoveredSessions::default());
    }
    let value: serde_json::Value = serde_json::from_str(trimmed)?;
    if value.is_array() {
        let live: Vec<DiscoveredSession> = serde_json::from_value(value)?;
        return Ok(DiscoveredSessions {
            live,
            history: Vec::new(),
        });
    }
    let sessions: DiscoveredSessions = serde_json::from_value(value)?;
    Ok(sessions)
}

/// Run `koma sessions --json` over SSH and parse the result.
///
/// The remote host must have `koma` installed (ensured by the bootstrap
/// phase). Best-effort: returns an empty list on any SSH/parse failure at the
/// call site (`unwrap_or_default`).
pub fn list_sessions_over_ssh(
    target: &super::RemoteTarget,
    auth: Option<&super::auth::SshAuth>,
) -> Result<DiscoveredSessions> {
    let path = super::ssh::find_koma(target, auth)?;
    let command = super::ssh::remote_command(&path, &["sessions", "--json"])?;
    let output = super::ssh::exec_remote(target, &command, auth)?;
    parse_sessions_json(&output)
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

/// Physically delete one remote on-disk session via SSH:
/// `koma daemon delete --session <id>`.
///
/// HISTORY-pane only. Refuses live sessions on the remote side. Missing id is
/// best-effort success (already gone). Never touches laptop disk.
pub fn delete_session_over_ssh(
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
        &["daemon", "delete", "--session", session_id],
    )?;
    match super::ssh::exec_remote(target, &command, auth) {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
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

/// Build remote-delete argv for tests / docs: `daemon delete --session <id>`.
#[cfg(test)]
pub(crate) fn delete_session_args(session_id: &str) -> Result<Vec<String>> {
    if session_id.is_empty() || session_id.contains('\0') {
        anyhow::bail!("invalid remote session id");
    }
    Ok(vec![
        "daemon".into(),
        "delete".into(),
        "--session".into(),
        session_id.into(),
    ])
}

#[cfg(test)]
#[path = "sessions_test.rs"]
mod tests;

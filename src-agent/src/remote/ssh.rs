//! SSH client via shell-out to system `ssh` binary.
//!
//! Phase 1 uses the system `ssh` for maximum compatibility.
//! Phase 2+ will use `thrussh` for a pure-Rust implementation.

use std::process::{Command as StdCommand, Stdio};

use anyhow::Result;
use tokio::io::BufReader;
use tokio::process::{Child, Command};

use super::auth::SshAuth;
use super::RemoteTarget;

/// An active SSH session with piped stdio.
pub(crate) struct SshSession {
    pub(crate) child: Child,
    pub(crate) stdin: tokio::process::ChildStdin,
    pub(crate) stdout: BufReader<tokio::process::ChildStdout>,
}

/// Connect to the remote target and exec `koma server --session <id>`.
///
/// When `auth` is `Some`, uses `SSH_ASKPASS` for password-based authentication
/// instead of `BatchMode=yes`. When `auth` is `None`, behaves identically to
/// the previous key-only implementation.
pub(crate) fn connect(
    target: &RemoteTarget,
    session_id: &str,
    auth: Option<&SshAuth>,
) -> Result<SshSession> {
    let mut cmd = Command::new("ssh");

    cmd.arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10");

    if auth.is_none() {
        // Key-only mode: BatchMode=yes prevents password prompts.
        cmd.arg("-o").arg("BatchMode=yes");
    }

    if let Some(port) = target.port {
        cmd.arg("-p").arg(port.to_string());
    }

    if let Some(ref key) = target.key {
        cmd.arg("-i").arg(key);
    }

    // Apply SSH_ASKPASS env vars when password auth is available.
    if let Some(a) = auth {
        a.apply_to_tokio_command(&mut cmd);
    }

    let remote_cmd = format!("koma server --session {session_id}");

    cmd.arg(format!("{}@{}", target.user, target.host))
        .arg(&remote_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start SSH: {e}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("SSH stdin not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("SSH stdout not piped"))?;

    Ok(SshSession {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

/// Run a quick command on the remote and return its output.
///
/// When `auth` is `Some`, uses `SSH_ASKPASS` for password-based authentication.
pub(crate) fn exec_remote(
    target: &RemoteTarget,
    command: &str,
    auth: Option<&SshAuth>,
) -> Result<String> {
    let mut cmd = StdCommand::new("ssh");

    cmd.arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10");

    if auth.is_none() {
        cmd.arg("-o").arg("BatchMode=yes");
    }

    if let Some(port) = target.port {
        cmd.arg("-p").arg(port.to_string());
    }

    if let Some(ref key) = target.key {
        cmd.arg("-i").arg(key);
    }

    // Apply SSH_ASKPASS env vars when password auth is available.
    if let Some(a) = auth {
        a.apply_to_std_command(&mut cmd);
    }

    cmd.arg(format!("{}@{}", target.user, target.host))
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| anyhow::anyhow!("SSH command failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("SSH command failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

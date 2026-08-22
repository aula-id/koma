//! SSH client via shell-out to system `ssh` binary.
//!
//! Remote argv is always `koma server --session <id> [--cwd <path>]`. That
//! process is a stdio↔sock **bridge** (see [`crate::app::runtime::server`]): it
//! ensures the remote session-daemon and proxies IPC frames. It is not the agent.

use std::process::{Command as StdCommand, Stdio};

use anyhow::Result;
use tokio::io::BufReader;
use tokio::process::{Child, Command};

use super::auth::SshAuth;
use super::RemoteTarget;

pub(crate) struct SshSession {
    pub(crate) child: Child,
    pub(crate) stdin: tokio::process::ChildStdin,
    pub(crate) stdout: BufReader<tokio::process::ChildStdout>,
}

/// Construct the remote server arguments without shell interpolation.
pub(crate) fn server_args(session_id: &str, cwd: Option<&str>) -> Result<Vec<String>> {
    if session_id.is_empty() || session_id.contains('\0') {
        anyhow::bail!("invalid remote session id");
    }
    let mut args = vec![
        "server".to_string(),
        "--session".to_string(),
        session_id.to_string(),
    ];
    if let Some(cwd) = cwd {
        if cwd.is_empty() || cwd.contains('\0') {
            anyhow::bail!("invalid remote working directory");
        }
        args.extend(["--cwd".to_string(), cwd.to_string()]);
    }
    Ok(args)
}

/// Quote one argument for the remote POSIX shell. SSH joins command arguments into a
/// shell command, so passing unquoted values would allow shell metacharacters through.
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Construct a remote command with each argument safely shell-quoted.
pub(crate) fn remote_command(program: &str, args: &[&str]) -> Result<String> {
    if program.is_empty() || program.contains('\0') || program.contains('\n') {
        anyhow::bail!("invalid remote executable path");
    }
    if !program.starts_with('/') {
        anyhow::bail!("remote executable path must be absolute");
    }
    let mut command = shell_quote(program);
    for arg in args {
        if arg.contains('\0') || arg.contains('\n') {
            anyhow::bail!("invalid remote command argument");
        }
        command.push(' ');
        command.push_str(&shell_quote(arg));
    }
    Ok(command)
}

/// Locate the installed remote Koma binary without relying on interactive shell startup files.
pub(crate) fn find_koma(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<String> {
    let output = exec_remote(
        target,
        "for p in \"$HOME/.local/bin/koma\" \"$HOME/bin/koma\" /usr/local/bin/koma /usr/bin/koma; do if [ -x \"$p\" ]; then printf '%s' \"$p\"; exit 0; fi; done; exit 1",
        auth,
    )?;
    if output.is_empty()
        || !output.starts_with('/')
        || output.contains('\0')
        || output.contains('\n')
    {
        anyhow::bail!("remote Koma executable path is invalid");
    }
    Ok(output)
}
pub(crate) fn connect(
    target: &RemoteTarget,
    session_id: &str,
    auth: Option<&SshAuth>,
    cwd: Option<&str>,
    koma_path: &str,
) -> Result<SshSession> {
    if koma_path.is_empty() || koma_path.contains('\0') || koma_path.contains('\n') {
        anyhow::bail!("invalid remote Koma executable path");
    }
    let mut cmd = Command::new("ssh");
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
    if let Some(a) = auth {
        a.apply_to_tokio_command(&mut cmd);
    }
    let remote_args = server_args(session_id, cwd)?;
    let remote_arg_refs: Vec<&str> = remote_args.iter().map(String::as_str).collect();
    let remote_command = remote_command(koma_path, &remote_arg_refs)?;
    cmd.arg(format!("{}@{}", target.user, target.host));
    cmd.arg(remote_command);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

pub(crate) fn validate_remote_path(path: &str) -> Result<&str> {
    let path = path.trim();
    if path.is_empty() || path.contains('\0') {
        anyhow::bail!("invalid remote path");
    }
    Ok(path)
}

pub(crate) fn list_dirs(
    target: &RemoteTarget,
    path: &str,
    auth: Option<&SshAuth>,
) -> Result<Vec<String>> {
    const MAX_DIRS: usize = 200;

    let path = validate_remote_path(path)?;
    let quoted = format!("'{}'", path.replace('\'', "'\\''"));
    let output = exec_remote(
        target,
        &format!(
            "test -d {quoted} && find {quoted} -xdev -mindepth 1 -maxdepth 1 -type d -print 2>/dev/null | head -n {}",
            MAX_DIRS + 1
        ),
        auth,
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .take(MAX_DIRS)
        .map(str::to_string)
        .collect())
}

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
    if let Some(a) = auth {
        a.apply_to_std_command(&mut cmd);
    }
    let output = cmd
        .arg(format!("{}@{}", target.user, target.host))
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("SSH command failed: {e}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "SSH command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
#[path = "ssh_test.rs"]
mod tests;

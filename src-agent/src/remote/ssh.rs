//! SSH client via shell-out to system `ssh` binary.
//!
//! Remote argv is always `koma server --session <id> [--cwd <path>]`. That
//! process is a stdio↔sock **bridge** (see [`crate::app::runtime::server`]): it
//! ensures the remote session-daemon and proxies IPC frames. It is not the agent.
//!
//! # Connection multiplexing (unix)
//!
//! Every `ssh` invocation for a given `user@host:port` shares one ControlMaster
//! via `ControlMaster=auto` + `ControlPath=~/.koma/ssh-mux/%C` +
//! `ControlPersist=300`. The first connection pays the handshake; hub list /
//! kill / bootstrap one-shots and the long-lived bridge reuse it.
//!
//! Call [`exit_multiplex`] only when leaving the **host** entirely (disconnect),
//! not when detaching a session back to the remote hub.

use std::path::PathBuf;
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

/// Directory + ControlPath template for OpenSSH multiplexing (`%C` = hash of
/// local/host/port/user). Unix only — Windows OpenSSH lacks reliable mux sockets.
#[cfg(unix)]
fn control_path_template() -> Result<PathBuf> {
    let dir = crate::model::store::base_dir()?.join("ssh-mux");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir.join("%C"))
}

/// OpenSSH mux options shared by every remote SSH invocation on unix.
/// Empty on windows / if the path can't be prepared (callers still work unmuxed).
pub(crate) fn multiplex_opts() -> Vec<String> {
    #[cfg(unix)]
    {
        if let Ok(path) = control_path_template() {
            return vec![
                "-o".into(),
                "ControlMaster=auto".into(),
                "-o".into(),
                format!("ControlPath={}", path.display()),
                "-o".into(),
                // Keep the master up 5 minutes after the last session exits so hub
                // refreshes and a quick re-attach skip the handshake. Host disconnect
                // calls [`exit_multiplex`] to tear it down immediately.
                "ControlPersist=300".into(),
            ];
        }
    }
    Vec::new()
}

/// Apply shared host-key / timeout / mux / port / key / auth options to a std `ssh`.
fn apply_std_ssh_base(cmd: &mut StdCommand, target: &RemoteTarget, auth: Option<&SshAuth>) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o").arg("ConnectTimeout=10");
    for opt in multiplex_opts() {
        cmd.arg(opt);
    }
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
        a.apply_to_std_command(cmd);
    }
}

/// Apply shared options to a tokio `ssh` (long-lived bridge).
fn apply_tokio_ssh_base(cmd: &mut Command, target: &RemoteTarget, auth: Option<&SshAuth>) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o").arg("ConnectTimeout=10");
    for opt in multiplex_opts() {
        cmd.arg(opt);
    }
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
        a.apply_to_tokio_command(cmd);
    }
}

/// Close the ControlMaster for `target` if one is up.
///
/// Best-effort: no master / non-unix / ssh missing → silent no-op. Call only on
/// full host disconnect, not on session detach.
pub(crate) fn exit_multiplex(target: &RemoteTarget) {
    #[cfg(unix)]
    {
        let Ok(path) = control_path_template() else {
            return;
        };
        let mut cmd = StdCommand::new("ssh");
        cmd.arg("-o")
            .arg(format!("ControlPath={}", path.display()))
            .arg("-O")
            .arg("exit");
        if let Some(port) = target.port {
            cmd.arg("-p").arg(port.to_string());
        }
        cmd.arg(format!("{}@{}", target.user, target.host));
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = cmd.status();
    }
    let _ = target; // silence unused on windows
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
    let remote_args = server_args(session_id, cwd)?;
    let remote_arg_refs: Vec<&str> = remote_args.iter().map(String::as_str).collect();
    connect_command(target, auth, koma_path, &remote_arg_refs)
}

/// Spawn a long-lived SSH child running an arbitrary remote koma argv over stdio.
///
/// Used by panel thin clients (`remote-fs`, `remote-git`, …) that are NOT the
/// session-daemon bridge. Reuses the same mux / StrictHostKeyChecking / auth base
/// as [`connect`].
pub(crate) fn connect_command(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
    koma_path: &str,
    remote_argv: &[&str],
) -> Result<SshSession> {
    if koma_path.is_empty() || koma_path.contains('\0') || koma_path.contains('\n') {
        anyhow::bail!("invalid remote Koma executable path");
    }
    let mut cmd = Command::new("ssh");
    apply_tokio_ssh_base(&mut cmd, target, auth);
    let remote_command = remote_command(koma_path, remote_argv)?;
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
    apply_std_ssh_base(&mut cmd, target, auth);
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

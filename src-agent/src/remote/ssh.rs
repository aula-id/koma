//! SSH client via shell-out to system `ssh` binary.

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

pub(crate) fn connect(
    target: &RemoteTarget,
    session_id: &str,
    auth: Option<&SshAuth>,
    cwd: Option<&str>,
) -> Result<SshSession> {
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
    cmd.arg(format!("{}@{}", target.user, target.host));
    cmd.arg("koma");
    for arg in remote_args {
        cmd.arg(arg);
    }
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
    let path = validate_remote_path(path)?;
    let quoted = format!("'{}'", path.replace('\'', "'\\''"));
    let output = exec_remote(
        target,
        &format!("test -d {quoted} && find {quoted} -mindepth 1 -maxdepth 1 -type d -print"),
        auth,
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|p| !p.is_empty())
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
mod tests {
    use super::{server_args, validate_remote_path};
    #[test]
    fn server_args_keep_cwd_as_one_argument() {
        let args = server_args("id", Some("/tmp/a; echo bad")).unwrap();
        assert_eq!(
            args,
            ["server", "--session", "id", "--cwd", "/tmp/a; echo bad"]
        );
    }
    #[test]
    fn server_args_reject_empty_values() {
        assert!(server_args("", None).is_err());
        assert!(server_args("id", Some("")).is_err());
    }

    #[test]
    fn remote_paths_are_trimmed_and_reject_empty_or_nul() {
        assert_eq!(validate_remote_path(" /srv/work ").unwrap(), "/srv/work");
        assert!(validate_remote_path(" ").is_err());
        assert!(validate_remote_path("/srv/\0work").is_err());
    }
}

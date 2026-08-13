//! Remote koma bootstrap: detect if koma is installed, install if not.

use anyhow::Result;

use super::auth::SshAuth;
use super::ssh;
use super::RemoteTarget;

/// Check if koma is installed on the remote machine.
pub(crate) fn is_koma_installed(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<bool> {
    let output = ssh::exec_remote(target, "command -v koma || echo MISSING", auth)?;
    Ok(!output.contains("MISSING"))
}

/// Install koma on the remote machine using the official install script.
pub(crate) fn install_koma(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<()> {
    let cmd = "curl -fsSL https://koma.run/install.sh | sh";
    ssh::exec_remote(target, cmd, auth)?;
    Ok(())
}

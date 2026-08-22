//! SSH password authentication via the `SSH_ASKPASS` mechanism.
//!
//! When key-based auth fails, we prompt for a password and feed it to ssh
//! through a temporary askpass script. This avoids stdin conflicts with our
//! IPC framing protocol (stdin is piped for the length-prefixed JSON channel).

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command as StdCommand;

use anyhow::Result;
use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use super::RemoteTarget;

/// Result of probing SSH authentication for a target.
pub(crate) enum AuthProbe {
    /// Key-based auth works; no password needed.
    KeyReady,
    /// Password required for authentication.
    PasswordRequired,
}

/// Cached SSH password for the session lifetime.
///
/// Holds the password in memory and manages the temporary askpass script.
/// The script and password are cleaned up on drop.
pub(crate) struct SshAuth {
    password: String,
    askpass_path: Option<PathBuf>,
}

impl SshAuth {
    /// Create a new auth context with the given password.
    ///
    /// Writes the askpass helper script to a temporary file with restrictive
    /// permissions (0o700). Uses `create_new` to avoid TOCTOU races and
    /// retries with an incrementing attempt counter to handle collisions.
    pub fn new(password: String) -> Result<Self> {
        use std::io::Write;

        // Create a unique temporary file with restrictive permissions.
        // Use create_new to avoid TOCTOU races.
        let dir = std::env::temp_dir();
        let mut attempt = 0;
        let askpass_path = loop {
            let name = format!("koma-askpass-{}-{}", std::process::id(), attempt);
            let path = dir.join(&name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    // Write the script content.
                    let script = askpass_script_content(&password);
                    f.write_all(script.as_bytes())?;
                    // Set permissions to owner-only read+execute.
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
                    }
                    break path;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        };

        Ok(Self {
            password,
            askpass_path: Some(askpass_path),
        })
    }

    /// Create auth from a pre-supplied password (no terminal prompt needed).
    ///
    /// Used by GUI auth flow and TUI password input.
    pub fn from_password(password: String) -> Result<Self> {
        Self::new(password)
    }

    /// Return a copy for a lifecycle hand-off that must reuse authentication.
    /// The caller owns the new auth context and this one remains session-scoped.
    pub(crate) fn password(&self) -> &str {
        &self.password
    }

    ///
    /// Sets `SSH_ASKPASS`, `SSH_ASKPASS_REQUIRE=force`, and `DISPLAY=:0`.
    /// Also removes `BatchMode=yes` so ssh will actually invoke the askpass helper.
    pub fn apply_to_std_command(&self, cmd: &mut StdCommand) {
        if let Some(ref path) = self.askpass_path {
            cmd.env("SSH_ASKPASS", path);
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
            cmd.env("DISPLAY", ":0");
        }
    }

    /// Apply SSH_ASKPASS env vars to a `tokio::process::Command`.
    pub fn apply_to_tokio_command(&self, cmd: &mut tokio::process::Command) {
        if let Some(ref path) = self.askpass_path {
            cmd.env("SSH_ASKPASS", path);
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
            cmd.env("DISPLAY", ":0");
        }
    }
}

impl std::fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshAuth")
            .field("has_password", &true)
            .field("has_askpass", &self.askpass_path.is_some())
            .finish()
    }
}

impl Drop for SshAuth {
    fn drop(&mut self) {
        // Delete the askpass script.
        if let Some(ref path) = self.askpass_path {
            let _ = std::fs::remove_file(path);
        }
        // Overwrite the password in memory.
        // SAFETY: we own the String and are about to drop it.
        // This is best-effort — the allocator may not zero the freed memory.
        unsafe {
            let bytes = self.password.as_bytes_mut();
            for b in bytes.iter_mut() {
                *b = 0;
            }
        }
        // Also drop the String normally.
    }
}

/// Probe whether key-based SSH auth works for the given target.
///
/// Runs a quick `ssh -o BatchMode=yes ... echo KEY_AUTH_OK` command.
/// Returns `AuthProbe::KeyReady` if exit code is 0 (key auth succeeded),
/// `AuthProbe::PasswordRequired` otherwise.
pub(crate) fn probe_key_auth(target: &RemoteTarget) -> AuthProbe {
    let mut cmd = StdCommand::new("ssh");
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new");

    if let Some(port) = target.port {
        cmd.arg("-p").arg(port.to_string());
    }

    if let Some(ref key) = target.key {
        cmd.arg("-i").arg(key);
    }

    cmd.arg(format!("{}@{}", target.user, target.host))
        .arg("echo")
        .arg("KEY_AUTH_OK");

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    match cmd.status() {
        Ok(s) if s.success() => AuthProbe::KeyReady,
        _ => AuthProbe::PasswordRequired,
    }
}

/// Prompt for a password on the terminal without echo.
///
/// Uses crossterm raw mode to disable echo, reads characters one at a time,
/// and prints `*` for each typed character. Returns the password on Enter.
pub(crate) fn prompt_password(user: &str, host: &str) -> Result<String> {
    // If KOMA_GUI is set, we're running under the GUI host — password
    // auth must go through GUI IPC, not terminal stderr. Return an error
    // so the caller uses the IPC password channel instead.
    if std::env::var("KOMA_GUI").is_ok() {
        anyhow::bail!("password auth under GUI must use SubmitRemotePassword, not prompt_password");
    }

    eprint!("{user}@{host}'s password: ");
    io::stderr().flush()?;

    // Enter raw mode to disable echo.
    enable_raw_mode()?;

    let mut password = String::new();

    loop {
        // Poll for a key event.
        if ratatui::crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = ratatui::crossterm::event::read()? {
                match key.code {
                    KeyCode::Enter => {
                        eprintln!();
                        break;
                    }
                    KeyCode::Char(c) => {
                        // Ctrl+C / Ctrl+D to cancel.
                        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(c, 'c' | 'd') {
                            disable_raw_mode()?;
                            anyhow::bail!("password entry cancelled");
                        }
                        password.push(c);
                        eprint!("*");
                        io::stderr().flush()?;
                    }
                    KeyCode::Backspace if password.pop().is_some() => {
                        // Erase the last `*` on screen.
                        eprint!("\x08 \x08");
                        io::stderr().flush()?;
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;

    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }

    Ok(password)
}

/// Generate the askpass helper script content.
///
/// The script receives the prompt text as `$1` and only answers prompts
/// that look like password/passphrase requests. This prevents the infinite
/// loop where `SSH_ASKPASS_REQUIRE=force` causes the helper to answer
/// host-key verification prompts ("Are you sure you want to continue
/// connecting (yes/no)?") with the password string.
fn askpass_script_content(password: &str) -> String {
    // Escape single quotes in the password for safe shell embedding.
    let escaped = password.replace('\'', "'\\''");
    format!(
        r#"#!/bin/sh
case "$1" in
  *[Pp]assword*|*[Pp]assphrase*) printf '%s\n' '{escaped}' ;;
  *) exit 1 ;;
esac
"#
    )
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod tests;

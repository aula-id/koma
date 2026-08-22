//! SSH password authentication via the `SSH_ASKPASS` mechanism.
//!
//! When key-based auth fails, we prompt for a password and feed it to ssh
//! through a temporary askpass script. This avoids stdin conflicts with our
//! IPC framing protocol (stdin is piped for the length-prefixed JSON channel).
//!
//! Interactive collection prefers an in-TUI modal (stays on the alternate
//! screen). Stored passwords are loaded from [`super::secrets`] when a
//! `host_id` is known. Stderr `prompt_password` remains only as a last-resort
//! fallback outside the TUI.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command as StdCommand;

use anyhow::Result;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use super::RemoteTarget;

/// Result of probing SSH authentication for a target.
pub(crate) enum AuthProbe {
    /// Key-based auth works; no password needed.
    KeyReady,
    /// Password required for authentication.
    PasswordRequired,
}

/// How to collect a password when key auth fails and nothing is stored.
#[derive(Debug, Clone, Copy)]
pub(crate) enum InteractivePassword {
    /// Ratatui modal on the alternate screen (preferred for TUI `/remote`).
    TuiModal,
    /// Stderr no-echo prompt (legacy / non-TUI fallback).
    #[allow(dead_code)]
    StderrPrompt,
    /// Do not prompt — return an error if a password is required.
    None,
}

/// Outcome of [`resolve_ssh_auth`]: optional askpass context plus whether the
/// password came from the encrypted store (so callers can invalidate on fail).
pub(crate) struct ResolvedAuth {
    pub auth: Option<SshAuth>,
    /// Password string when auth is Some — for persistence after success.
    pub password: Option<String>,
    pub from_store: bool,
    pub host_id: Option<String>,
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

/// Resolve SSH auth for a target: probe key, try encrypted store, then interactive.
///
/// - `prefilled`: skip probe/store and use this password (GUI already collected it).
/// - `host_id`: used for store lookup/save attribution; may be resolved from address.
/// - Call **before** dropping the TUI `TerminalGuard` when using [`InteractivePassword::TuiModal`].
pub(crate) fn resolve_ssh_auth(
    target: &RemoteTarget,
    host_id: Option<&str>,
    prefilled: Option<&str>,
    interactive: InteractivePassword,
) -> Result<ResolvedAuth> {
    let host_id = host_id
        .map(str::to_string)
        .or_else(|| super::secrets::host_id_for_address(&target.user, &target.host, target.port));

    if let Some(password) = prefilled {
        if password.is_empty() {
            anyhow::bail!("password must not be empty");
        }
        let auth = SshAuth::new(password.to_string())?;
        return Ok(ResolvedAuth {
            password: Some(password.to_string()),
            auth: Some(auth),
            from_store: false,
            host_id,
        });
    }

    match probe_key_auth(target) {
        AuthProbe::KeyReady => {
            return Ok(ResolvedAuth {
                auth: None,
                password: None,
                from_store: false,
                host_id,
            });
        }
        AuthProbe::PasswordRequired => {}
    }

    // Try encrypted store.
    if let Some(ref id) = host_id {
        if let Some(password) = super::secrets::get_remote_password(id) {
            let auth = SshAuth::new(password.clone())?;
            return Ok(ResolvedAuth {
                auth: Some(auth),
                password: Some(password),
                from_store: true,
                host_id,
            });
        }
    }

    let password = match interactive {
        InteractivePassword::TuiModal => {
            if std::env::var("KOMA_GUI").is_ok() {
                anyhow::bail!(
                    "password auth under GUI must use SubmitRemotePassword, not TUI modal"
                );
            }
            prompt_password_tui(&target.user, &target.host)?
        }
        InteractivePassword::StderrPrompt => prompt_password(&target.user, &target.host)?,
        InteractivePassword::None => {
            anyhow::bail!(
                "password required for {}@{} but no interactive prompt available",
                target.user,
                target.host
            );
        }
    };

    let auth = SshAuth::new(password.clone())?;
    Ok(ResolvedAuth {
        auth: Some(auth),
        password: Some(password),
        from_store: false,
        host_id,
    })
}

/// Persist password after a successful remote connect (no-op if no host_id/password).
pub(crate) fn remember_password(resolved: &ResolvedAuth) {
    let (Some(host_id), Some(password)) =
        (resolved.host_id.as_deref(), resolved.password.as_deref())
    else {
        return;
    };
    let _ = super::secrets::set_remote_password(host_id, password);
}

/// Drop a bad stored password so the next attempt re-prompts.
pub(crate) fn forget_stored_password(resolved: &ResolvedAuth) {
    if !resolved.from_store {
        return;
    }
    if let Some(host_id) = resolved.host_id.as_deref() {
        let _ = super::secrets::delete_remote_password(host_id);
    }
}

/// In-TUI password modal. Prefer calling while the outer TUI still holds
/// `TerminalGuard` (raw mode already on) so we do not LeaveAlternateScreen.
pub(crate) fn prompt_password_tui(user: &str, host: &str) -> Result<String> {
    use std::time::Duration;

    use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};
    use ratatui::{backend::CrosstermBackend, Terminal};

    use crate::app::runtime::terminal::TerminalGuard;

    // Only take a guard if raw mode is not already on — avoids dropping the
    // caller's alt-screen when this modal is used mid-handoff before drop(guard).
    let already_raw = ratatui::crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    let _guard = if already_raw {
        None
    } else {
        Some(TerminalGuard::enter()?)
    };

    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    // Don't clear the whole alt-screen if we joined an existing TUI — just draw overlay.
    if !already_raw {
        terminal.clear()?;
    }

    let palette = crate::view::theme::palette(&crate::model::app_config::AppConfig::load());
    let mut password = String::new();
    let mut status = format!("password for {user}@{host}");
    let title = format!("{user}@{host}");

    let result = loop {
        terminal.draw(|frame| {
            let area = frame.area();
            crate::view::clear_and_fill(frame, area, palette.bg);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(35),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(area);

            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "remote authentication",
                    Style::default()
                        .fg(palette.fg)
                        .bg(palette.bg)
                        .add_modifier(Modifier::BOLD),
                )))
                .alignment(Alignment::Center),
                chunks[1],
            );

            let box_w = (area.width.saturating_sub(10)).clamp(24, 48);
            let box_x = area.x + (area.width.saturating_sub(box_w)) / 2;
            let input_area = Rect {
                x: box_x,
                y: chunks[2].y,
                width: box_w,
                height: 3,
            };
            frame.render_widget(Clear, input_area);
            let masked: String = std::iter::repeat_n('•', password.chars().count()).collect();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.accent).bg(palette.bg))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default().fg(palette.dim).bg(palette.bg),
                ))
                .style(Style::default().bg(palette.bg));
            let inner = block.inner(input_area);
            frame.render_widget(block, input_area);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    if masked.is_empty() {
                        " ".to_string()
                    } else {
                        masked
                    },
                    Style::default().fg(palette.fg).bg(palette.bg),
                )),
                inner,
            );

            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    status.clone(),
                    Style::default().fg(palette.dim).bg(palette.bg),
                )))
                .alignment(Alignment::Center),
                chunks[3],
            );
        })?;

        if !ratatui::crossterm::event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = ratatui::crossterm::event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Enter => {
                if password.is_empty() {
                    status = "password must not be empty".into();
                    continue;
                }
                break Ok(password);
            }
            KeyCode::Esc => {
                break Err(anyhow::anyhow!("password entry cancelled"));
            }
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(c, 'c' | 'd') =>
            {
                break Err(anyhow::anyhow!("password entry cancelled"));
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                password.push(c);
                status = format!("password for {user}@{host}");
            }
            KeyCode::Backspace => {
                password.pop();
            }
            _ => {}
        }
    };

    // When we didn't own the guard, leave the caller's terminal as-is.
    drop(terminal);
    result
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

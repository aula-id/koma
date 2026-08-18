//! Remote client: connects to a remote `koma server` and runs the local TUI.

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::auth::{self, AuthProbe, SshAuth};
use super::{bootstrap, ssh, RemoteTarget};

/// Cloneable remote handoff state. `SshAuth` is reconstructed only when an
/// SSH/bootstrap operation is about to start because it owns temporary askpass state.
#[derive(Clone)]
pub(crate) struct RemoteContext {
    pub(crate) target: RemoteTarget,
    pub(crate) key_hint: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) cwd: Option<String>,
}

use crate::app::runtime::client::remote::connect_remote;
use crate::app::runtime::terminal::TerminalGuard;

/// Outcome of a remote session — did the user want to resume or fully exit?
pub(crate) enum RemoteExit {
    /// The user exited the remote session completely (e.g. `/quit`).
    Exit,
    /// The user opened the swapper inside the remote session (`/resume`).
    Resume {
        /// Target and authentication retained for remote reattachment.
        context: RemoteContext,
    },
    /// The remote daemon requested a distinct new session (`/new`).
    /// The caller reconnects using the same target and authentication context.
    NewSession {
        /// Whether the old remote daemon should be terminated.
        kill: bool,
    },
}
fn session_id_for(requested: Option<&str>) -> String {
    requested
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Run a remote session, optionally reusing an existing remote session id.
pub(crate) fn run_remote_client_with_cwd(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
    requested_session_id: Option<&str>,
    cwd: Option<&str>,
) -> Result<RemoteExit> {
    let session_id = session_id_for(requested_session_id);

    let rt = tokio::runtime::Runtime::new()?;

    // SSH connect and exec `koma server`; tokio::process::Command::spawn
    // requires an entered runtime even though connect itself is synchronous.
    let koma_path = ssh::find_koma(target, auth)?;
    let mut session = {
        let _rt_ctx = rt.enter();
        ssh::connect(target, &session_id, auth, cwd, &koma_path)?
    };

    let handle = rt.handle().clone();

    // Connect the client bridge over SSH stdin/stdout.
    let connection = connect_remote(
        &handle,
        session.stdout,
        session.stdin,
        target.host.clone(),
        session_id.clone(),
    )?;

    // Touch last_connected for the matching host (best-effort).
    {
        let mut hosts = crate::remote::hosts::load_hosts();
        let target_str = match target.port {
            Some(22) | None => format!("{}@{}", target.user, target.host),
            Some(port) => format!("{}@{}:{}", target.user, target.host, port),
        };
        if let Some(host) = hosts.hosts.iter_mut().find(|h| h.address() == target_str) {
            host.touch_last_connected();
            let _ = crate::remote::hosts::save_hosts(&hosts);
        }
    }

    // Set up terminal for TUI rendering.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the same render loop a local thin-client uses.
    let transition =
        crate::app::runtime::client::run_remote_render_loop(&mut terminal, connection, &handle)?;

    // Map the render-loop transition to a RemoteExit.
    let outcome = match transition {
        crate::app::runtime::client::ClientTransition::OpenSwapper => RemoteExit::Resume {
            context: RemoteContext {
                target: target.clone(),
                key_hint: target.key.clone(),
                password: auth.map(|a| a.password().to_string()),
                session_id: Some(session_id.clone()),
                cwd: cwd.map(str::to_string),
            },
        },
        crate::app::runtime::client::ClientTransition::NewSession { kill } => {
            // `run_remote_render_loop` queues QuitDaemon before it tears down the
            // bridge, so this outcome is only the lifecycle result to the caller.
            RemoteExit::NewSession { kill }
        }
        _ => RemoteExit::Exit,
    };

    // Clean up the SSH child process — always kill on exit (the session is
    // ephemeral to the remote client; the daemon owns the real lifecycle).
    let _ = rt.block_on(async { session.child.kill().await });

    Ok(outcome)
}

pub(crate) fn prompt_remote_cwd(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
) -> Result<Option<String>> {
    use std::sync::mpsc;
    use std::time::Duration;

    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

    const LIST_TIMEOUT: Duration = Duration::from_secs(4);

    fn load_dirs(target: &RemoteTarget, path: &str, password: Option<&str>) -> Result<Vec<String>> {
        let target = target.clone();
        let path = path.to_string();
        let password = password.map(str::to_string);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let auth = password
                .map(SshAuth::new)
                .transpose()
                .map_err(anyhow::Error::from);
            let result = match auth {
                Ok(auth) => ssh::list_dirs(&target, &path, auth.as_ref()),
                Err(error) => Err(error),
            };
            let _ = tx.send(result);
        });
        rx.recv_timeout(LIST_TIMEOUT)
            .map_err(|_| anyhow::anyhow!("remote directory listing timed out"))?
    }

    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let password = auth.map(|a| a.password().to_string());
    let mut path = "/".to_string();
    let mut loaded_path = path.clone();
    let mut selected = 0usize;
    let mut status = String::from("Enter opens a directory; Ctrl+Enter selects this path");
    let mut dirs = load_dirs(target, &path, password.as_deref()).unwrap_or_else(|error| {
        status = format!("Unable to list {path}: {error:#}");
        Vec::new()
    });

    loop {
        terminal.draw(|frame| {
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .split(frame.area());
            frame.render_widget(
                Paragraph::new(path.as_str()).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Remote directory"),
                ),
                areas[0],
            );
            let items = dirs.iter().enumerate().map(|(index, dir)| {
                let label = dir.strip_prefix(&format!("{path}/")).unwrap_or(dir);
                ListItem::new(format!(
                    "{}{}",
                    if index == selected { "> " } else { "  " },
                    label
                ))
            });
            frame.render_widget(
                List::new(items)
                    .block(Block::default().borders(Borders::ALL).title("Directories"))
                    .highlight_symbol("> "),
                areas[1],
            );
            frame.render_widget(Paragraph::new(status.as_str()), areas[2]);
        })?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Up | KeyCode::Char('k') => {
                selected = selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                selected = selected.saturating_add(1).min(dirs.len().saturating_sub(1));
            }
            KeyCode::Backspace => {
                if path != "/" {
                    path.pop();
                    if path.is_empty() {
                        path.push('/');
                    }
                    selected = 0;
                    match load_dirs(target, &path, password.as_deref()) {
                        Ok(next) => {
                            dirs = next;
                            loaded_path = path.clone();
                            status = String::from(
                                "Enter opens a directory; Ctrl+Enter selects this path",
                            );
                        }
                        Err(error) => status = format!("Unable to list {path}: {error:#}"),
                    }
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if c == '/' && path.ends_with('/') {
                    continue;
                }
                if path == "/" {
                    path.push(c);
                } else {
                    path.push(c);
                }
                selected = 0;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if path == loaded_path {
                    return Ok(Some(path));
                }
                status = String::from("Press Enter to inspect this path before selecting it");
            }
            KeyCode::Char('s') if path == loaded_path => return Ok(Some(path)),
            KeyCode::Enter => {
                if path != loaded_path {
                    selected = 0;
                    match load_dirs(target, &path, password.as_deref()) {
                        Ok(next_dirs) => {
                            dirs = next_dirs;
                            loaded_path = path.clone();
                            status = String::from(
                                "Enter opens a directory; Ctrl+Enter selects this path",
                            );
                        }
                        Err(error) => status = format!("Unable to list {path}: {error:#}"),
                    }
                } else if let Some(next) = dirs.get(selected).cloned() {
                    path = next;
                    selected = 0;
                    match load_dirs(target, &path, password.as_deref()) {
                        Ok(next_dirs) => {
                            dirs = next_dirs;
                            loaded_path = path.clone();
                            status = String::from(
                                "Enter opens a directory; Ctrl+Enter selects this path",
                            );
                        }
                        Err(error) => status = format!("Unable to list {path}: {error:#}"),
                    }
                } else {
                    status = String::from("No directory selected; Ctrl+Enter selects this path");
                }
            }
            _ => {}
        }
    }
}

///
/// Auth flow mirrors VS Code Remote-SSH:
/// 1. Try key-based auth first (fast, silent).
/// 2. If that fails, prompt for password and use SSH_ASKPASS.
/// 3. The password is cached for the session (bootstrap + connect).
pub(crate) fn run_remote_client_target(
    target_str: &str,
    key: Option<&str>,
    port: Option<u16>,
    new_session: bool,
    session_id: Option<&str>,
) -> Result<RemoteExit> {
    let mut target = super::parse_target(target_str)?;
    if let Some(k) = key {
        target.key = Some(k.to_string());
    }
    if let Some(p) = port {
        target.port = Some(p);
    }

    // Probe whether key-based auth works.
    eprintln!("Connecting to {}@{}...", target.user, target.host);
    let ssh_auth = match auth::probe_key_auth(&target) {
        AuthProbe::KeyReady => None,
        AuthProbe::PasswordRequired => {
            eprintln!("Key-based authentication failed. Password required.");
            let password = auth::prompt_password(&target.user, &target.host)?;
            Some(SshAuth::new(password)?)
        }
    };

    let auth_ref = ssh_auth.as_ref();
    let retained_password = auth_ref.map(|a| a.password().to_string());
    let cwd = if new_session {
        prompt_remote_cwd(&target, auth_ref)?
    } else {
        None
    };
    if new_session && cwd.is_none() {
        return Ok(RemoteExit::Resume {
            context: RemoteContext {
                target,
                key_hint: key.map(str::to_string),
                password: retained_password,
                session_id: None,
                cwd: None,
            },
        });
    }

    // Bootstrap: validate/install/upgrade koma on remote (uses same auth).
    eprintln!("Checking remote Koma version...");
    if bootstrap::ensure_koma_compatible(&target, auth_ref)? {
        eprintln!("Remote Koma installed or upgraded successfully.");
    } else {
        eprintln!("Remote Koma version matches.");
    }

    if !new_session && session_id.is_none() {
        return Ok(RemoteExit::Resume {
            context: RemoteContext {
                target,
                key_hint: key.map(str::to_string),
                password: retained_password,
                session_id: None,
                cwd: None,
            },
        });
    }

    let mut requested_session_id = session_id.map(str::to_string);
    loop {
        match run_remote_client_with_cwd(
            &target,
            auth_ref,
            requested_session_id.as_deref(),
            cwd.as_deref(),
        )? {
            // `/new` is a remote lifecycle operation, not an exit to the local
            // session picker. The next remote server gets a fresh id but keeps cwd/auth.
            RemoteExit::NewSession { kill } => {
                let _ = kill;
                requested_session_id = None;
            }
            outcome => return Ok(outcome),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::session_id_for;

    #[test]
    fn existing_remote_session_id_is_preserved() {
        assert_eq!(session_id_for(Some("remote-session")), "remote-session");
    }

    #[test]
    fn new_remote_session_gets_a_uuid() {
        let id = session_id_for(None);
        assert!(!id.is_empty());
        assert!(uuid::Uuid::parse_str(&id).is_ok());
    }
}

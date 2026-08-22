//! Remote client: SSH-attaches via `koma server` (stdio bridge) and runs the local TUI.
//!
//! The remote peer is a thin bridge that dials the durable session-daemon. Detach
//! and SSH drop leave the daemon cooking; QuitDaemon stops it.

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::auth::{self, SshAuth};
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

    // SSH connect and exec `koma server` (bridge); tokio::process::Command::spawn
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

    // Set up terminal for TUI rendering. If the caller already owns the
    // alt-screen (bootstrap timeline hand-off), join it instead of nesting
    // a second TerminalGuard (which would LeaveAlternateScreen on drop).
    let already_raw = ratatui::crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    let _guard = if already_raw {
        None
    } else {
        Some(TerminalGuard::enter()?)
    };
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

    // Reap the SSH *bridge* child only. The remote session-daemon keeps cooking
    // unless the client already flushed QuitDaemon (e.g. `/new kill` / hub [k]).
    // Order: connection teardown already flushed Detach/QuitDaemon before we
    // get here; wait briefly, kill only if the bridge is wedged.
    rt.block_on(async {
        crate::app::runtime::stdio_bridge::reap_bridge_child(&mut session.child).await;
    });

    // Full remote quit leaves the host — close ControlMaster. Resume/NewSession
    // stay on the host and keep the mux for hub list / next bridge.
    if matches!(outcome, RemoteExit::Exit) {
        super::ssh::exit_multiplex(target);
    }

    Ok(outcome)
}

pub(crate) fn prompt_remote_cwd(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
) -> Result<Option<String>> {
    use std::io::Stdout;
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};

    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::layout::{Alignment, Constraint, Direction, Layout};
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
    use ratatui::{backend::CrosstermBackend, Terminal};

    const LIST_TIMEOUT: Duration = Duration::from_secs(4);
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    fn load_dirs(
        target: &RemoteTarget,
        path: &str,
        password: Option<&str>,
    ) -> Receiver<Result<Vec<String>>> {
        let target = target.clone();
        let path = path.to_string();
        let password = password.map(str::to_string);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let auth = password.map(SshAuth::new).transpose();
            let result = match auth {
                Ok(auth) => ssh::list_dirs(&target, &path, auth.as_ref()),
                Err(error) => Err(error),
            };
            let _ = tx.send(result);
        });
        rx
    }

    fn wait_dirs(
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        rx: Receiver<Result<Vec<String>>>,
        path: &str,
        palette: &crate::view::theme::Palette,
        spinner: &mut u64,
    ) -> Result<Vec<String>> {
        let started = Instant::now();
        loop {
            if let Ok(result) = rx.recv_timeout(Duration::from_millis(100)) {
                return result;
            }
            if started.elapsed() >= LIST_TIMEOUT {
                anyhow::bail!("remote directory listing timed out");
            }
            terminal.draw(|frame| {
                crate::view::clear_and_fill(frame, frame.area(), palette.bg);
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(35),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(0),
                    ])
                    .split(frame.area());
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "remote directory",
                        Style::default().fg(palette.fg).bg(palette.bg),
                    )))
                    .alignment(Alignment::Center),
                    chunks[1],
                );
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            SPINNER[(*spinner % SPINNER.len() as u64) as usize],
                            Style::default().fg(palette.accent).bg(palette.bg),
                        ),
                        Span::styled(
                            format!("  listing {path}"),
                            Style::default().fg(palette.dim).bg(palette.bg),
                        ),
                    ]))
                    .alignment(Alignment::Center),
                    chunks[2],
                );
            })?;
            *spinner = spinner.wrapping_add(1);
        }
    }

    let already_raw = ratatui::crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    let _guard = if already_raw {
        None
    } else {
        Some(TerminalGuard::enter()?)
    };
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    if !already_raw {
        terminal.clear()?;
    }
    let palette = crate::view::theme::palette(&crate::model::app_config::AppConfig::load());
    let password = auth.map(|a| a.password().to_string());
    let mut spinner = 0;
    let mut path = "/".to_string();
    let mut loaded_path = path.clone();
    let mut selected = 0usize;
    // Focus toggles with Tab between the path/list (browse) and the physical
    // [Select folder] button. Enter on the button confirms cwd; Enter on the
    // list opens/lists. Replaces the old Ctrl+Enter select gesture.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Focus {
        Path,
        Select,
    }
    let mut focus = Focus::Path;
    const HINT: &str = "Tab switches · Enter opens dir · Enter on [Select folder] confirms · Esc cancels";
    let mut status = String::from(HINT);
    let mut dirs = match wait_dirs(
        &mut terminal,
        load_dirs(target, &path, password.as_deref()),
        &path,
        &palette,
        &mut spinner,
    ) {
        Ok(dirs) => dirs,
        Err(error) => {
            status = format!("Unable to list {path}: {error:#}");
            Vec::new()
        }
    };

    loop {
        terminal.draw(|frame| {
            // This picker owns the complete terminal frame.  In particular, do not leave
            // the chat/session view underneath it: that made the remote flow look like a
            // modal belonging to the wrong client.
            crate::view::clear_and_fill(frame, frame.area(), palette.bg);
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(frame.area());
            let path_focused = focus == Focus::Path;
            let select_focused = focus == Focus::Select;
            let title_style = if path_focused {
                Style::default()
                    .fg(palette.sel_fg)
                    .bg(palette.sel_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.bg)
                    .add_modifier(Modifier::BOLD)
            };
            let border_style = Style::default().fg(palette.dim).bg(palette.bg);
            let path_border = if path_focused {
                Style::default().fg(palette.accent).bg(palette.bg)
            } else {
                border_style
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" path  {path}"),
                    title_style,
                )))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if path_focused {
                            " Remote working directory (focused) "
                        } else {
                            " Remote working directory "
                        })
                        .border_style(path_border),
                ),
                areas[0],
            );
            let items = dirs.iter().enumerate().map(|(index, dir)| {
                let label = dir.strip_prefix(&format!("{path}/")).unwrap_or(dir);
                let style = if path_focused && index == selected {
                    Style::default()
                        .fg(palette.sel_fg)
                        .bg(palette.sel_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.fg).bg(palette.bg)
                };
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{}{}",
                        if path_focused && index == selected {
                            "› "
                        } else {
                            "  "
                        },
                        label
                    ),
                    style,
                )))
            });
            let list_title = if dirs.is_empty() {
                "Directories (empty)"
            } else {
                "Directories"
            };
            frame.render_widget(
                List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(list_title)
                            .border_style(border_style),
                    )
                    .highlight_style(Style::default().fg(palette.sel_fg).bg(palette.sel_bg)),
                areas[1],
            );

            // Footer: status + physical [Select folder] button (Tab target).
            let footer = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(10), Constraint::Length(20)])
                .split(areas[2]);
            let status_style = if status.starts_with("Unable") {
                Style::default().fg(palette.error).bg(palette.bg)
            } else {
                Style::default().fg(palette.dim).bg(palette.bg)
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {status}"),
                    status_style,
                )))
                .block(
                    Block::default()
                        .borders(Borders::TOP | Borders::LEFT | Borders::BOTTOM)
                        .border_style(border_style),
                ),
                footer[0],
            );
            let btn_label = if select_focused {
                "▸ [ Select folder ]"
            } else {
                "  [ Select folder ]"
            };
            let btn_style = if select_focused {
                Style::default()
                    .fg(palette.sel_fg)
                    .bg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.fg).bg(palette.bg)
            };
            let btn_border = if select_focused {
                Style::default().fg(palette.accent).bg(palette.bg)
            } else {
                border_style
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(btn_label, btn_style)))
                    .alignment(Alignment::Center)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(btn_border),
                    ),
                footer[1],
            );
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
            KeyCode::Tab => {
                focus = match focus {
                    Focus::Path => Focus::Select,
                    Focus::Select => Focus::Path,
                };
                status = String::from(HINT);
            }
            // Arrow keys always move the dir highlight (even with Select focused,
            // so users can peek without Tab-ing back). j/k only when Path-focused
            // so typing a path containing those letters still works… wait, j/k as
            // vim nav conflicts with typing — same as the old picker; keep parity.
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
            }
            KeyCode::Down => {
                selected = selected.saturating_add(1).min(dirs.len().saturating_sub(1));
            }
            KeyCode::Char('k')
                if focus == Focus::Path && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                selected = selected.saturating_sub(1);
            }
            KeyCode::Char('j')
                if focus == Focus::Path && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                selected = selected.saturating_add(1).min(dirs.len().saturating_sub(1));
            }
            KeyCode::Backspace if focus == Focus::Path => {
                if path != "/" {
                    path.pop();
                    if path.is_empty() {
                        path.push('/');
                    }
                    selected = 0;
                    match wait_dirs(
                        &mut terminal,
                        load_dirs(target, &path, password.as_deref()),
                        &path,
                        &palette,
                        &mut spinner,
                    ) {
                        Ok(next) => {
                            dirs = next;
                            loaded_path = path.clone();
                            status = String::from(HINT);
                        }
                        Err(error) => status = format!("Unable to list {path}: {error:#}"),
                    }
                }
            }
            KeyCode::Char(c)
                if focus == Focus::Path && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if c == '/' && path.ends_with('/') {
                    continue;
                }
                path.push(c);
                selected = 0;
            }
            // Enter on the Select-folder button confirms the current listed path.
            KeyCode::Enter if focus == Focus::Select => {
                if path == loaded_path {
                    return Ok(Some(path));
                }
                status = String::from("Press Enter on the path to inspect it before selecting");
                focus = Focus::Path;
            }
            // Enter on the path/list: list typed path, or open highlighted child.
            KeyCode::Enter if focus == Focus::Path => {
                if path != loaded_path {
                    selected = 0;
                    match wait_dirs(
                        &mut terminal,
                        load_dirs(target, &path, password.as_deref()),
                        &path,
                        &palette,
                        &mut spinner,
                    ) {
                        Ok(next_dirs) => {
                            dirs = next_dirs;
                            loaded_path = path.clone();
                            status = String::from(HINT);
                        }
                        Err(error) => status = format!("Unable to list {path}: {error:#}"),
                    }
                } else if let Some(next) = dirs.get(selected).cloned() {
                    path = next;
                    selected = 0;
                    match wait_dirs(
                        &mut terminal,
                        load_dirs(target, &path, password.as_deref()),
                        &path,
                        &palette,
                        &mut spinner,
                    ) {
                        Ok(next_dirs) => {
                            dirs = next_dirs;
                            loaded_path = path.clone();
                            status = String::from(HINT);
                        }
                        Err(error) => status = format!("Unable to list {path}: {error:#}"),
                    }
                } else {
                    // Empty listing — jump focus to Select folder so Enter confirms.
                    focus = Focus::Select;
                    status = String::from("No subfolders — Enter confirms this path");
                }
            }
            _ => {}
        }
    }
}

/// Run a remote thin-client session against `user@host[:port]`.
///
/// Auth flow mirrors VS Code Remote-SSH:
/// 1. Try key-based auth first (fast, silent).
/// 2. If that fails, try encrypted store, then interactive (TUI modal preferred).
/// 3. The password is cached for the session (bootstrap + connect) and
///    persisted per host_id after a successful bootstrap.
///
/// Arguments for [`run_remote_client_target`].
pub(crate) struct RemoteClientTarget<'a> {
    pub target_str: &'a str,
    pub key: Option<&'a str>,
    pub port: Option<u16>,
    pub new_session: bool,
    pub session_id: Option<&'a str>,
    pub host_id: Option<&'a str>,
    pub pre_resolved: Option<auth::ResolvedAuth>,
    pub interactive: auth::InteractivePassword,
    /// When set, bootstrap runs under a braille timeline on this terminal
    /// (caller still owns the alt-screen). When `None`, bootstrap is silent
    /// on stderr (legacy / headless).
    pub terminal: Option<&'a mut Terminal<CrosstermBackend<std::io::Stdout>>>,
}

/// Callers that still own the TUI alt-screen should resolve auth first via
/// [`auth::resolve_ssh_auth`] with [`auth::InteractivePassword::TuiModal`], then
/// pass `pre_resolved` + `terminal` so bootstrap stays on the alt-screen.
pub(crate) fn run_remote_client_target(args: RemoteClientTarget<'_>) -> Result<RemoteExit> {
    let RemoteClientTarget {
        target_str,
        key,
        port,
        new_session,
        session_id,
        host_id,
        pre_resolved,
        interactive,
        terminal,
    } = args;

    let mut target = super::parse_target(target_str)?;
    if let Some(k) = key {
        target.key = Some(k.to_string());
    }
    if let Some(p) = port {
        target.port = Some(p);
    }

    let resolved = match pre_resolved {
        Some(r) => r,
        None => auth::resolve_ssh_auth(&target, host_id, None, interactive)?,
    };
    let auth_ref = resolved.auth.as_ref();
    let retained_password = resolved.password.clone();

    let cwd = if new_session {
        match prompt_remote_cwd(&target, auth_ref) {
            Ok(c) => c,
            Err(e) => {
                auth::forget_stored_password(&resolved);
                return Err(e);
            }
        }
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
    let host_label = match target.port {
        Some(22) | None => format!("{}@{}", target.user, target.host),
        Some(port) => format!("{}@{}:{}", target.user, target.host, port),
    };
    let mut term_opt = terminal;
    let bootstrap_result = match &mut term_opt {
        Some(term) => {
            bootstrap::ensure_koma_compatible_animated(term, &target, auth_ref, &host_label)
        }
        None => {
            eprintln!("Checking remote Koma version...");
            bootstrap::ensure_koma_compatible(&target, auth_ref)
        }
    };
    match bootstrap_result {
        Ok(upgraded) => {
            if term_opt.is_none() {
                if upgraded {
                    eprintln!("Remote Koma installed or upgraded successfully.");
                } else {
                    eprintln!("Remote Koma version matches.");
                }
            }
            auth::remember_password(&resolved);
        }
        Err(e) => {
            auth::forget_stored_password(&resolved);
            // One retry with fresh interactive password if store was the source.
            if resolved.from_store {
                let retry = auth::resolve_ssh_auth(
                    &target,
                    resolved.host_id.as_deref(),
                    None,
                    interactive,
                )?;
                let retry_result = match &mut term_opt {
                    Some(term) => bootstrap::ensure_koma_compatible_animated(
                        term,
                        &target,
                        retry.auth.as_ref(),
                        &host_label,
                    ),
                    None => bootstrap::ensure_koma_compatible(&target, retry.auth.as_ref()),
                };
                match retry_result {
                    Ok(_) => {
                        auth::remember_password(&retry);
                        return finish_remote_after_auth(
                            target,
                            key,
                            retry.password.clone(),
                            retry.auth.as_ref(),
                            new_session,
                            session_id,
                            cwd,
                        );
                    }
                    Err(e2) => {
                        auth::forget_stored_password(&retry);
                        return Err(e2);
                    }
                }
            }
            return Err(e);
        }
    }

    finish_remote_after_auth(
        target,
        key,
        retained_password,
        auth_ref,
        new_session,
        session_id,
        cwd,
    )
}

fn finish_remote_after_auth(
    target: RemoteTarget,
    key: Option<&str>,
    retained_password: Option<String>,
    auth_ref: Option<&SshAuth>,
    new_session: bool,
    session_id: Option<&str>,
    cwd: Option<String>,
) -> Result<RemoteExit> {
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
            // session picker. The next remote bridge gets a fresh id but keeps
            // cwd/auth. `kill: true` already queued QuitDaemon inside
            // `run_remote_render_loop` before the previous bridge was reaped.
            RemoteExit::NewSession { kill: _ } => {
                requested_session_id = None;
            }
            outcome => return Ok(outcome),
        }
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod tests;

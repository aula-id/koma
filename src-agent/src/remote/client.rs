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
/// Run a remote koma session: SSH connect, exec server, bridge to local TUI.
pub(crate) fn run_remote_client(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
) -> Result<RemoteExit> {
    run_remote_client_with_cwd(target, auth, None)
}

pub(crate) fn run_remote_client_with_cwd(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
    cwd: Option<&str>,
) -> Result<RemoteExit> {
    // Generate session id.
    let session_id = uuid::Uuid::new_v4().to_string();

    // SSH connect and exec `koma server`.
    let mut session = ssh::connect(target, &session_id, auth, cwd)?;

    // Set up tokio runtime for the bridge.
    let rt = tokio::runtime::Runtime::new()?;
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
    use std::io::{self, Write};
    print!("Remote working directory (empty to cancel): ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let path = ssh::validate_remote_path(input)?;
    let _dirs = ssh::list_dirs(target, path, auth)?;
    Ok(Some(path.to_string()))
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

    // Bootstrap: validate/install/upgrade koma on remote (uses same auth).
    let auth_ref = ssh_auth.as_ref();
    eprintln!("Checking remote Koma version...");
    if bootstrap::ensure_koma_compatible(&target, auth_ref)? {
        eprintln!("Remote Koma installed or upgraded successfully.");
    } else {
        eprintln!("Remote Koma version matches.");
    }

    loop {
        match run_remote_client(&target, auth_ref)? {
            // `/new` is a remote lifecycle operation, not an exit to the local
            // session picker. Keep both the target and the already-established
            // authentication context and start the newly requested remote daemon.
            RemoteExit::NewSession { kill } => {
                let _ = kill;
                continue;
            }
            outcome => return Ok(outcome),
        }
    }
}

//! Remote client: connects to a remote `koma server` and runs the local TUI.

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::auth::{self, SshAuth};
use super::{bootstrap, ssh, RemoteTarget};
use crate::app::runtime::client::remote::connect_remote;
use crate::app::runtime::terminal::TerminalGuard;

/// Run a remote koma session: SSH connect, exec server, bridge to local TUI.
pub(crate) fn run_remote_client(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
) -> Result<()> {
    // Generate session id.
    let session_id = uuid::Uuid::new_v4().to_string();

    // SSH connect and exec `koma server`.
    let mut session = ssh::connect(target, &session_id, auth)?;

    // Set up tokio runtime for the bridge.
    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Connect the client bridge over SSH stdin/stdout.
    let connection = connect_remote(&handle, session.stdout, session.stdin)?;

    // Touch last_connected for the matching host (best-effort).
    {
        let mut hosts = crate::remote::hosts::load_hosts();
        let target_str = if target.port == Some(22) || target.port.is_none() {
            format!("{}@{}", target.user, target.host)
        } else {
            format!("{}@{}:{}", target.user, target.host, target.port.unwrap())
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
    let result = crate::app::runtime::client::run_remote_render_loop(
        &mut terminal,
        connection,
        &handle,
    );

    // Clean up the SSH child process on ALL paths.
    if result.is_err() {
        let _ = rt.block_on(async { session.child.kill().await });
    } else {
        let status = rt.block_on(async { session.child.wait().await })?;
        if !status.success() {
            eprintln!("Remote koma server exited with status: {status}");
        }
    }

    result
}

/// Entry point from main.rs: parse target, probe auth, and run.
///
/// Auth flow mirrors VS Code Remote-SSH:
/// 1. Try key-based auth first (fast, silent).
/// 2. If that fails, prompt for password and use SSH_ASKPASS.
/// 3. The password is cached for the session (bootstrap + connect).
pub(crate) fn run_remote_client_target(
    target_str: &str,
    key: Option<&str>,
    port: Option<u16>,
) -> Result<()> {
    let mut target = super::parse_target(target_str)?;
    if let Some(k) = key {
        target.key = Some(k.to_string());
    }
    if let Some(p) = port {
        target.port = Some(p);
    }

    // Probe whether key-based auth works.
    eprintln!("Connecting to {}@{}...", target.user, target.host);
    let ssh_auth = if auth::probe_key_auth(&target) {
        None
    } else {
        eprintln!("Key-based authentication failed. Password required.");
        let password = auth::prompt_password(&target.user, &target.host)?;
        Some(SshAuth::new(password)?)
    };

    // Bootstrap: check/install koma on remote (uses same auth).
    let auth_ref = ssh_auth.as_ref();
    if !bootstrap::is_koma_installed(&target, auth_ref)? {
        eprintln!("koma not found on remote. Installing...");
        bootstrap::install_koma(&target, auth_ref)?;
        eprintln!("koma installed successfully.");
    }

    run_remote_client(&target, auth_ref)
}

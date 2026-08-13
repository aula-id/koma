//! Remote host connect/disconnect worker for the GUI host-relay bridge.
//!
//! Handles the off-thread SSH connect sequence and password exchange
//! for the GUI's remote-host panel. The worker runs on a dedicated
//! `std::thread::spawn` so the blocking SSH/auth operations never stall
//! the 16ms push loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Shared state for an active or in-progress remote SSH session.
/// The push_loop owns this; the worker thread reads/writes it through
/// `Arc` clones.
pub(super) struct RemoteSessionShared {
    /// The worker waits here when password authentication is required. The
    /// relay sends the entered password, or drops the sender to cancel.
    pub password_tx: Mutex<Option<Sender<String>>>,
    pub cancelled: Arc<AtomicBool>,
}

impl RemoteSessionShared {
    pub fn new() -> Self {
        Self {
            password_tx: Mutex::new(None),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Event the remote connect worker sends back to the push_loop for
/// re-pushing as a `RemoteState` envelope.
pub(super) struct RemoteStateUpdate {
    pub state: String,
    pub host_id: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
    pub session_id: Option<String>,
    pub error: Option<String>,
    pub sessions: Vec<serde_json::Value>,
}

/// A successfully established remote transport, handed back to the host relay
/// so it can fold the remote daemon's frames into the GUI like a local session.
pub(super) struct ActiveRemote {
    pub connection: super::connect::Connection,
    pub ssh_child: tokio::process::Child,
}

/// Spawn the remote connect worker thread. The blocking SSH/bootstrap work stays
/// off the host relay; the resulting transport is returned over `connected_tx`.
pub(super) fn spawn_connect_worker(
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    connected_tx: Sender<ActiveRemote>,
    pw_rx: Receiver<String>,
    cancelled: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
) {
    std::thread::spawn(move || {
        remote_connect_worker(host_id, state_tx, connected_tx, pw_rx, cancelled, handle);
    });
}

/// The full remote SSH connect sequence, run on a dedicated thread.
///
/// 1. Load host by id from `~/.koma/remote-hosts.json`
/// 2. Probe key-based auth (blocking)
/// 3. If password required: push `auth_required`, wait for password via channel
/// 4. Bootstrap koma on the remote (check/install, blocking)
/// 5. SSH connect and exec `koma server` (blocking)
/// 6. Bridge the SSH channel to a Connection via `connect_remote`
/// 7. Push `connected` with the session id
/// 8. Touch `last_connected` on the host record
/// 9. Hand the live transport back to the host relay
#[allow(clippy::too_many_lines)]
fn remote_connect_worker(
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    connected_tx: Sender<ActiveRemote>,
    pw_rx: Receiver<String>,
    cancelled: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
) {
    use crate::remote::auth::{self, AuthProbe, SshAuth};
    use crate::remote::{bootstrap, ssh, RemoteTarget};

    // Helper: push a state update back to the push_loop.
    let push_state = {
        let host_id = host_id.clone();
        move |state: &str,
              user: Option<&str>,
              host: Option<&str>,
              session_id: Option<&str>,
              error: Option<&str>,
              sessions: Vec<serde_json::Value>| {
            let _ = state_tx.send(RemoteStateUpdate {
                state: state.to_string(),
                host_id: Some(host_id.clone()),
                user: user.map(str::to_string),
                host: host.map(str::to_string),
                session_id: session_id.map(str::to_string),
                error: error.map(str::to_string),
                sessions,
            });
        }
    };

    // 1. Load host from disk.
    let hosts = crate::remote::hosts::load_hosts();
    let host_data = match crate::remote::hosts::host_by_id(&hosts, &host_id) {
        Some(h) => h.clone(),
        None => {
            push_state(
                "error",
                None,
                None,
                None,
                Some("host not found"),
                Vec::new(),
            );
            return;
        }
    };

    let user_str = host_data.user.clone();
    let host_str = host_data.host.clone();
    let port = if host_data.port == 22 {
        None
    } else {
        Some(host_data.port)
    };

    let target = RemoteTarget {
        user: host_data.user.clone(),
        host: host_data.host.clone(),
        port,
        key: host_data.key_path.clone(),
    };

    push_state(
        "resolving",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );

    // 2. Probe key-based auth.
    let auth = match auth::probe_key_auth(&target) {
        AuthProbe::KeyReady => {
            // Key-based auth works — no password needed.
            None
        }
        AuthProbe::PasswordRequired => {
            // 3. Password required — signal the GUI and wait.
            push_state(
                "auth_required",
                Some(&user_str),
                Some(&host_str),
                None,
                None,
                Vec::new(),
            );

            // Block until a password arrives or the channel closes.
            match pw_rx.recv() {
                Ok(password) => match SshAuth::from_password(password) {
                    Ok(auth) => Some(auth),
                    Err(e) => {
                        push_state(
                            "error",
                            Some(&user_str),
                            Some(&host_str),
                            None,
                            Some(&format!("auth setup failed: {e:#}")),
                            Vec::new(),
                        );
                        return;
                    }
                },
                Err(_) => {
                    push_state(
                        "error",
                        Some(&user_str),
                        Some(&host_str),
                        None,
                        Some("password input cancelled"),
                        Vec::new(),
                    );
                    return;
                }
            }
        }
    };

    // 4. Bootstrap: check if koma is installed, install if not.
    push_state(
        "bootstrapping",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );

    let auth_ref = auth.as_ref();
    let installed = match bootstrap::is_koma_installed(&target, auth_ref) {
        Ok(v) => v,
        Err(e) => {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("bootstrap check failed: {e:#}")),
                Vec::new(),
            );
            return;
        }
    };

    if !installed {
        if let Err(e) = bootstrap::install_koma(&target, auth_ref) {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("koma install failed: {e:#}")),
                Vec::new(),
            );
            return;
        }
    }

    // 5. SSH connect and exec `koma server --session <id>`.
    let session_id = uuid::Uuid::new_v4().to_string();
    push_state(
        "connecting",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );

    let ssh_session = match ssh::connect(&target, &session_id, auth_ref) {
        Ok(s) => s,
        Err(e) => {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("ssh connect failed: {e:#}")),
                Vec::new(),
            );
            return;
        }
    };

    let crate::remote::ssh::SshSession {
        child: mut ssh_child,
        stdin,
        stdout,
    } = ssh_session;
    let connection = match crate::app::runtime::client::remote::connect_remote(
        &handle,
        stdout,
        stdin,
        host_str.clone(),
        session_id.clone(),
    ) {
        Ok(conn) => conn,
        Err(e) => {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("bridge failed: {e:#}")),
                Vec::new(),
            );
            let mut ssh_child = ssh_child;
            let _ = handle.block_on(async { ssh_child.kill().await });
            return;
        }
    };

    // 7. Push connected state + list existing sessions on the remote host.
    let sessions = match crate::remote::sessions::list_sessions_over_ssh(&target, auth_ref) {
        Ok(s) => s
            .iter()
            .map(|s| {
                serde_json::json!({
                    "sessionId": s.session_id,
                    "name": s.name,
                    "working": s.working,
                    "isForeground": s.is_foreground
                })
            })
            .collect(),
        Err(_) => Vec::new(), // non-fatal
    };
    push_state(
        "connected",
        Some(&user_str),
        Some(&host_str),
        Some(&session_id),
        None,
        sessions,
    );

    // 8. Touch last_connected.
    {
        let mut hosts = crate::remote::hosts::load_hosts();
        // Find the host by id and mutate in place.
        if let Some(h) = hosts.hosts.iter_mut().find(|h| h.id == host_id) {
            h.touch_last_connected();
            let _ = crate::remote::hosts::save_hosts(&hosts);
        }
    }

    // 9. Hand the established remote connection to the host relay. It now owns
    // the SSH child and folds the remote daemon's frames into normal GUI pushes.
    if cancelled.load(Ordering::Acquire) {
        let _ = handle.block_on(async { ssh_child.kill().await });
        return;
    }
    if let Err(e) = connected_tx.send(ActiveRemote {
        connection,
        ssh_child,
    }) {
        let mut active = e.0;
        let _ = handle.block_on(async { active.ssh_child.kill().await });
        push_state(
            "error",
            Some(&user_str),
            Some(&host_str),
            None,
            Some("remote session could not be opened"),
            Vec::new(),
        );
    }
}

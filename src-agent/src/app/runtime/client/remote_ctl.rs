//! Remote host connect/disconnect worker for the GUI host-relay bridge.
//!
//! Handles the off-thread SSH connect sequence and password exchange
//! for the GUI's remote-host panel. The worker runs on a dedicated
//! `std::thread::spawn` so the blocking SSH/auth operations never stall
//! the 16ms push loop.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Shared state for an active or in-progress remote SSH session.
/// The push_loop owns this; the worker thread reads/writes it through
/// `Arc` clones.
pub(super) struct RemoteSessionShared {
    /// When the worker needs a password, it waits on the receiver end.
    /// The push_loop stores the sender here so `SubmitRemotePassword`
    /// can forward it.
    pub password_tx: Mutex<Option<Sender<String>>>,
    /// Sender for disconnect signal. `DisconnectRemote` / `CancelRemoteConnect`
    /// sends through this; the worker's receiver wakes up and cleans up.
    pub disconnect_tx: Mutex<Option<Sender<()>>>,
}

impl RemoteSessionShared {
    pub fn new() -> Self {
        Self {
            password_tx: Mutex::new(None),
            disconnect_tx: Mutex::new(None),
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

/// Spawn the remote connect worker thread. Called from the push_loop's
/// `HostCtl::ConnectRemote` handler.
pub(super) fn spawn_connect_worker(
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    pw_rx: Receiver<String>,
    disconnect_rx: Receiver<()>,
    shared: Arc<RemoteSessionShared>,
) {
    std::thread::spawn(move || {
        remote_connect_worker(host_id, state_tx, pw_rx, disconnect_rx, shared);
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
/// 9. Wait for disconnect signal, then clean up
#[allow(clippy::too_many_lines)]
fn remote_connect_worker(
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    pw_rx: Receiver<String>,
    disconnect_rx: Receiver<()>,
    shared: Arc<RemoteSessionShared>,
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
                Ok(password) => {
                    // Clear the password sender from shared state — auth is done.
                    if let Ok(mut tx) = shared.password_tx.lock() {
                        *tx = None;
                    }
                    match SshAuth::from_password(password) {
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
                    }
                }
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

    let mut ssh_session = match ssh::connect(&target, &session_id, auth_ref) {
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

    // 6. Bridge the SSH channel to a Connection.
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("runtime init failed: {e:#}")),
                Vec::new(),
            );
            // Can't block_on without the runtime; just kill the child directly.
            // best-effort: the OS will reap it when the process exits.
            let _ = ssh_session.child.start_kill();
            return;
        }
    };
    let handle = rt.handle().clone();

    let connection = match crate::app::runtime::client::remote::connect_remote(
        &handle,
        ssh_session.stdout,
        ssh_session.stdin,
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
            let _ = rt.block_on(async { ssh_session.child.kill().await });
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

    // 9. Wait for disconnect signal, then clean up.
    //    For v1: the worker stays alive keeping the SSH session open.
    //    The push_loop sends a disconnect signal through the channel.
    let _ = disconnect_rx.recv();

    // Kill the SSH child.
    let _ = rt.block_on(async { ssh_session.child.kill().await });

    // Drop the connection (frame_rx/req_tx).
    drop(connection);
    drop(rt);

    push_state(
        "disconnected",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );

    // Clear the disconnect sender from shared state.
    if let Ok(mut tx) = shared.disconnect_tx.lock() {
        *tx = None;
    }
}

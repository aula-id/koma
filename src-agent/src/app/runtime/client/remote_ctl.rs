//! Remote host connect/disconnect worker for the GUI host-relay bridge.
//!
//! Blocking SSH/auth work runs on a dedicated thread. Every attempt has a fresh
//! cancellation token and monotonically increasing id so late worker results
//! cannot replace a newer transport.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

pub(super) struct RemoteSessionShared {
    password_tx: Mutex<Option<(u64, Sender<String>)>>,
    cancellation: Mutex<Option<(u64, Arc<AtomicBool>)>>,
    next_attempt: AtomicU64,
    current_attempt: AtomicU64,
}

impl RemoteSessionShared {
    pub fn new() -> Self {
        Self {
            password_tx: Mutex::new(None),
            cancellation: Mutex::new(None),
            next_attempt: AtomicU64::new(1),
            current_attempt: AtomicU64::new(0),
        }
    }

    pub fn begin(&self, password_tx: Sender<String>) -> (u64, Arc<AtomicBool>) {
        self.cancel();
        let attempt_id = self.next_attempt.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.current_attempt.store(attempt_id, Ordering::Release);
        if let Ok(mut slot) = self.password_tx.lock() {
            *slot = Some((attempt_id, password_tx));
        }
        if let Ok(mut slot) = self.cancellation.lock() {
            *slot = Some((attempt_id, Arc::clone(&cancelled)));
        }
        (attempt_id, cancelled)
    }

    pub fn is_current(&self, attempt_id: u64) -> bool {
        self.current_attempt.load(Ordering::Acquire) == attempt_id
    }

    pub fn submit_password(&self, password: String) {
        let sender = self
            .password_tx
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some((attempt_id, tx)) = sender {
            if self.is_current(attempt_id) {
                let _ = tx.send(password);
            }
        }
    }

    pub fn clear_password(&self, attempt_id: u64) {
        if let Ok(mut slot) = self.password_tx.lock() {
            if slot.as_ref().is_some_and(|(id, _)| *id == attempt_id) {
                *slot = None;
            }
        }
    }

    pub fn finish(&self, attempt_id: u64) {
        self.clear_password(attempt_id);
        if let Ok(mut slot) = self.cancellation.lock() {
            if slot.as_ref().is_some_and(|(id, _)| *id == attempt_id) {
                *slot = None;
            }
        }
    }

    pub fn cancel(&self) {
        if let Ok(mut slot) = self.cancellation.lock() {
            if let Some((_, token)) = slot.take() {
                token.store(true, Ordering::Release);
            }
        }
        if let Ok(mut slot) = self.password_tx.lock() {
            *slot = None;
        }
        self.current_attempt.store(0, Ordering::Release);
    }
}

impl Drop for RemoteSessionShared {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) struct RemoteStateUpdate {
    pub attempt_id: u64,
    pub state: String,
    pub host_id: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
    pub session_id: Option<String>,
    pub error: Option<String>,
    pub sessions: Vec<serde_json::Value>,
}

pub(super) struct ActiveRemote {
    pub attempt_id: u64,
    pub connection: super::connect::Connection,
    pub ssh_child: tokio::process::Child,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_connect_worker(
    attempt_id: u64,
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    connected_tx: Sender<ActiveRemote>,
    pw_rx: Receiver<String>,
    cancelled: Arc<AtomicBool>,
    shared: Arc<RemoteSessionShared>,
    handle: tokio::runtime::Handle,
) {
    std::thread::spawn(move || {
        remote_connect_worker(
            attempt_id,
            host_id,
            state_tx,
            connected_tx,
            pw_rx,
            cancelled,
            shared,
            handle,
        );
    });
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn remote_connect_worker(
    attempt_id: u64,
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    connected_tx: Sender<ActiveRemote>,
    pw_rx: Receiver<String>,
    cancelled: Arc<AtomicBool>,
    shared: Arc<RemoteSessionShared>,
    handle: tokio::runtime::Handle,
) {
    use crate::remote::auth::{self, AuthProbe, SshAuth};
    use crate::remote::{bootstrap, ssh, RemoteTarget};

    let is_cancelled = || cancelled.load(Ordering::Acquire) || !shared.is_current(attempt_id);
    let push_state = {
        let host_id = host_id.clone();
        let state_tx = state_tx.clone();
        move |state: &str,
              user: Option<&str>,
              host: Option<&str>,
              session_id: Option<&str>,
              error: Option<&str>,
              sessions: Vec<serde_json::Value>| {
            if !is_cancelled() {
                let _ = state_tx.send(RemoteStateUpdate {
                    attempt_id,
                    state: state.to_string(),
                    host_id: Some(host_id.clone()),
                    user: user.map(str::to_string),
                    host: host.map(str::to_string),
                    session_id: session_id.map(str::to_string),
                    error: error.map(str::to_string),
                    sessions,
                });
            }
        }
    };

    let hosts = crate::remote::hosts::load_hosts();
    let host_data = match crate::remote::hosts::host_by_id(&hosts, &host_id) {
        Some(host) => host.clone(),
        None => {
            push_state(
                "error",
                None,
                None,
                None,
                Some("host not found"),
                Vec::new(),
            );
            shared.finish(attempt_id);
            return;
        }
    };
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }

    let user_str = host_data.user.clone();
    let host_str = host_data.host.clone();
    let target = RemoteTarget {
        user: host_data.user,
        host: host_data.host,
        port: (host_data.port != 22).then_some(host_data.port),
        key: host_data.key_path,
    };
    push_state(
        "resolving",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }

    let auth = match auth::probe_key_auth(&target) {
        AuthProbe::KeyReady => {
            shared.clear_password(attempt_id);
            None
        }
        AuthProbe::PasswordRequired => {
            if is_cancelled() {
                shared.finish(attempt_id);
                return;
            }
            push_state(
                "auth_required",
                Some(&user_str),
                Some(&host_str),
                None,
                None,
                Vec::new(),
            );
            let password = match pw_rx.recv() {
                Ok(password) if !is_cancelled() => password,
                _ => {
                    shared.finish(attempt_id);
                    return;
                }
            };
            shared.clear_password(attempt_id);
            match SshAuth::from_password(password) {
                Ok(auth) => Some(auth),
                Err(error) => {
                    push_state(
                        "error",
                        Some(&user_str),
                        Some(&host_str),
                        None,
                        Some(&format!("auth setup failed: {error:#}")),
                        Vec::new(),
                    );
                    shared.finish(attempt_id);
                    return;
                }
            }
        }
    };

    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }
    push_state(
        "bootstrapping",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );
    let auth_ref = auth.as_ref();
    if let Err(error) = bootstrap::ensure_koma_compatible(&target, auth_ref) {
        push_state(
            "error",
            Some(&user_str),
            Some(&host_str),
            None,
            Some(&format!("remote bootstrap failed: {error:#}")),
            Vec::new(),
        );
        shared.finish(attempt_id);
        return;
    }
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    push_state(
        "connecting",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        Vec::new(),
    );
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }
    let koma_path = match ssh::find_koma(&target, auth_ref) {
        Ok(path) => path,
        Err(error) => {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("remote Koma executable not found: {error:#}")),
                Vec::new(),
            );
            shared.finish(attempt_id);
            return;
        }
    };
    // `ssh::connect` uses tokio::process::Command::spawn synchronously, so the
    // worker thread must have an entered runtime while spawning SSH.
    let ssh_session = {
        let _rt_ctx = handle.enter();
        match ssh::connect(&target, &session_id, auth_ref, None, &koma_path) {
            Ok(session) => session,
            Err(error) => {
                push_state(
                    "error",
                    Some(&user_str),
                    Some(&host_str),
                    None,
                    Some(&format!("ssh connect failed: {error:#}")),
                    Vec::new(),
                );
                shared.finish(attempt_id);
                return;
            }
        }
    };
    let crate::remote::ssh::SshSession {
        mut child,
        stdin,
        stdout,
    } = ssh_session;
    if is_cancelled() {
        let _ = handle.block_on(async { child.kill().await });
        shared.finish(attempt_id);
        return;
    }
    let connection = match super::remote::connect_remote(
        &handle,
        stdout,
        stdin,
        host_str.clone(),
        session_id.clone(),
    ) {
        Ok(connection) => connection,
        Err(error) => {
            let _ = handle.block_on(async { child.kill().await });
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                None,
                Some(&format!("bridge failed: {error:#}")),
                Vec::new(),
            );
            shared.finish(attempt_id);
            return;
        }
    };
    if is_cancelled() {
        let _ = handle.block_on(async { child.kill().await });
        shared.finish(attempt_id);
        return;
    }

    let sessions = crate::remote::sessions::list_sessions_over_ssh(&target, auth_ref)
        .map(|sessions| {
            sessions
                .iter()
                .map(|session| {
                    serde_json::json!({
                        "sessionId": session.session_id,
                        "name": session.name,
                        "pwd": session.pwd,
                        "working": session.working,
                        "isForeground": session.is_foreground
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if is_cancelled() {
        let _ = handle.block_on(async { child.kill().await });
        shared.finish(attempt_id);
        return;
    }
    push_state(
        "connected",
        Some(&user_str),
        Some(&host_str),
        Some(&session_id),
        None,
        sessions,
    );

    if is_cancelled() {
        let _ = handle.block_on(async { child.kill().await });
        shared.finish(attempt_id);
        return;
    }
    let mut hosts = crate::remote::hosts::load_hosts();
    if let Some(host) = hosts.hosts.iter_mut().find(|host| host.id == host_id) {
        host.touch_last_connected();
        let _ = crate::remote::hosts::save_hosts(&hosts);
    }
    if is_cancelled() {
        let _ = handle.block_on(async { child.kill().await });
        shared.finish(attempt_id);
        return;
    }
    shared.finish(attempt_id);
    if let Err(error) = connected_tx.send(ActiveRemote {
        attempt_id,
        connection,
        ssh_child: child,
    }) {
        let mut active = error.0;
        let _ = handle.block_on(async { active.ssh_child.kill().await });
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteSessionShared;

    #[test]
    fn new_attempt_rejects_old_generation_and_closes_password_channel() {
        let shared = RemoteSessionShared::new();
        let (first_tx, first_rx) = std::sync::mpsc::channel();
        let (first, first_cancel) = shared.begin(first_tx);
        let (second_tx, _second_rx) = std::sync::mpsc::channel();
        let (second, _) = shared.begin(second_tx);

        assert!(first_cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(!shared.is_current(first));
        assert!(shared.is_current(second));
        assert!(first_rx.recv().is_err());
    }

    #[test]
    fn finish_clears_password_sender() {
        let shared = RemoteSessionShared::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let (attempt, _) = shared.begin(tx);
        shared.finish(attempt);
        assert!(rx.recv().is_err());
    }
}

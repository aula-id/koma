//! Remote host connect/disconnect worker for the GUI host-relay bridge.
//!
//! Blocking SSH/auth work runs on a dedicated thread. Host connect and session
//! attach are separate: connect stops at `ready` with a retained [`RemoteCtx`];
//! session attach SSHes `koma server` (stdio↔sock **bridge**, not the agent)
//! only after the user picks a session or folder. Every attempt has a fresh
//! cancellation token and monotonically increasing id so late worker results
//! cannot replace a newer transport.
//!
//! Cancel/error paths kill the SSH bridge child only. That does **not** delete
//! the remote session-daemon — QuitDaemon (hub kill / `/new kill`) does.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::remote::auth::SshAuth;
use crate::remote::RemoteTarget;

/// Retained remote-host context after auth+bootstrap succeed and before/while a
/// session is attached. Lives only on the host-relay thread — never serialised
/// into a PushEnvelope (password must not leave process memory as wire JSON).
///
/// Password is stored as a plain string (same as TUI `RemoteContext`) so the
/// ctx can be cloned across hub ↔ attach transitions; each SSH op rebuilds a
/// short-lived [`SshAuth`] askpass file.
#[derive(Clone)]
pub(super) struct RemoteCtx {
    pub host_id: String,
    pub target: RemoteTarget,
    pub password: Option<String>,
    pub koma_path: String,
}

impl RemoteCtx {
    /// Password string for off-thread SSH helpers that must rebuild [`SshAuth`]
    /// (askpass files are single-owner). `None` when key auth is in use.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Build a short-lived askpass context for one SSH operation.
    pub fn make_auth(&self) -> anyhow::Result<Option<SshAuth>> {
        match self.password.as_ref() {
            Some(pw) => Ok(Some(SshAuth::from_password(pw.clone())?)),
            None => Ok(None),
        }
    }

    pub fn host_label(&self) -> String {
        format!("{}@{}", self.target.user, self.target.host)
    }
}

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

    /// Replace the password oneshot for an in-flight attempt (stale-store retry).
    /// Returns the new receiver, or `None` if the attempt is no longer current.
    pub fn rearm_password(&self, attempt_id: u64) -> Option<Receiver<String>> {
        if !self.is_current(attempt_id) {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut slot) = self.password_tx.lock() {
            *slot = Some((attempt_id, tx));
        }
        Some(rx)
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

/// Live SSH-bridged remote session (after the user picked a session/folder).
pub(super) struct ActiveRemote {
    pub attempt_id: u64,
    pub connection: super::connect::Connection,
    pub ssh_child: tokio::process::Child,
    /// Host context retained across session teardown so `/resume` can return to
    /// the remote hub without re-authing.
    pub ctx: RemoteCtx,
    /// Coding-panel thin client (`koma remote-fs` over SSH). Started best-effort
    /// at attach; `None` if spawn failed (File* then error via local path or
    /// explicit remote error envelopes).
    pub fs: Option<super::remote_fs_client::RemoteFsClient>,
    /// Source Control thin client (`koma remote-git` over SSH). Started
    /// best-effort at attach; `None` if spawn failed (Git* then error via
    /// remote-unavailable envelopes — never local git against remote paths).
    pub git: Option<super::remote_git_client::RemoteGitClient>,
    /// Import-Graph thin client (`koma remote-linker` over SSH). Started
    /// best-effort at attach; `None` if spawn failed or linker feature off.
    #[cfg(feature = "linker")]
    pub linker: Option<super::remote_linker_client::RemoteLinkerClient>,
    /// Initial remote cwd used to seed remote-fs sandbox (if any).
    pub cwd: Option<String>,
}

/// Outcome of the host-connect worker: host is ready, no session attached yet.
pub(super) struct ReadyRemote {
    pub attempt_id: u64,
    pub ctx: RemoteCtx,
    pub sessions: Vec<serde_json::Value>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_connect_worker(
    attempt_id: u64,
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    ready_tx: Sender<ReadyRemote>,
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
            ready_tx,
            pw_rx,
            cancelled,
            shared,
            handle,
        );
    });
}

/// Attach (or create) a remote session over SSH using a retained [`RemoteCtx`].
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_session_worker(
    attempt_id: u64,
    ctx: RemoteCtx,
    session_id: String,
    cwd: Option<String>,
    state_tx: Sender<RemoteStateUpdate>,
    connected_tx: Sender<ActiveRemote>,
    cancelled: Arc<AtomicBool>,
    shared: Arc<RemoteSessionShared>,
    handle: tokio::runtime::Handle,
) {
    std::thread::spawn(move || {
        remote_session_worker(
            attempt_id,
            ctx,
            session_id,
            cwd,
            state_tx,
            connected_tx,
            cancelled,
            shared,
            handle,
        );
    });
}

fn sessions_to_json(
    sessions: &crate::remote::sessions::DiscoveredSessions,
) -> Vec<serde_json::Value> {
    // Live-only list for the remoteState badge / status strip. History is pushed
    // through the hub panes (`build_remote_hub`), not this badge payload.
    sessions
        .live
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
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn remote_connect_worker(
    attempt_id: u64,
    host_id: String,
    state_tx: Sender<RemoteStateUpdate>,
    ready_tx: Sender<ReadyRemote>,
    pw_rx: Receiver<String>,
    cancelled: Arc<AtomicBool>,
    shared: Arc<RemoteSessionShared>,
    _handle: tokio::runtime::Handle,
) {
    use crate::remote::auth::{self, AuthProbe};
    use crate::remote::{bootstrap, ssh};

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

    let mut password_from_store = false;
    let mut password = match auth::probe_key_auth(&target) {
        AuthProbe::KeyReady => {
            shared.clear_password(attempt_id);
            None
        }
        AuthProbe::PasswordRequired => {
            if is_cancelled() {
                shared.finish(attempt_id);
                return;
            }
            // Prefer encrypted store (shared with TUI) before prompting the UI.
            let password = if let Some(password) =
                crate::remote::secrets::get_remote_password(&host_id)
            {
                password_from_store = true;
                password
            } else {
                push_state(
                    "auth_required",
                    Some(&user_str),
                    Some(&host_str),
                    None,
                    None,
                    Vec::new(),
                );
                match pw_rx.recv() {
                    Ok(password) if !is_cancelled() => password,
                    _ => {
                        shared.finish(attempt_id);
                        return;
                    }
                }
            };
            shared.clear_password(attempt_id);
            Some(password)
        }
    };

    // Short-lived askpass for bootstrap + find_koma + session list.
    let mut auth = match password
        .as_ref()
        .map(|p| SshAuth::from_password(p.clone()))
        .transpose()
    {
        Ok(auth) => auth,
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
    if let Err(error) = bootstrap::ensure_koma_compatible(&target, auth.as_ref()) {
        // Stale store password: wipe, re-open UI prompt once, retry bootstrap.
        if password_from_store {
            let _ = crate::remote::secrets::delete_remote_password(&host_id);
            if is_cancelled() {
                shared.finish(attempt_id);
                return;
            }
            let Some(retry_rx) = shared.rearm_password(attempt_id) else {
                shared.finish(attempt_id);
                return;
            };
            push_state(
                "auth_required",
                Some(&user_str),
                Some(&host_str),
                None,
                None,
                Vec::new(),
            );
            let retry_password = match retry_rx.recv() {
                Ok(password) if !is_cancelled() => password,
                _ => {
                    shared.finish(attempt_id);
                    return;
                }
            };
            shared.clear_password(attempt_id);
            password = Some(retry_password.clone());
            auth = match SshAuth::from_password(retry_password) {
                Ok(a) => Some(a),
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
            };
            push_state(
                "bootstrapping",
                Some(&user_str),
                Some(&host_str),
                None,
                None,
                Vec::new(),
            );
            if let Err(error) = bootstrap::ensure_koma_compatible(&target, auth.as_ref()) {
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
        } else {
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
    }
    // Persist password after successful bootstrap (shared with TUI).
    if let Some(ref pw) = password {
        let _ = crate::remote::secrets::set_remote_password(&host_id, pw);
    }
    let auth_ref = auth.as_ref();
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
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }

    let sessions = crate::remote::sessions::list_sessions_over_ssh(&target, auth_ref)
        .map(|s| sessions_to_json(&s))
        .unwrap_or_default();
    // Drop askpass before retaining the password-only ctx.
    drop(auth);
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }

    // Host is live — no session UUID, no `ssh::connect`. Session attach is a
    // separate user action (pick existing id or open a remote folder).
    push_state(
        "ready",
        Some(&user_str),
        Some(&host_str),
        None,
        None,
        sessions.clone(),
    );

    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }
    let mut hosts = crate::remote::hosts::load_hosts();
    if let Some(host) = hosts.hosts.iter_mut().find(|host| host.id == host_id) {
        host.touch_last_connected();
        let _ = crate::remote::hosts::save_hosts(&hosts);
    }
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }
    shared.finish(attempt_id);
    let _ = ready_tx.send(ReadyRemote {
        attempt_id,
        ctx: RemoteCtx {
            host_id,
            target,
            password,
            koma_path,
        },
        sessions,
    });
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn remote_session_worker(
    attempt_id: u64,
    ctx: RemoteCtx,
    session_id: String,
    cwd: Option<String>,
    state_tx: Sender<RemoteStateUpdate>,
    connected_tx: Sender<ActiveRemote>,
    cancelled: Arc<AtomicBool>,
    shared: Arc<RemoteSessionShared>,
    handle: tokio::runtime::Handle,
) {
    use crate::remote::ssh;

    let is_cancelled = || cancelled.load(Ordering::Acquire) || !shared.is_current(attempt_id);
    let user_str = ctx.target.user.clone();
    let host_str = ctx.target.host.clone();
    let host_id = ctx.host_id.clone();
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

    push_state(
        "connecting",
        Some(&user_str),
        Some(&host_str),
        Some(&session_id),
        None,
        Vec::new(),
    );
    if is_cancelled() {
        shared.finish(attempt_id);
        return;
    }

    let auth = match ctx.make_auth() {
        Ok(auth) => auth,
        Err(error) => {
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                Some(&session_id),
                Some(&format!("auth setup failed: {error:#}")),
                Vec::new(),
            );
            shared.finish(attempt_id);
            return;
        }
    };
    let auth_ref = auth.as_ref();
    // `ssh::connect` uses tokio::process::Command::spawn synchronously, so the
    // worker thread must have an entered runtime while spawning SSH.
    let ssh_session = {
        let _rt_ctx = handle.enter();
        match ssh::connect(
            &ctx.target,
            &session_id,
            auth_ref,
            cwd.as_deref(),
            &ctx.koma_path,
        ) {
            Ok(session) => session,
            Err(error) => {
                push_state(
                    "error",
                    Some(&user_str),
                    Some(&host_str),
                    Some(&session_id),
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
    // Cancel after SSH spawn: kill the bridge only (session-daemon keeps cooking).
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
            // Bridge process only — not session delete.
            let _ = handle.block_on(async { child.kill().await });
            push_state(
                "error",
                Some(&user_str),
                Some(&host_str),
                Some(&session_id),
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

    let sessions = crate::remote::sessions::list_sessions_over_ssh(&ctx.target, auth_ref)
        .map(|s| sessions_to_json(&s))
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
    shared.finish(attempt_id);
    // Best-effort Coding-panel thin client. Failure is non-fatal — File* will
    // surface errors when used; chat still works via the bridge above.
    let fs = super::remote_fs_client::RemoteFsClient::start(
        &handle,
        &ctx,
        cwd.as_deref(),
    )
    .ok();
    // Best-effort Source Control thin client. Failure is non-fatal — Git* will
    // surface unavailable when used; chat still works via the bridge above.
    let git = super::remote_git_client::RemoteGitClient::start(
        &handle,
        &ctx,
        Some(&session_id),
    )
    .ok();
    // Best-effort Import-Graph thin client (linker feature). Failure is
    // non-fatal — ImportGraph* will surface unavailable when used.
    #[cfg(feature = "linker")]
    let linker = super::remote_linker_client::RemoteLinkerClient::start(
        &handle,
        &ctx,
        cwd.as_deref(),
    )
    .ok();
    if let Err(error) = connected_tx.send(ActiveRemote {
        attempt_id,
        connection,
        ssh_child: child,
        ctx,
        fs,
        git,
        #[cfg(feature = "linker")]
        linker,
        cwd,
    }) {
        // Receiver gone: drop the unused bridge child only.
        let mut active = error.0;
        let _ = handle.block_on(async { active.ssh_child.kill().await });
        if let Some(mut fs) = active.fs.take() {
            fs.shutdown();
        }
        if let Some(mut git) = active.git.take() {
            git.shutdown();
        }
        #[cfg(feature = "linker")]
        if let Some(mut linker) = active.linker.take() {
            linker.shutdown();
        }
    }
}

#[cfg(test)]
#[path = "remote_ctl_test.rs"]
mod tests;

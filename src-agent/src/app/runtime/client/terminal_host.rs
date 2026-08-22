//! Host-side PTY lifecycle management for the GUI terminal view.
//!
//! Each terminal tab in the React UI is backed by a real pseudo-terminal spawned
//! via [`portable_pty`]. The [`TerminalManager`] owns every live session and
//! exposes thin `create`/`create_remote`/`input`/`resize`/`kill`/`cleanup_all`
//! operations that the host-relay control loop calls when it receives
//! [`super::HostCtl`] terminal messages.
//!
//! ## Local vs remote
//!
//! - **Local** — spawn the host `$SHELL` (or COMSPEC) in a PTY.
//! - **Remote** — spawn `ssh -t user@host '…login shell…'` in a local PTY, reusing
//!   the same ControlMaster / askpass path as the remote agent bridge. Input /
//!   resize / output protocol is identical; only the child argv changes.
//!
//! ## Threading model
//!
//! - **Reader thread** -- one per session, spawned at creation. Reads the PTY's
//!   `master` half in a blocking loop and pushes each chunk to the webview via
//!   `push_terminal_output`. Terminates on EOF or read error (the child exited
//!   or the master was dropped), at which point it pushes a `TerminalExit`
//!   envelope.
//! - **Control flow** -- `input`/`resize`/`kill` are called synchronously on
//!   the host-relay thread (the 16ms control loop); they lock the manager
//!   briefly and forward to the PTY/child, which are `Send`.
//!
//! No separate waiter thread is needed: the reader naturally observes EOF when
//! the child dies and emits the exit notification.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::sync::Arc;

use portable_pty::{CommandBuilder, MasterPty, PtySize};

use crate::remote::auth::SshAuth;
use crate::remote::ssh;
use crate::remote::RemoteTarget;

use super::push_proto::{push_terminal_exit, push_terminal_output};

/// Resolve the default shell for the current platform.
fn platform_shell() -> String {
    if cfg!(target_os = "windows") {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// A single live terminal session.
struct TerminalSession {
    /// The PTY master -- kept alive so the reader thread can drain it. Dropping
    /// this closes the master end, which signals EOF to the reader and (on most
    /// Unix platforms) SIGHUP to the child.
    ///
    /// `+ Send` matches [`portable_pty::PtyPair::master`] so `TerminalManager`
    /// is `Send` and can live in an `Arc<Mutex<_>>` shared across host states.
    master: Box<dyn MasterPty + Send>,
    /// The child process. Used for `kill`.
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Writer end of the PTY for forwarding keystrokes from the webview.
    writer: Box<dyn Write + Send>,
    /// Keeps the askpass script alive for password-auth remote shells. Dropped
    /// only when the session is killed (SshAuth::Drop deletes the temp file).
    _auth: Option<SshAuth>,
}

/// Manages all live terminal sessions for the host-relay. Shared between the
/// host-swapper and host-attached control loops via `Arc<Mutex<...>>`.
pub(super) struct TerminalManager {
    sessions: HashMap<String, TerminalSession>,
    /// An owned clone of the host-relay's push sink so reader threads can push
    /// envelopes without borrowing the caller's stack frame.
    push: Arc<dyn Fn(String) + Send + Sync>,
}

impl TerminalManager {
    /// Create a new manager. `push` is cloned into an `Arc` so reader threads
    /// can outlive any individual `create` call.
    pub fn new(push: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            sessions: HashMap::new(),
            push: Arc::new(push),
        }
    }

    /// Spawn a new **local** PTY session. `id` is a stable identifier from the
    /// React side; `cwd` is the working directory (falls back to the process cwd
    /// if `None`).
    pub fn create(&mut self, id: String, cwd: Option<String>) -> anyhow::Result<()> {
        let shell = platform_shell();

        let mut cmd = CommandBuilder::new(&shell);
        if cfg!(not(target_os = "windows")) {
            cmd.arg("--login");
        }
        if let Some(dir) = &cwd {
            cmd.cwd(dir);
        }

        self.spawn_cmd(id, cmd, None)
    }

    /// Spawn a PTY whose child is `ssh -t` into `target` (interactive remote
    /// login shell). Reuses ControlMaster mux + askpass from the remote stack.
    ///
    /// `password` rebuilds a short-lived [`SshAuth`] kept on the session for the
    /// life of the PTY. `cwd` is an optional remote path to `cd` into first.
    pub fn create_remote(
        &mut self,
        id: String,
        target: &RemoteTarget,
        password: Option<&str>,
        cwd: Option<&str>,
    ) -> anyhow::Result<()> {
        let auth = match password {
            Some(pw) => Some(SshAuth::from_password(pw.to_string())?),
            None => None,
        };
        let cmd = ssh::interactive_shell_command(target, auth.as_ref(), cwd)?;
        self.spawn_cmd(id, cmd, auth)
    }

    fn spawn_cmd(
        &mut self,
        id: String,
        cmd: CommandBuilder,
        auth: Option<SshAuth>,
    ) -> anyhow::Result<()> {
        let pty_size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = portable_pty::native_pty_system().openpty(pty_size)?;

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("failed to spawn terminal: {e}"))?;

        let mut writer = pair.master.take_writer()?;
        writer.flush()?;

        let reader = pair.master.try_clone_reader()?;

        let session = TerminalSession {
            master: pair.master,
            child,
            writer,
            _auth: auth,
        };
        self.sessions.insert(id.clone(), session);

        // --- reader thread: stream PTY output to the webview ---
        let push_clone = Arc::clone(&self.push);
        let id_clone = id;
        std::thread::spawn(move || {
            let mut reader = BufReader::with_capacity(8192, reader);
            loop {
                let mut buf = vec![0u8; 8192];
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                        push_terminal_output(&*push_clone, &id_clone, &text);
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(())
    }

    /// Forward user input to a terminal session.
    pub fn input(&mut self, id: &str, data: &str) {
        if let Some(session) = self.sessions.get_mut(id) {
            let _ = session.writer.write_all(data.as_bytes());
            let _ = session.writer.flush();
        }
    }

    /// Resize a terminal session's PTY.
    pub fn resize(&mut self, id: &str, cols: u16, rows: u16) {
        if let Some(session) = self.sessions.get_mut(id) {
            let size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            let _ = session.master.resize(size);
        }
    }

    /// Kill a terminal session by killing the child process. Dropping the master
    /// after ensures the reader thread gets EOF.
    pub fn kill(&mut self, id: &str) {
        if let Some(mut session) = self.sessions.remove(id) {
            let _ = session.child.kill();
            drop(session.master);
            drop(session.writer);
            drop(session._auth);
            push_terminal_exit(&*self.push, id, None);
        }
    }

    /// Kill all sessions. Called on host-relay teardown.
    pub fn cleanup_all(&mut self) {
        for (_, mut session) in self.sessions.drain() {
            let _ = session.child.kill();
            drop(session.master);
            drop(session._auth);
        }
    }
}

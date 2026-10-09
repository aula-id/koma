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
//! - **Local** — spawn Git Bash when it is installed or bundled beside koma,
//!   otherwise `COMSPEC`. Unix uses `$SHELL`.
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
use std::path::PathBuf;
use std::sync::Arc;

use portable_pty::{CommandBuilder, MasterPty, PtySize};

use crate::remote::auth::SshAuth;
use crate::remote::ssh;
use crate::remote::RemoteTarget;

use super::push_proto::{push_terminal_exit, push_terminal_output};

const POSIX_DISCOVERY: &str = r#"{
  printf '%s\n' "$SHELL"
  if [ -r /etc/shells ]; then cat /etc/shells; fi
  for s in bash zsh fish sh dash ksh nu pwsh; do command -v "$s" || :; done
} | while IFS= read -r s; do
  case "$s" in /*) if [ -f "$s" ] && [ -x "$s" ]; then
    if command -v realpath >/dev/null 2>&1; then realpath "$s"; else printf '%s\n' "$s"; fi
  fi;; esac
done"#;

pub(crate) fn shell_args(shell: &str) -> &'static [&'static str] {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    match name.trim_end_matches(".exe") {
        "cmd" => &[],
        "powershell" | "pwsh" => &["-NoLogo", "-NoExit"],
        "nu" => &["--login", "--interactive"],
        "bash" | "rbash" | "zsh" | "fish" | "sh" | "dash" | "ksh" => &["-l", "-i"],
        _ => &[],
    }
}

fn executable(path: &std::path::Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|p| executable(p))
        .filter_map(|p| {
            let key = std::fs::canonicalize(&p).ok()?;
            #[cfg(windows)]
            let key = key.to_string_lossy().to_lowercase();
            if seen.insert(key) {
                Some(p)
            } else {
                None
            }
        })
        .collect()
}

fn local_shells() -> anyhow::Result<Vec<String>> {
    let mut paths = Vec::new();
    #[cfg(not(windows))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            paths.push(PathBuf::from(shell));
        }
        match std::fs::read_to_string("/etc/shells") {
            Ok(shells) => paths.extend(
                shells
                    .lines()
                    .map(str::trim)
                    .filter(|s| s.starts_with('/'))
                    .map(PathBuf::from),
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    #[cfg(windows)]
    {
        if let Some(bash) = crate::tool::shell::find_git_bash() {
            paths.push(bash);
        }
        if let Ok(shell) = std::env::var("COMSPEC") {
            paths.push(PathBuf::from(shell));
        }
        if let Ok(root) = std::env::var("SystemRoot") {
            paths.push(PathBuf::from(&root).join("System32/WindowsPowerShell/v1.0/powershell.exe"));
            paths.push(PathBuf::from(root).join("System32/cmd.exe"));
        }
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(root) = std::env::var(variable) {
                let root = PathBuf::from(root);
                paths.push(root.join("Git/bin/bash.exe"));
                paths.push(root.join("Programs/Git/bin/bash.exe"));
                if let Ok(versions) = std::fs::read_dir(root.join("PowerShell")) {
                    paths.extend(versions.flatten().map(|v| v.path().join("pwsh.exe")));
                }
                paths.push(root.join("Microsoft/WindowsApps/pwsh.exe"));
            }
        }
    }
    #[cfg(windows)]
    let names = ["bash.exe", "pwsh.exe", "powershell.exe", "cmd.exe"];
    #[cfg(not(windows))]
    let names = ["bash", "zsh", "fish", "sh", "dash", "ksh", "nu", "pwsh"];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            paths.extend(names.iter().map(|name| dir.join(name)));
        }
    }
    Ok(unique_paths(paths)
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
}

fn shell_label(path: &str) -> String {
    match path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .trim_end_matches(".exe")
    {
        "powershell" => "Windows PowerShell".into(),
        "pwsh" => "PowerShell".into(),
        #[cfg(windows)]
        "bash" => "Git Bash".into(),
        name => name.to_string(),
    }
}

pub(super) fn discover_async(
    manager: Arc<std::sync::Mutex<TerminalManager>>,
    request_id: String,
    context: String,
    remote: Option<(RemoteTarget, Option<String>)>,
) {
    std::thread::spawn(move || {
        let result = match remote {
            Some((target, password)) => (|| -> anyhow::Result<Vec<String>> {
                let auth = password.map(SshAuth::from_password).transpose()?;
                let output = ssh::shell_discovery(&target, auth.as_ref(), POSIX_DISCOVERY)?;
                Ok(output
                    .lines()
                    .filter(|s| s.starts_with('/'))
                    .map(str::to_owned)
                    .collect())
            })(),
            None => local_shells(),
        };
        if let Ok(mut mgr) = manager.lock() {
            let mut shells = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let error = match result {
                Ok(paths) => {
                    for path in paths {
                        if !seen.insert(path.clone()) {
                            continue;
                        }
                        let id = mgr
                            .shells
                            .iter()
                            .find(|(_, (host, executable))| host == &context && executable == &path)
                            .map(|(id, _)| id.clone())
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        shells.push(serde_json::json!({"id": id, "label": shell_label(&path)}));
                        mgr.shells.insert(id, (context.clone(), path));
                    }
                    None
                }
                Err(e) => Some(e.to_string()),
            };
            let envelope = super::push_proto::PushEnvelope::TerminalShells {
                request_id,
                context,
                shells,
                error,
            };
            if let Ok(json) = serde_json::to_string(&envelope) {
                (mgr.push)(json);
            }
        }
    });
}

/// Resolve the default shell for the current platform.
///
/// Windows prefers Git for Windows (or the MSI fallback under `shell\`), and
/// uses `cmd.exe` only when neither exists. The GUI itself stays a native
/// process; only this child is Bash.
fn platform_shell() -> String {
    #[cfg(windows)]
    {
        if let Some(bash) = crate::tool::shell::find_git_bash() {
            return bash.display().to_string();
        }
        return std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn shell_is_zsh(shell: &str) -> bool {
    shell.ends_with("zsh") || shell.ends_with("/zsh")
}

/// On macOS, login zsh from GUI PTYs often mis-handles xterm erase/delete
/// (Backspace → space, Delete → "~") even when TERM is set. Point ZDOTDIR at a
/// tiny rc that loads the user's config then re-binds xterm-style keys.
#[cfg(target_os = "macos")]
fn macos_zsh_zdot(cmd: &mut CommandBuilder, shell: &str) -> Option<PathBuf> {
    if !shell_is_zsh(shell) {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("koma-zdot-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok()?;
    let zprofile = r#"[[ -f "$HOME/.zprofile" ]] && source "$HOME/.zprofile"
[[ -f "$HOME/.zlogin" ]] && source "$HOME/.zlogin"
"#;
    let zshrc = r#"# Koma GUI PTY — xterm.js / WebKit key sequences for zsh.
[[ -f "$HOME/.zshrc" ]] && source "$HOME/.zshrc"
bindkey '^?' backward-delete-char
bindkey '^H' backward-delete-char
bindkey '^[[3~' delete-char
"#;
    std::fs::write(dir.join(".zprofile"), zprofile).ok()?;
    std::fs::write(dir.join(".zshrc"), zshrc).ok()?;
    cmd.env("ZDOTDIR", &dir);
    Some(dir)
}

#[cfg(not(target_os = "macos"))]
fn macos_zsh_zdot(_cmd: &mut CommandBuilder, _shell: &str) -> Option<PathBuf> {
    None
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
    /// macOS zsh: temp ZDOTDIR so login shells bind xterm erase/delete keys.
    _zdot_dir: Option<PathBuf>,
}

/// Manages all live terminal sessions for the host-relay. Shared between the
/// host-swapper and host-attached control loops via `Arc<Mutex<...>>`.
pub(super) struct TerminalManager {
    sessions: HashMap<String, TerminalSession>,
    shells: HashMap<String, (String, String)>,
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
            shells: HashMap::new(),
            push: Arc::new(push),
        }
    }

    /// Spawn a new **local** PTY session. `id` is a stable identifier from the
    /// React side; `cwd` is the working directory (falls back to the process cwd
    /// if `None`).
    pub fn create(
        &mut self,
        id: String,
        cwd: Option<String>,
        shell_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let shell = match shell_id {
            Some(id) => {
                let (context, path) = self
                    .shells
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown shell; reopen the shell picker"))?;
                anyhow::ensure!(
                    context == "local" && executable(std::path::Path::new(path)),
                    "Selected shell is no longer available"
                );
                path.clone()
            }
            None => platform_shell(),
        };

        let mut cmd = CommandBuilder::new(&shell);
        let args = shell_args(&shell);
        #[cfg(windows)]
        let mut args = args;
        #[cfg(windows)]
        if shell_label(&shell) == "Git Bash" {
            // Every discovered Git Bash gets the same MSYS environment, including
            // alternate installations selected from PATH or standard locations.
            let bash = PathBuf::from(&shell);
            args = &["-i"];
            for (key, value) in crate::tool::shell::git_bash_env(&bash) {
                cmd.env(key, value);
            }
        }
        for arg in args {
            cmd.arg(arg);
        }
        if let Some(dir) = &cwd {
            cmd.cwd(dir);
        }
        let zdot_dir = macos_zsh_zdot(&mut cmd, &shell);

        self.spawn_cmd(id, cmd, None, zdot_dir)
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
        context: &str,
        password: Option<&str>,
        cwd: Option<&str>,
        shell_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let auth = match password {
            Some(pw) => Some(SshAuth::from_password(pw.to_string())?),
            None => None,
        };
        let shell = shell_id
            .map(|id| {
                self.shells
                    .get(id)
                    .filter(|(host, _)| host == context)
                    .map(|(_, path)| path.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Unknown shell; reopen the shell picker"))
            })
            .transpose()?;
        let cmd = ssh::interactive_shell_command(target, auth.as_ref(), cwd, shell)?;
        self.spawn_cmd(id, cmd, auth, None)
    }

    pub fn report_error(&self, id: &str, error: &str) {
        push_terminal_output(
            &*self.push,
            id,
            &format!("\r\nTerminal launch failed: {error}\r\n"),
        );
        push_terminal_exit(&*self.push, id, Some(1));
    }

    fn spawn_cmd(
        &mut self,
        id: String,
        cmd: CommandBuilder,
        auth: Option<SshAuth>,
        zdot_dir: Option<PathBuf>,
    ) -> anyhow::Result<()> {
        let pty_size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = portable_pty::native_pty_system().openpty(pty_size)?;

        // Match the coding-task PTY and xterm.js: a real TERM so readline/terminfo
        // bind Delete/Backspace correctly (otherwise ESC [ 3 ~ can echo "~", etc.).
        let mut cmd = cmd;
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

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
            _zdot_dir: zdot_dir,
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
            if let Some(dir) = session._zdot_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_shell_arguments() {
        for shell in [
            "/bin/bash",
            "/bin/sh",
            "/bin/dash",
            "/bin/zsh",
            "/bin/fish",
            "/bin/ksh",
        ] {
            assert_eq!(shell_args(shell), ["-l", "-i"]);
        }
        assert_eq!(shell_args("/opt/nu"), ["--login", "--interactive"]);
        assert_eq!(
            shell_args("C:\\PowerShell\\pwsh.exe"),
            ["-NoLogo", "-NoExit"]
        );
        assert_eq!(shell_args("powershell.exe"), ["-NoLogo", "-NoExit"]);
        assert!(shell_args("cmd.exe").is_empty());
        assert!(shell_args("/usr/bin/custom-shell").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn terminal_executable_filter_and_deduplication() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("koma-shell-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("shell");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!executable(&file));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = dir.join("alias");
        symlink(&file, &alias).unwrap();
        assert_eq!(
            unique_paths(vec![
                file.clone(),
                alias,
                file.clone(),
                dir.clone(),
                dir.join("missing")
            ]),
            vec![file.clone()]
        );
        let mut mgr = TerminalManager::new(|_| {});
        mgr.shells.insert(
            "missing".into(),
            ("local".into(), file.to_string_lossy().into_owned()),
        );
        std::fs::remove_file(&file).unwrap();
        assert!(mgr.create("id".into(), None, Some("missing")).is_err());
        assert!(mgr.create("id".into(), None, Some("unknown")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn terminal_posix_discovery_filters_executables() {
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", POSIX_DISCOVERY])
            .output()
            .unwrap();
        assert!(output.status.success());
        let paths: Vec<_> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(PathBuf::from)
            .collect();
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|p| executable(p)));
        assert!(local_shells()
            .unwrap()
            .iter()
            .all(|p| executable(std::path::Path::new(p))));
    }

    #[cfg(unix)]
    #[test]
    fn terminal_selected_shell_starts_in_workdir() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut manager = TerminalManager::new(move |s| {
            let _ = tx.send(s);
        });
        let cwd = std::fs::canonicalize(std::env::temp_dir()).expect("temp dir");
        for shell in ["/bin/bash", "/bin/sh", "/bin/dash"] {
            if !executable(std::path::Path::new(shell)) {
                continue;
            }
            manager
                .shells
                .insert(shell.into(), ("local".into(), shell.into()));
            manager
                .create(
                    shell.into(),
                    Some(cwd.to_string_lossy().into_owned()),
                    Some(shell),
                )
                .unwrap();
            manager.input(shell, "printf 'SHELL=%s CWD=%s\\n' \"$0\" \"$PWD\"\n");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let expected = format!("SHELL={shell} CWD={}", cwd.display());
            let mut text = String::new();
            while !text.contains(&expected) && std::time::Instant::now() < deadline {
                if let Ok(json) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    let env: serde_json::Value = serde_json::from_str(&json).unwrap();
                    if env["id"] == shell {
                        text.push_str(env["data"].as_str().unwrap_or_default());
                    }
                }
            }
            manager.kill(shell);
            assert!(text.contains(&expected), "{shell}: {text}");
        }
    }
}

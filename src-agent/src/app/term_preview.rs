//! New terminal. Spawns the OpenTUI client in `src-term`. `koma` opens a new
//! session. `koma agents` and `koma --resume` open the resume hub. The client
//! starts a session daemon only when a session is opened. `koma legacy` is the
//! ratatui client.

use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

/// Where the new terminal lands on launch.
pub enum TermStart {
    /// Resume hub. No daemon until a row is opened.
    Hub,
    Remote {
        target: String,
        key: Option<String>,
        port: Option<u16>,
        session: Option<String>,
    },
    /// A session. `None` means the client mints a fresh id.
    Session(Option<String>),
}

/// Spawn the OpenTUI preview and wait until it exits.
///
/// The preview owns the terminal. On Unix it is moved into the foreground
/// process group so Ctrl-C reaches OpenTUI, which restores the screen, instead
/// of killing this process mid-frame.
pub fn run_term_preview(start: TermStart) -> Result<()> {
    let launch = super::term_bundle::prepare().context(
        "the new terminal could not be prepared on this machine.\n  \
         Use `koma legacy` to open the legacy terminal.",
    )?;
    let term_dir = launch.dir;
    let bun = launch.bun;

    let mut cmd = Command::new(&bun);
    // Absolute entry so the client keeps the user's working directory.
    // A new session is bucketed from that directory, not from src-term.
    cmd.arg(term_dir.join("src/index.tsx"))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // The client shells out to this binary for `sessions --json` and
    // `--daemon --session`. `current_exe` stays correct if the binary was renamed.
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("KOMA_BIN", exe);
    }
    // Tree-sitter wasm keeps about 10 MB per compiler thread. One thread is enough.
    cmd.env("BUN_JSC_numberOfWasmCompilerThreads", "1");
    // Hello.version is this fingerprint. The client restarts a session daemon
    // that was built from an older binary, which would reject ReplaceInput.
    cmd.env("KOMA_DATA_DIR", crate::model::store::base_dir()?);
    cmd.env("KOMA_BUILD", crate::model::store::build_fingerprint());
    cmd.env("KOMA_VERSION", crate::model::store::current_version());
    match start {
        TermStart::Remote {
            target,
            key,
            port,
            session,
        } => {
            let mut parsed = crate::remote::parse_target(&target)?;
            if let Some(port) = port {
                parsed.port = Some(port);
            }
            let target = match parsed.port {
                Some(port) => format!("{}@{}:{}", parsed.user, parsed.host, port),
                None => format!("{}@{}", parsed.user, parsed.host),
            };
            cmd.env("KOMA_REMOTE",serde_json::to_string(&serde_json::json!({"target":target,"key":key,"newSession":false,"sessionId":session,"hostId":null}))?);
        }
        TermStart::Hub => {
            cmd.env("KOMA_START", "hub");
        }
        TermStart::Session(id) => {
            cmd.env("KOMA_START", "session");
            if let Some(id) = id {
                cmd.env("KOMA_SESSION", id);
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start {}", bun.display()))?;

    #[cfg(unix)]
    let _foreground = ForegroundGuard::handoff(child.id());
    // Ctrl+C is delivered to the foreground job. If this parent is still in
    // that group, the default action kills it and the hub vanishes before the
    // client can open the quit screen. Ignore it here; the client handles it.
    #[cfg(unix)]
    let _interrupt = InterruptGuard::ignore();
    // A resize signal often hits this parent, not the client. Forward it so
    // the client sees the real width and can replace the UI under 50 columns.
    #[cfg(unix)]
    let _winch = WinchGuard::forward(child.id());

    let status = child
        .wait()
        .context("the new terminal stopped before it could exit")?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => std::process::exit(code),
        None => std::process::exit(1),
    }
}

#[cfg(unix)]
static TERM_CHILD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn forward_winch(_: libc::c_int) {
    let pid = TERM_CHILD.load(std::sync::atomic::Ordering::Relaxed);
    if pid > 0 {
        unsafe {
            libc::kill(pid, libc::SIGWINCH);
        }
    }
}

/// Forwards SIGWINCH to the terminal client.
#[cfg(unix)]
struct WinchGuard;

#[cfg(unix)]
impl WinchGuard {
    fn forward(pid: u32) -> Self {
        TERM_CHILD.store(pid as i32, std::sync::atomic::Ordering::Relaxed);
        unsafe {
            libc::signal(
                libc::SIGWINCH,
                forward_winch as *const () as libc::sighandler_t,
            );
        }
        Self
    }
}

#[cfg(unix)]
impl Drop for WinchGuard {
    fn drop(&mut self) {
        TERM_CHILD.store(0, std::sync::atomic::Ordering::Relaxed);
        unsafe {
            libc::signal(libc::SIGWINCH, libc::SIG_DFL);
        }
    }
}

/// Ignores SIGINT and SIGQUIT in this process while the client is running.
#[cfg(unix)]
struct InterruptGuard;

#[cfg(unix)]
impl InterruptGuard {
    fn ignore() -> Self {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        }
        Self
    }
}

#[cfg(unix)]
impl Drop for InterruptGuard {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
        }
    }
}

/// Puts the preview in the foreground while it runs, then hands the terminal back.
#[cfg(unix)]
struct ForegroundGuard {
    previous: libc::pid_t,
    active: bool,
}

#[cfg(unix)]
impl ForegroundGuard {
    fn handoff(child: u32) -> Self {
        let previous = unsafe { libc::getpgrp() };
        let tty = unsafe { libc::isatty(libc::STDIN_FILENO) } == 1;
        if !tty {
            return Self {
                previous,
                active: false,
            };
        }
        unsafe {
            libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            libc::tcsetpgrp(libc::STDIN_FILENO, child as libc::pid_t);
        }
        Self {
            previous,
            active: true,
        }
    }
}

#[cfg(unix)]
impl Drop for ForegroundGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        unsafe {
            libc::tcsetpgrp(libc::STDIN_FILENO, self.previous);
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
        }
    }
}

/// SSH into a saved host, then return so the OpenTUI client can resume.
///
/// Invoked as `koma term-remote --target user@host`. The password never appears
/// on the command line; the existing TUI auth modal asks when a key is not enough.
pub fn run_term_remote() -> Result<()> {
    let mut target_str = None;
    let mut key = None;
    let mut session_id = None;
    let mut host_id = None;
    let mut new_session = false;
    let mut args = std::env::args().skip(2);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--target" => target_str = args.next(),
            "--key" => key = blank_to_none(args.next()),
            "--session" => session_id = blank_to_none(args.next()),
            "--host" => host_id = blank_to_none(args.next()),
            "--new" => new_session = true,
            other => anyhow::bail!("unknown term-remote argument: {other}"),
        }
    }
    let target_str = target_str.ok_or_else(|| anyhow!("term-remote requires --target"))?;
    let mut parsed = crate::remote::parse_target(&target_str)?;
    if let Some(ref path) = key {
        parsed.key = Some(path.clone());
    }
    let host = host_id.or_else(|| {
        crate::remote::secrets::host_id_for_address(&parsed.user, &parsed.host, parsed.port)
    });
    crate::remote::term_helper::run(parsed, host, session_id, new_session)
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

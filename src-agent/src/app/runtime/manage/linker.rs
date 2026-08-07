//! GLOBAL linker daemon spawn/ensure (singleton, not session-keyed). Mirrors
//! [`super::oauth`] exactly: no build-skew fingerprint probe for v1, no heavy
//! children.

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::ipc::SyncIpcStream;
use crate::model::store;

use super::{StopSignal, SIGNAL_GRACE, SPAWN_CONNECT_TIMEOUT, SPAWN_POLL_INTERVAL};

/// Whether the GLOBAL linker daemon is currently ALIVE, by the bind-as-oracle
/// rule: try to CONNECT to its singleton socket ([`store::linker_daemon_sock_path`]).
#[allow(dead_code)]
pub fn linker_daemon_alive() -> bool {
    let Ok(path) = store::linker_daemon_sock_path() else {
        return false;
    };
    SyncIpcStream::connect(&path).is_ok()
}

/// Spawn a DETACHED `koma --linker-daemon` child and return its PID.
fn spawn_linker_daemon() -> Result<u32> {
    let exe = std::env::current_exe().context("cannot resolve current executable path")?;

    let mut cmd = Command::new(exe);
    cmd.arg("--linker-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd
        .spawn()
        .context("failed to spawn `koma --linker-daemon`")?;
    Ok(child.id())
}

/// Spawn a detached `koma --linker-daemon` and POLL the bind-as-oracle liveness
/// until it accepts, up to [`SPAWN_CONNECT_TIMEOUT`].
fn spawn_linker_and_wait_until_alive(path: &Path) -> Result<()> {
    let pid = spawn_linker_daemon()?;
    let deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    loop {
        match SyncIpcStream::connect(path) {
            Ok(_stream) => return Ok(()),
            Err(_) if Instant::now() < deadline => std::thread::sleep(SPAWN_POLL_INTERVAL),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "spawned linker daemon (pid {pid}) did not start accepting on {} within {:?}: {e}",
                    path.display(),
                    SPAWN_CONNECT_TIMEOUT
                ));
            }
        }
    }
}

/// Read the advisory PID from the GLOBAL linker daemon's pidfile.
pub(super) fn read_linker_pidfile() -> Option<u32> {
    let path = store::linker_daemon_pid_path().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    contents.trim().parse::<u32>().ok()
}

/// Best-effort unlink of the GLOBAL linker daemon's socket + pidfile.
pub(super) fn unlink_linker_daemon_files() {
    #[cfg(unix)]
    if let Ok(sock) = store::linker_daemon_sock_path() {
        let _ = std::fs::remove_file(sock);
    }
    if let Ok(pid) = store::linker_daemon_pid_path() {
        let _ = std::fs::remove_file(pid);
    }
}

/// Poll the GLOBAL linker daemon's bind-as-oracle liveness until it stops
/// accepting or `timeout` elapses.
fn linker_wait_until_dead(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !linker_daemon_alive() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(SPAWN_POLL_INTERVAL);
    }
}

/// Windows graceful linker-daemon stop: connect the singleton linker pipe and
/// frame-write a [`LinkerRequest::Shutdown`].
#[cfg(windows)]
fn send_linker_shutdown_request() {
    use std::io::Write;

    let Ok(sock) = store::linker_daemon_sock_path() else {
        return;
    };
    let Ok(mut stream) = SyncIpcStream::connect(&sock) else {
        return;
    };
    let Ok(payload) = serde_json::to_vec(&crate::ipc::linker_proto::LinkerRequest::Shutdown) else {
        return;
    };
    let prefix = (payload.len() as u32).to_be_bytes();
    let _ = stream.write_all(&prefix);
    let _ = stream.write_all(&payload);
    let _ = stream.flush();
}

/// Ensure the GLOBAL linker daemon is RUNNING and accepting on its singleton
/// socket, spawning one as needed. Simpler than
/// [`super::mcp::ensure_mcp_daemon_running`]: no build-skew fingerprint probe
/// for v1. Just probe-or-clear + spawn if needed.
pub fn ensure_linker_daemon_running() -> Result<()> {
    let path = store::linker_daemon_sock_path()?;
    if super::probe_or_clear(&path)? {
        return Ok(()); // already live
    }
    spawn_linker_and_wait_until_alive(&path)
}

/// Stop the GLOBAL linker daemon (best-effort). SIGTERM → wait → SIGKILL
/// escalation, mirroring [`super::oauth::stop_oauth_daemon`].
///
/// When `quiet` is `true`, ALL terminal output is suppressed.
pub fn stop_linker_daemon(quiet: bool) {
    if !linker_daemon_alive() {
        unlink_linker_daemon_files();
        if !quiet {
            println!("koma daemon: Linker daemon not running");
        }
        return;
    }

    let Some(pid) = read_linker_pidfile() else {
        unlink_linker_daemon_files();
        if !quiet {
            println!(
                "koma daemon: Linker daemon still up but no pidfile to signal; removed stale \
                 socket/pidfile. If a process is still running, stop it manually."
            );
        }
        return;
    };

    // Graceful terminate, then wait.
    #[cfg(unix)]
    super::send_signal(pid, StopSignal::Term);
    #[cfg(windows)]
    send_linker_shutdown_request();
    if linker_wait_until_dead(SIGNAL_GRACE) {
        unlink_linker_daemon_files();
        if !quiet {
            println!("koma daemon: stopped linker daemon (SIGTERM to pid {pid})");
        }
        return;
    }

    // SIGKILL (last resort), then wait.
    super::send_signal(pid, StopSignal::Kill);
    let died = linker_wait_until_dead(SIGNAL_GRACE);
    unlink_linker_daemon_files();
    if !quiet {
        if died {
            println!("koma daemon: killed linker daemon (SIGKILL to pid {pid})");
        } else {
            println!(
                "koma daemon: sent SIGKILL to pid {pid} (linker daemon) but the socket is still \
                 up; removed socket/pidfile. The process may be unkillable (zombie/stuck IO)."
            );
        }
    }
}

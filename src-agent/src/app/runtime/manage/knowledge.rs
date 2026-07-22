//! GLOBAL knowledge daemon spawn/ensure (singleton, not session-keyed).
//!
//! Mirrors [`super::mcp`] — one knowledge daemon serves all sessions. Sessions
//! push facts and query for graph-expanded recall over `~/.koma/knowledge.sock`.
//!
//! # Phase 1 (this commit): liveness probe only
//!
//! `knowledge_daemon_alive()` — bind-as-oracle liveness check — so session
//! integration can gate "is the daemon up?" before attempting an IPC call.
//! The full `ensure_knowledge_daemon_running()` (spawn, fingerprint probe, restart)
//! is deferred to the session-integration phase when sessions actually spawn it.

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::ipc::SyncIpcStream;
use crate::model::store;

/// Whether the GLOBAL knowledge daemon is currently ALIVE, by bind-as-oracle:
/// try to CONNECT to `~/.koma/knowledge.sock`. A successful connect proves the
/// daemon is accepting; refused / not-found proves it is not.
pub fn knowledge_daemon_alive() -> bool {
    let Ok(path) = store::knowledge_daemon_sock_path() else {
        return false;
    };
    SyncIpcStream::connect(&path).is_ok()
}

/// Spawn a DETACHED `koma --knowledge-daemon` child and return its PID.
///
/// Same detach strategy as [`super::mcp::spawn_mcp_daemon`]: re-exec the current
/// binary, setsid, stdio → /dev/null.
pub fn spawn_knowledge_daemon() -> anyhow::Result<u32> {
    let exe =
        std::env::current_exe().map_err(|e| anyhow::anyhow!("cannot resolve current exe: {e}"))?;

    let mut cmd = Command::new(exe);
    cmd.arg("--knowledge-daemon")
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
        use windows_sys::Win32::System::Threading::{
            CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        };
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn `koma --knowledge-daemon`: {e}"))?;
    Ok(child.id())
}

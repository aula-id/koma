//! Orphan daemon process sweep (PID-based, `koma daemon kill` only). Split out
//! of [`super`] (the `manage` module) for file size — pure code motion, no
//! behaviour change.
//!
//! Everything ELSE in `manage` deliberately avoids `/proc`/PID-based liveness in favor
//! of bind-as-oracle (see the module doc): a stale PID can be reused by an unrelated
//! process. This module is the ONE deliberate exception, scoped tightly to `koma daemon
//! kill`: an explicit "find and kill every process that looks like a koma daemon" sweep,
//! used only to catch orphans the socket scan structurally cannot see (socket file
//! removed out from under a still-running daemon, or a daemon spawned by an
//! older/different-path build). It never decides whether a *session* is alive — that
//! remains the socket's job everywhere else.
//!
//! `kill_orphan_daemon_processes` is bumped to `pub(super)` (was private) — called
//! from `manage::commands::cmd_kill`.

use std::time::Instant;

use super::{SIGNAL_GRACE, SPAWN_POLL_INTERVAL};

/// Whether `pid` is still alive, via `kill(pid, 0)` (sends no signal, only validates
/// the pid). A zero return means the process exists. `ESRCH` means it is gone. `EPERM`
/// (exists but owned by someone else) is treated as ALIVE too — the process is
/// definitely still running, we just may not be able to signal it.
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 is the standard "does this pid exist" probe; it sends nothing
    // and has no memory-safety preconditions. FFI types match libc's signature.
    let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if ret == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Best-effort sweep of orphan koma daemon processes that the socket scan
/// ([`super::live_session_sockets`] / [`super::mcp::mcp_daemon_alive`]) misses — e.g. the socket file was
/// removed out from under a still-running daemon, or the daemon was spawned by an
/// older/different-path binary that this build's socket paths don't line up with.
///
/// Scans `/proc` directly: for every numeric `/proc/<pid>` entry, reads
/// `/proc/<pid>/cmdline` and matches a process whose `argv[0]` BASENAME equals our own
/// executable's basename (e.g. `"koma"`) AND whose args contain `--daemon` or
/// `--mcp-daemon`. Matching on the basename (not the full exe path) is deliberate — it
/// is exactly what lets this catch a daemon spawned by an older build installed at a
/// different path, which is the whole point of this sweep. `koma daemon kill` itself is
/// never matched: its argv is `["koma", "daemon", "kill"]`, which has no
/// `--daemon`/`--mcp-daemon` token, regardless of basename.
///
/// SIGTERMs every match, briefly polls (up to [`SIGNAL_GRACE`]) for them to exit, then
/// SIGKILLs any survivor. Never panics: any unreadable/unparseable `/proc` entry is
/// simply skipped (best-effort, matching this module's robustness contract). Returns
/// the number of processes matched.
#[cfg(target_os = "linux")]
pub(super) fn kill_orphan_daemon_processes() -> usize {
    let self_pid = std::process::id();

    // Our own exe's basename, e.g. "koma". If it can't be resolved, fall back to the
    // known binary name so the sweep still runs.
    let our_basename = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "koma".to_string());

    let entries = match std::fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut matched: Vec<u32> = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue; // not a pid directory (e.g. "self", "net", ...)
        };
        if pid == self_pid {
            continue;
        }

        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue; // process gone / unreadable between the scan and the read
        };
        // argv is NUL-separated with a trailing NUL; the empty-string filter drops
        // that trailing tail (and any other empty argv entries harmlessly).
        let args: Vec<String> = raw
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        let Some(argv0) = args.first() else {
            continue;
        };
        let argv0_basename = argv0.rsplit('/').next().unwrap_or(argv0);
        if argv0_basename != our_basename {
            continue;
        }
        if !args.iter().any(|a| a == "--daemon" || a == "--mcp-daemon") {
            continue;
        }

        matched.push(pid);
    }

    if matched.is_empty() {
        return 0;
    }

    for &pid in &matched {
        super::send_signal(pid, libc::SIGTERM);
    }

    // Poll until every matched pid has exited or SIGNAL_GRACE elapses.
    let deadline = Instant::now() + SIGNAL_GRACE;
    while Instant::now() < deadline && matched.iter().any(|&pid| pid_alive(pid)) {
        std::thread::sleep(SPAWN_POLL_INTERVAL);
    }

    // Last resort for anything still alive.
    for &pid in &matched {
        if pid_alive(pid) {
            super::send_signal(pid, libc::SIGKILL);
        }
    }

    matched.len()
}

/// Non-Linux stub: no `/proc`, so orphan process discovery is unsupported. `koma
/// daemon kill` still works via the normal socket-based path; this just means it can't
/// catch a socket-less orphan on platforms without `/proc`.
#[cfg(not(target_os = "linux"))]
pub(super) fn kill_orphan_daemon_processes() -> usize {
    0
}

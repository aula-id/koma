//! Per-session locking.
//!
//! A running instance marks its active session with a `session.lock` file inside
//! the session directory; the file holds the owner's PID. The picker reads these
//! to show a lock marker and to refuse re-entering a session that is already open
//! (including this instance's own session, which it holds the lock for).
//!
//! Crash safety is by PID liveness, not by the file's mere presence: if the PID
//! recorded in `session.lock` is no longer a live process, the lock is STALE and
//! treated as unlocked (and the stale file is swept). Liveness is probed with the
//! portable `kill(pid, 0)` idiom (see [`pid_alive`]), which works on Linux AND
//! macOS — macOS has no `/proc`, so a `/proc/<pid>` check there would read every
//! lock as stale and let two instances enter the same session (#119). All IO here
//! is best-effort — a failed read/write/remove must never crash or block the TUI.
//!
//! LOCK OWNERSHIP: these locks are owned by whichever process runs the session
//! lifecycle — the local TUI (`run`/`run_loop`) or the headless daemon
//! (`daemon_run`). Both write `std::process::id()` into the lock via `write_lock`,
//! so a daemon-owned lock holds the DAEMON's PID. The thin attach client
//! (`client_run`, `--attach`) runs NO session lifecycle — its sessions are shadow
//! copies rebuilt from daemon frames — so it MUST NOT call any lock function here
//! (`write_lock` / `remove_lock` / `is_locked`); it only renders + forwards keys,
//! and the daemon executes the real lock writes on its behalf.

use std::path::{Path, PathBuf};

/// Path to a session's lock file: `<session_dir>/session.lock`.
fn lock_path(session_dir: &Path) -> PathBuf {
    session_dir.join("session.lock")
}

/// Whether `pid` refers to a live process on this host.
///
/// Portable across Linux and macOS via the `kill(pid, 0)` idiom (#119): sending
/// signal `0` performs the kernel's permission + existence checks WITHOUT actually
/// delivering a signal. The earlier `/proc/<pid>` existence check was Linux-only —
/// macOS has no `/proc`, so it read every foreign PID as dead, marking all locks
/// stale and letting two instances enter the same session.
///
/// Interpreting the result:
/// - returns `0` → the process exists and we may signal it → ALIVE.
/// - returns `-1` with `EPERM` → the process EXISTS but is owned by another user we lack permission to signal → ALIVE.
/// - returns `-1` with `ESRCH` → no such process → DEAD.
/// - any other errno (e.g. `EINVAL`, which signal 0 won't produce) → treat as DEAD so a surprising kernel reply can never wedge a session as permanently locked.
///
/// Our own PID is alive by definition (kept as a fast path / belt-and-suspenders).
fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    // SAFETY: `kill` with signal 0 sends no signal; it only runs the existence +
    // permission checks. It has no memory-safety preconditions and the FFI types
    // match libc's signature.
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        // Signal 0 succeeded → the process exists and we can signal it.
        return true;
    }
    // kill returned -1: distinguish "exists but not ours" (EPERM) from "gone"
    // (ESRCH) by errno. EPERM means the PID is live, just owned by another user.
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

/// Write our PID into the session's lock file, overwriting any existing one.
///
/// Best-effort: IO errors are ignored (e.g. a read-only or vanished dir just
/// means the session won't appear locked — never a crash).
pub fn write_lock(session_dir: &Path) {
    let _ = std::fs::write(lock_path(session_dir), std::process::id().to_string());
}

/// Remove the session's lock file. Best-effort; a missing file or IO error is
/// ignored.
pub fn remove_lock(session_dir: &Path) {
    let _ = std::fs::remove_file(lock_path(session_dir));
}

/// Whether the session is locked by a LIVE process (this one included).
///
/// Reads the PID from `session.lock`:
/// - file missing / unreadable → NOT locked.
/// - PID parses and is alive   → locked.
/// - PID is dead or unparseable → STALE: treat as NOT locked and opportunistically
///   remove the stale file so it stops haunting future checks.
pub fn is_locked(session_dir: &Path) -> bool {
    let path = lock_path(session_dir);
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false, // no lock file (or unreadable) → not locked
    };
    match contents.trim().parse::<u32>() {
        Ok(pid) if pid_alive(pid) => true,
        // Dead PID or garbage in the file: stale lock. Sweep it (best-effort)
        // and report unlocked so a crashed instance never blocks a session.
        _ => {
            let _ = std::fs::remove_file(&path);
            false
        }
    }
}

//! Windows process liveness + termination helpers (phase B2).
//!
//! ONE `cfg(windows)` home for the raw Win32 process calls the port needs, so
//! [`crate::model::session_lock`] (lock-staleness checks) and
//! [`crate::app::runtime::manage`] (the `koma daemon kill` escalation) share a single
//! implementation instead of each re-deriving the `OpenProcess` dance. Unix uses the
//! `kill(2)` idioms in those modules directly; this file is never compiled there.

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};

/// Whether `pid` refers to a live process on this host — the Windows analogue of the
/// unix `kill(pid, 0)` idiom.
///
/// Biased toward reporting ALIVE on any ambiguity, because the primary caller
/// ([`crate::model::session_lock`]) treats "dead" as license to STEAL a session lock: a
/// false "dead" would let two instances enter one session (the #119 corruption),
/// whereas a false "alive" merely lets a stale lock linger.
///
/// - own pid → alive (fast path).
/// - `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` succeeds → inspect the exit code:
///   `GetExitCodeProcess == STILL_ACTIVE` (259) → alive; a concrete exit code → dead; a
///   failed query → alive (we hold a handle, so it exists).
/// - `OpenProcess` fails with `ERROR_ACCESS_DENIED` → the process EXISTS but is not ours
///   to open → alive.
/// - `OpenProcess` fails otherwise (typically `ERROR_INVALID_PARAMETER` for a reaped
///   pid) → dead.
///
/// Caveat: a process that genuinely exited with code 259 is misreported as alive (the
/// well-known `STILL_ACTIVE` ambiguity) — acceptable here, since it only ever biases
/// toward the safe "still locked" direction.
pub fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    // SAFETY: `OpenProcess` with a real access mask + pid has no memory-safety
    // preconditions; the returned HANDLE is null-checked and closed exactly once below.
    // FFI types match windows-sys' signatures. `GetLastError` is read immediately after
    // the failing `OpenProcess`, before any other Win32 call could overwrite it.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code: u32 = 0;
        let got = GetExitCodeProcess(handle, &mut code);
        let _ = CloseHandle(handle);
        if got == 0 {
            // Query failed but we held a handle → the object exists; bias to alive.
            return true;
        }
        code == STILL_ACTIVE as u32
    }
}

/// Best-effort forceful termination of `pid` (the Windows analogue of `SIGKILL`):
/// `OpenProcess(PROCESS_TERMINATE)` → `TerminateProcess` → `CloseHandle`. Every failure
/// (null handle = gone / not ours, terminate refused) is ignored — the `koma daemon
/// kill` escalation re-checks liveness via the pipe afterwards, exactly like the unix
/// `libc::kill` best-effort contract.
pub fn terminate_process(pid: u32) {
    // SAFETY: as `pid_alive` — validated handle, closed once, FFI types match.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return;
        }
        // Exit code 1 (non-zero) marks a forced/abnormal exit, mirroring a signal kill.
        let _ = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
    }
}

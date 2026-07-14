use std::sync::Arc;

/// Install the daemon's process-signal handling on the tokio runtime and return
/// the shared `shutting_down` flag the SYNC [`daemon_loop`] polls each tick.
///
/// One async task owns the three unix signal streams and reacts WITHOUT ever
/// touching the loop directly (the loop stays synchronous — the task only flips an
/// atomic the loop reads):
///
/// - **SIGHUP — survive a lost controlling terminal.** Registering a tokio handler
///   for SIGHUP overrides its default "terminate" disposition; the task simply
///   consumes each SIGHUP and loops, so closing the terminal that launched the
///   daemon does NOT kill it. (Full detach-from-tty spawning is the stage-7 CLI
///   machinery; here an already-running daemon just ignores SIGHUP.)
/// - **SIGTERM / SIGINT (first) — begin graceful shutdown.** Flip `shutting_down`;
///   the loop observes it next tick and runs the shared teardown (release every
///   session lock, drop the runtime, unlink socket + pidfile).
/// - **SIGTERM / SIGINT (second, while already shutting down) — hard exit.** A
///   repeated terminate/interrupt means "I asked once, stop now": skip the orderly
///   teardown and `std::process::exit(0)` immediately. Guarded by the task's own
///   local `requested` counter (no second atomic / no TOCTOU).
///
/// SIGPIPE is handled separately by the caller (`SIG_IGN`, set before any socket
/// IO) and is intentionally NOT part of this task — a dead-client write must return
/// EPIPE per-write, never reach a handler.
///
/// Registration runs inside the runtime context (`handle.enter()`) because
/// `tokio::signal::unix::signal` needs the reactor. If any stream fails to register
/// (extremely unlikely on Linux), the daemon proceeds WITHOUT that handler rather
/// than aborting — a controller's `QuitDaemon` still provides a clean stop path.
#[cfg(unix)]
pub(super) fn install_daemon_signals(
    handle: &tokio::runtime::Handle,
) -> Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::signal::unix::{signal, SignalKind};

    let shutting_down = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutting_down);

    let _enter = handle.enter();
    handle.spawn(async move {
        // Best-effort registration. If any stream can't be built (extremely
        // unlikely on Linux), the task exits and the daemon runs without signal
        // handling — a controller's `QuitDaemon` remains as a clean stop path.
        let mut hup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(_) => return, // no signal handling available; rely on QuitDaemon
        };
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Count of terminate/interrupt requests seen. 0 -> first one begins
        // graceful shutdown; >=1 -> a second one hard-exits (double-SIGTERM guard).
        let mut requested = 0u32;
        loop {
            tokio::select! {
                // SIGHUP: consume + ignore so a closed controlling terminal never
                // kills the daemon. Never sets the shutdown flag.
                _ = hup.recv() => {}
                // SIGTERM / SIGINT: first begins graceful shutdown; a second hard-exits.
                _ = term.recv() => {
                    if requested == 0 {
                        requested = 1;
                        flag.store(true, Ordering::Relaxed);
                    } else {
                        std::process::exit(0);
                    }
                }
                _ = int.recv() => {
                    if requested == 0 {
                        requested = 1;
                        flag.store(true, Ordering::Relaxed);
                    } else {
                        std::process::exit(0);
                    }
                }
            }
        }
    });

    shutting_down
}

/// Windows twin of the unix [`install_daemon_signals`] above.
///
/// Windows has no SIGHUP/SIGTERM — the console-control set is
/// CTRL_C/CTRL_BREAK/CTRL_CLOSE/CTRL_LOGOFF/CTRL_SHUTDOWN. This wires up the three that
/// `tokio::signal::windows` exposes and that a headless (`DETACHED_PROCESS`, console-less)
/// daemon can plausibly see, all flipping the SAME `shutting_down` flag the SYNC loop
/// polls:
///
/// - **`ctrl_c` (CTRL_C_EVENT / SIGINT analogue)** — first press begins graceful
///   shutdown, a second press hard-exits, mirroring the unix double-SIGTERM guard.
/// - **`ctrl_close` (CTRL_CLOSE_EVENT)** — the console/window is closing.
/// - **`ctrl_shutdown` (CTRL_SHUTDOWN_EVENT)** — the system is shutting down.
///
/// `ctrl_close`/`ctrl_shutdown` are BEST-EFFORT extra triggers: any stream that fails to
/// register is skipped (its `select!` branch parks forever), and — critically — Windows
/// may HARD-KILL the process before an async close/shutdown handler finishes running
/// (tokio #7039), so a graceful teardown from these is NOT guaranteed. The PRIMARY
/// graceful path on Windows is therefore the IPC message (`QuitDaemon` for a session
/// daemon, `McpRequest::Shutdown` for the MCP daemon) that `koma daemon kill` sends, which
/// runs entirely inside the still-alive daemon loop. There is no SIGHUP-survive equivalent
/// (a detached daemon has no controlling terminal to lose).
#[cfg(windows)]
pub(super) fn install_daemon_signals(
    handle: &tokio::runtime::Handle,
) -> Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_shutdown};

    let shutting_down = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutting_down);

    let _enter = handle.enter();
    handle.spawn(async move {
        // Best-effort registration of each stream. A `None` (registration failed) makes
        // that branch park forever via `pending()`, so the others still work; if ALL
        // three are `None` the task idles and the daemon relies on the IPC shutdown
        // message — the exact fallback the unix task takes when signal registration fails.
        let mut cc = ctrl_c().ok();
        let mut close = ctrl_close().ok();
        let mut shutdown = ctrl_shutdown().ok();

        let mut requested = 0u32;
        loop {
            tokio::select! {
                // CTRL_C_EVENT: first begins graceful shutdown, a second hard-exits.
                _ = async {
                    match cc.as_mut() {
                        Some(s) => { s.recv().await; }
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    if requested == 0 {
                        requested = 1;
                        flag.store(true, Ordering::Relaxed);
                    } else {
                        std::process::exit(0);
                    }
                }
                // CTRL_CLOSE_EVENT: console/window closing — begin graceful shutdown
                // (best-effort; Windows may hard-kill before cleanup finishes, #7039).
                _ = async {
                    match close.as_mut() {
                        Some(s) => { s.recv().await; }
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    flag.store(true, Ordering::Relaxed);
                }
                // CTRL_SHUTDOWN_EVENT: system shutting down — same best-effort trigger.
                _ = async {
                    match shutdown.as_mut() {
                        Some(s) => { s.recv().await; }
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    flag.store(true, Ordering::Relaxed);
                }
            }
        }
    });

    shutting_down
}

/// Windows-only: arm a "kill the whole tree when I die" safety net (phase B2).
///
/// Creates an anonymous Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assigns
/// THIS (daemon) process to it, and INTENTIONALLY LEAKS the handle so the job lives
/// exactly as long as the process. Child processes the daemon spawns later (shell
/// subprocesses, MCP stdio servers) are AUTO-ASSIGNED to the job by the kernel — Windows
/// associates a new process with its creator's job unless the child is created
/// `CREATE_BREAKAWAY_FROM_JOB` (which koma never sets) — so they belong to this job too.
/// When the daemon dies for ANY reason (graceful exit OR a hard `TerminateProcess`, which
/// skips Rust teardown), the kernel closes the last job handle and `KILL_ON_JOB_CLOSE`
/// terminates every process still alive in the job — closing the orphaned-child gap a
/// hard kill would otherwise leave on Windows (there is no process-group `SIGKILL` like
/// unix).
///
/// Entirely best-effort: any failure just leaves the net un-armed (the daemon still
/// runs). CAVEAT: assigning self to a job while ALREADY in one creates a NESTED job, which
/// requires Windows 8+ — on older Windows `AssignProcessToJobObject` fails and the net is
/// simply absent (acceptable; koma targets modern Windows). Call this at daemon startup
/// BEFORE any child is spawned so the whole tree is covered.
#[cfg(windows)]
pub(super) fn install_killtree_job() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: every Win32 call uses correct arg types; the job handle is null-checked and
    // CloseHandle'd ONLY on the failure paths. On SUCCESS the handle is deliberately NOT
    // closed (leaked) so the kernel holds the job open for the whole process lifetime.
    // `zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()` is valid — it is a plain repr(C)
    // POD whose all-zero state means "no limits", onto which we set only the one flag.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            let _ = CloseHandle(job);
            return;
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            // Most likely an already-jobbed process on pre-Win8 (no nested jobs). Release
            // the unused job rather than leak a net that would protect nothing.
            let _ = CloseHandle(job);
            return;
        }
        // SUCCESS: `job` (a Copy raw HANDLE, no Drop) goes out of scope WITHOUT a
        // CloseHandle — the intentional leak. The kernel closes it when this process
        // dies, firing KILL_ON_JOB_CLOSE on any surviving child.
    }
}

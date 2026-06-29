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

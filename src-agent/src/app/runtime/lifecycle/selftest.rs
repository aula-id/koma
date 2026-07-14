//! The `koma --daemon-selftest` end-to-end harness. Split out of
//! [`super`] (the `lifecycle` module) for file size — pure code motion, no
//! behaviour change. `run_daemon_selftest` is re-exported from `lifecycle`
//! (`pub use selftest::run_daemon_selftest;`) so the existing
//! `crate::app::runtime::lifecycle::run_daemon_selftest` path (re-exported
//! further up through `app::runtime::run_daemon_selftest` /
//! `crate::app::run_daemon_selftest`) keeps resolving unchanged.

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

use super::super::event_loop::daemon::{daemon_loop, DaemonHub};

/// End-to-end daemon self-test (`koma --daemon-selftest`): drive the FULL stage-5
/// stack — bind + accept loop + per-client tasks + the real [`daemon_loop`] hub —
/// over a real unix socket, with NO terminal and NO network/session.
///
/// It proves a client request reaches the daemon and DRIVES it: a client connects,
/// `Attach`es (and gets a full `Snapshot`), sends `SubmitInput` (which the daemon
/// applies through the SAME `Action::Submit` path the TUI uses — here, with no
/// active session, that lands as the `"no active session"` status line), and then
/// observes a `StatusChanged` `Delta` carrying exactly that new status — i.e. the
/// resulting state change folds back to the client. Finally `QuitDaemon` makes the
/// real loop return so the driver thread joins cleanly.
///
/// A dedicated socket path keeps it from colliding with a live daemon. The hub +
/// `daemon_loop` run on a std thread (the loop is synchronous); the client side runs
/// on a private tokio runtime here. Prints `OK` / `FAIL` and exits 0 / 1 — it never
/// returns normally (a short-circuit CLI mode, like the IPC self-test).
pub fn run_daemon_selftest() -> ! {
    let code = match daemon_selftest_inner() {
        Ok(()) => {
            println!("koma daemon-selftest: OK");
            0
        }
        Err(e) => {
            eprintln!("koma daemon-selftest: FAIL: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

/// The fallible body of [`run_daemon_selftest`].
fn daemon_selftest_inner() -> Result<()> {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;

    use crate::ipc::frame::{read_frame, write_frame, FrameReader};
    use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, StateDelta};

    // Ignore SIGPIPE for parity with the real daemon (a dead client write must not
    // kill us). SAFETY: SIG_IGN on SIGPIPE is async-signal-safe and touches no Rust
    // state — the same call `run_daemon` makes. SIGPIPE doesn't exist on Windows.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // Dedicated endpoint so the test never disturbs a live daemon. The bind needs a
    // tokio reactor, so enter the runtime context for the bind + spawn below. Unix uses
    // a socket file; windows uses a dedicated named pipe (not a filesystem object).
    #[cfg(unix)]
    let sock_path = crate::model::store::base_dir()?.join("daemon-selftest.sock");
    #[cfg(windows)]
    let sock_path = std::path::PathBuf::from(r"\\.\pipe\koma-daemon-selftest");
    let (mut hub, req_tx) = DaemonHub::new();
    {
        let _enter = handle.enter();
        let listener = crate::ipc::server::bind(&sock_path)?;
        handle.spawn(crate::ipc::server::accept_loop(listener, req_tx));
    }

    // Drive the REAL `daemon_loop` on a std thread (it is synchronous). A fresh
    // headless state with one foreground session and NO client (so `SubmitInput`
    // exercises the no-session branch, which still mutates the status line).
    let loop_handle = handle.clone();
    let driver = std::thread::spawn(move || {
        let mut state = AppState::new(Mode::Chat);
        let mut client: Option<Arc<OpenRouterClient>> = None;
        // Signals don't apply to the self-test (it stops via QuitDaemon), so pass a
        // flag that is never set; only the hub's QuitDaemon path drives the exit.
        let never = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        daemon_loop(&mut state, &mut client, &loop_handle, &mut hub, &never);
    });

    // Client side: connect, attach, submit, observe, quit.
    let result: Result<()> = rt.block_on(async {
        let mut stream = crate::ipc::client::connect(&sock_path).await?;
        let mut reader = FrameReader::new();

        // Attach -> expect a `Hello` (build-skew handshake, task #142) FOLLOWED by a
        // full Snapshot. Read frames until the Snapshot, tolerating the leading Hello
        // (and any interleaved control frame) so the test mirrors a real client.
        let attach =
            serde_json::to_vec(&ClientRequest::Attach { foreground_id: None, cwd: None })?;
        write_frame(&mut stream, &attach).await?;
        let mut saw_snapshot = false;
        for _ in 0..8 {
            let frame: DaemonFrame =
                serde_json::from_slice(&read_frame(&mut stream, &mut reader).await?)?;
            match frame.event {
                DaemonEvent::Snapshot(_) => {
                    saw_snapshot = true;
                    break;
                }
                // The leading Hello (or any other control frame) is expected before
                // the Snapshot — keep reading.
                _ => continue,
            }
        }
        anyhow::ensure!(saw_snapshot, "attach reply never produced a Snapshot");

        // SubmitInput -> the daemon applies Action::Submit; with no active session
        // it sets status = "no active session". Read frames until that status
        // change folds back as a Delta (skipping the request's own Ack, which may
        // interleave). Bounded so a missing delta fails the test instead of hanging.
        let submit = serde_json::to_vec(&ClientRequest::SubmitInput { text: "hi".into() })?;
        write_frame(&mut stream, &submit).await?;

        let mut saw_status = false;
        for _ in 0..50 {
            let buf = tokio::time::timeout(Duration::from_secs(5), async {
                read_frame(&mut stream, &mut reader).await
            })
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for the SubmitInput status delta"))??;
            let frame: DaemonFrame = serde_json::from_slice(&buf)?;

            match frame.event {
                DaemonEvent::Delta(StateDelta::StatusChanged { session_id, text }) => {
                    anyhow::ensure!(session_id.is_none(), "expected a GLOBAL status delta");
                    anyhow::ensure!(
                        text == "no active session",
                        "unexpected status text after SubmitInput: {text:?}"
                    );
                    saw_status = true;
                    break;
                }
                // A full resync is also a valid carrier of the change; accept it.
                DaemonEvent::Snapshot(s) => {
                    if s.global.status == "no active session" {
                        saw_status = true;
                        break;
                    }
                }
                // Ack for the request / unrelated deltas: keep reading.
                _ => {}
            }
        }
        anyhow::ensure!(saw_status, "never observed the SubmitInput status change");

        // QuitDaemon -> the real loop latches shutdown and returns; expect an Ack.
        let quit = serde_json::to_vec(&ClientRequest::QuitDaemon)?;
        write_frame(&mut stream, &quit).await?;
        // Drain a couple frames to find the Ack (deltas may interleave). Best-effort.
        for _ in 0..10 {
            match tokio::time::timeout(Duration::from_secs(5), async {
                read_frame(&mut stream, &mut reader).await
            })
            .await
            {
                Ok(Ok(buf)) => {
                    let f: DaemonFrame = serde_json::from_slice(&buf)?;
                    if matches!(f.event, DaemonEvent::Ack) {
                        break;
                    }
                }
                // Socket closed (daemon already tore down) is acceptable post-quit.
                _ => break,
            }
        }
        drop(stream);
        Ok(())
    });

    // The driver thread exits once `daemon_loop` observes the QuitDaemon shutdown
    // flag. Join it (bounded) so a wedged loop surfaces as a test failure. Use a
    // small channel to time-box the join.
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let _ = driver.join();
        let _ = done_tx.send(());
    });
    let joined = matches!(
        done_rx.recv_timeout(Duration::from_secs(10)),
        Ok(()) | Err(RecvTimeoutError::Disconnected)
    );

    // Clean up the socket regardless (best-effort). Unix-only: a Windows named pipe is
    // not a filesystem object and is released when its handles drop.
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock_path);

    result?;
    anyhow::ensure!(joined, "daemon_loop did not return after QuitDaemon");
    Ok(())
}

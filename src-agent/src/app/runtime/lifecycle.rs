use std::io::stdout;
use std::sync::Arc;

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{KeyInputForm, Mode, PickerState};
use crate::app::resolve::resolve_role;
use crate::app::state::AppState;
use crate::config::DEFAULT_MODEL;
use crate::model::app_config::ModelRole;
use crate::model::{app_config::AppConfig, settings::Settings, store};
use crate::service::openrouter::OpenRouterClient;

use super::terminal::TerminalGuard;
use super::event_loop::run_loop;
use super::event_loop::daemon::{daemon_loop, DaemonHub};
use super::session_mgmt::{build_client, warm_session};
use super::signals::install_daemon_signals;

/// Best-effort prefill of (api_key, model, provider) from the most-recently-modified
/// session that has a non-empty key. Ignores all errors.
pub(super) fn prefill_creds() -> (Option<String>, Option<String>, Option<String>) {
    let metas = match store::list_sessions() {
        Ok(m) => m,
        Err(_) => return (None, None, None),
    };
    let Some(meta) = metas.into_iter().next() else {
        return (None, None, None);
    };
    let settings = match Settings::load(&meta.path.join("settings.json")) {
        Ok(s) => s,
        Err(_) => return (None, None, None),
    };
    if settings.api_key.is_empty() {
        (None, None, None)
    } else {
        (Some(settings.api_key), Some(settings.model), Some(settings.provider))
    }
}

/// The shared startup prefix for BOTH the interactive TUI ([`run`]) and the
/// headless daemon ([`daemon_run`]).
///
/// Does everything that is independent of the terminal: ensure the config dirs
/// exist, build the tokio runtime + clone its handle, decide the initial
/// [`AppState`] (resume picker / returning-user chat / first-run wizard), load the
/// global config, capture the launch cwd for the harness workspace check, build
/// the keyless per-session client when a usable Main route resolves, and warm the
/// active session (workspace reindex + awareness). Returns the owned runtime (kept
/// alive + dropped LAST by the caller), its handle, the constructed state, and the
/// optional client (the no-client-no-send gate).
///
/// SAFE FOR HEADLESS USE: nothing here touches stdout / the terminal. `warm_session`
/// only spawns background tasks + mutates state + does best-effort lock IO, so the
/// daemon path can call this identically to the TUI path.
fn build_startup(
    opts: &crate::cli::Opts,
) -> Result<(
    tokio::runtime::Runtime,
    tokio::runtime::Handle,
    AppState,
    Option<Arc<OpenRouterClient>>,
)> {
    store::ensure_dirs()?;

    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Load the global config UP FRONT — before the first-run decision below — so the
    // gate can ask the real question ("does the user have a usable Main route?")
    // against the global provider/model catalogue, not just the legacy
    // `settings.api_key` field. `ensure_dirs` has already run (so the dir exists if we
    // later persist config.json); `AppConfig::load()` is a pure read and falls back to
    // `AppConfig::default()` on any error. Stashed into `state.rest.config` below at the
    // point the old code loaded it (so the MCP wiring that reads it is unchanged).
    let config = AppConfig::load();

    // Decide initial state.
    let mut state = if opts.resume {
        let metas = store::list_sessions()?;
        let (lk, lm, lp) = prefill_creds();
        let mut state = AppState::new(Mode::SessionPicker(PickerState::new(metas)));
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    } else {
        let (lk, lm, lp) = prefill_creds();
        // "Is this user configured?" is whether the MAIN role resolves to a route
        // with a non-empty api_key — `resolve_role` consults the global
        // `config.providers`/`config.models` AND the legacy per-field fallback, so a
        // populated ~/.koma/config.json (provider + Main model) counts even with no
        // legacy session key, and a legacy-only session still counts via the fallback.
        // Probe with a Settings reflecting the prefilled legacy creds so the fallback
        // path has them to work with.
        let probe = Settings {
            api_key: lk.clone().unwrap_or_default(),
            model: lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            provider: lp.clone().unwrap_or_default(),
            ..Default::default()
        };
        let key_known = resolve_role(&config, &probe, ModelRole::Main)
            .is_some_and(|r| !r.api_key.is_empty());
        let mut state = if key_known {
            // Returning user: spawn a fresh session pre-loaded with the last
            // creds and drop straight into chat. The credential prompt only
            // appears on the very first run. Per-session changes via /settings.
            let mut st = AppState::new(Mode::Chat);
            match store::create_session() {
                Ok(mut sess) => {
                    sess.settings.api_key = lk.clone().unwrap_or_default();
                    sess.settings.model =
                        lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());
                    sess.settings.provider = lp.clone().unwrap_or_default();
                    let _ = sess.save();
                    let sess_path = sess.path.clone();
                    st.rest.fg_mut().session = Some(sess);
                    // Fresh startup session → seed ITS OWN counters (0 here, since a
                    // brand-new session has no ledger yet); harmless and explicit.
                    let fg = st.rest.foreground;
                    st.rest.load_token_totals(fg, &sess_path);
                    // Every session spawn kicks a fresh NON-BLOCKING version check;
                    // the result lands in `latest_version` when (if) it succeeds.
                    if let Some(tx) = st.rest.version_tx.as_ref() {
                        crate::app::version::spawn_check(tx.clone());
                    }
                }
                Err(e) => {
                    // Couldn't create the session dir — fall back to the prompt.
                    st.rest.status = format!("error: {e}");
                    *st.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(
                        lk.clone().unwrap_or_default(),
                        lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                        true,
                        false,
                    ));
                }
            }
            st
        } else {
            // First ever run on this machine: prompt for credentials (lazy — no
            // session dir is created until the user confirms).
            AppState::new(Mode::KeyInput(KeyInputForm::prefilled(
                lk.clone().unwrap_or_default(),
                lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                true,  // first_run
                false, // from_picker
            )))
        };
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    };

    // Stash the global config loaded up front (before the first-run gate). Moved here
    // — at the original load point — so the MCP wiring below that reads
    // `state.rest.config.mcp_servers` is unchanged. No second AppConfig::load().
    state.rest.config = config;

    // Build the GLOBAL MCP client manager from the configured servers and stash it
    // in AppStateRest (cloned into every ToolCtx so `mcp__*` calls can dispatch).
    // NON-BLOCKING: `connect_all` returns immediately and connects each enabled
    // server in a background task on the runtime; tools appear once a server is
    // ready. With no `mcp_servers` configured this spawns nothing and advertises no
    // tools — behaviour is identical to a build without MCP.
    state.rest.mcp_manager = Some(crate::app::mcp::McpManager::connect_all(
        &handle,
        &state.rest.config.mcp_servers,
    ));

    // Build the security-daemon client. Mint a per-process token and, if the
    // daemon is installed, auto-start it (M1: gated only on install; a later
    // milestone adds the /security enabled-toggle gate). Non-blocking.
    let sec_token = uuid::Uuid::new_v4().to_string();
    let sec = crate::app::sec::SecDaemonManager::new(&handle);
    // Auto-start is gated on BOTH install and the runtime enable flag. The flag
    // starts `false` so the daemon stays off by default; the `/security` panel's
    // toggle key (`t`) sets it and calls `.start()` explicitly.
    if crate::security::is_installed() && state.rest.security_enabled {
        sec.start(sec_token.clone());
    }
    state.rest.sec_token = sec_token;
    state.rest.sec_manager = Some(sec);

    // Capture the process launch directory for the harness workspace check (WC).
    // This folder is always an allowed workspace regardless of the allow-list.
    if let Ok(cwd) = std::env::current_dir() {
        state.rest.launch_dir = cwd;
    }

    // If startup opened a session straight into chat (returning user), build its
    // client now; otherwise it's built when the user confirms credentials. The
    // None-gate is the "is there a usable session/key?" signal the whole runtime
    // relies on. Because the key now lives in config/settings (read per-call), the
    // condition is whether the MAIN role resolves to a usable route (a route with a
    // non-empty api_key) — NOT the old `!settings.api_key.is_empty()`. The client
    // itself is keyless; the gate just preserves the no-client-no-send invariant.
    let client: Option<Arc<OpenRouterClient>> = state
        .rest
        .fg()
        .session
        .as_ref()
        .filter(|s| {
            resolve_role(&state.rest.config, &s.settings, ModelRole::Main)
                .is_some_and(|r| !r.api_key.is_empty())
        })
        .map(|_| build_client());

    // Warm the session (reindex workspace + compute awareness summary) so a
    // cold launch is fully primed before the first keystroke. Picker / first-run
    // paths have no session yet; warm_session is a no-op for them and fires
    // later when a session becomes active (picker-select / creds-confirm / /new).
    warm_session(&mut state, &client, &handle);

    Ok((rt, handle, state, client))
}

/// Release every live session's on-disk lock, then drop the tokio runtime LAST.
///
/// Shared clean-exit teardown for both the TUI and daemon paths. Multi-session
/// aware — a quit (kill-all OR detach) can leave several sessions holding locks,
/// so releasing only the foreground's would strand the rest until PID-liveness
/// staleness kicked in. Dropping `rt` last cancels every spawned task; each task
/// owns the sender of its own per-request channel, and `let _ =` on each send
/// makes a post-drop send a safe no-op (no panic, no deadlock). A crash that skips
/// this is covered by PID-liveness staleness in `store::is_locked`.
fn shutdown_runtime(state: &mut AppState, rt: tokio::runtime::Runtime) {
    for s in &mut state.rest.sessions {
        if let Some(p) = s.held_lock.take() {
            crate::model::store::remove_lock(&p);
        }
    }
    drop(rt);
}

pub fn run(opts: crate::cli::Opts) -> Result<()> {
    // Shared, terminal-independent startup (dirs, runtime, state, client, warm).
    let (rt, handle, mut state, mut client) = build_startup(&opts)?;

    // Terminal setup. Guard created BEFORE the Terminal so its Drop covers a
    // failing Terminal::new, any later `?`-error, and panic-unwind.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    // Clear the alternate screen so no shell scrollback bleeds through the
    // cells the UI never paints (e.g. the empty part of the transcript).
    terminal.clear()?;

    let result = run_loop(&mut terminal, &mut state, &handle, &mut client);

    // Terminal teardown is handled by `_guard`'s Drop at function scope.
    // Release all session locks, then drop the runtime LAST (runs on Ok and Err).
    shutdown_runtime(&mut state, rt);

    result
}

/// Headless entry point: run the koma-daemon event loop with NO terminal.
///
/// Shares [`build_startup`] with the TUI [`run`] (same dirs / runtime / state /
/// client / warm), then — instead of the terminal + `run_loop` — ignores SIGPIPE,
/// installs the SIGHUP-survive + graceful/double-SIGTERM signal task, records the
/// pidfile, binds the unix socket, spawns the per-client accept loop, and enters
/// [`daemon_loop`]. The accept loop runs on the tokio runtime (async socket I/O);
/// `daemon_loop` runs synchronously on this thread and drains the bridge each tick
/// (critique #6). It returns when a controller sends `QuitDaemon` OR the process is
/// signalled (SIGTERM/SIGINT, via the polled `shutting_down` flag); the shared
/// teardown then releases every session lock, drops the runtime, and unlinks the
/// socket + pidfile.
pub fn run_daemon(opts: crate::cli::Opts) -> Result<()> {
    // Critique #10: writing to a dead client must never kill the daemon. Ignore
    // SIGPIPE process-wide BEFORE any socket IO so a broken-pipe write returns
    // EPIPE (handled per-write) instead of terminating the process. `libc` is a
    // direct dependency; this is the one tiny unsafe FFI call it is needed for.
    // SAFETY: `signal` with SIG_IGN on SIGPIPE is async-signal-safe and the
    // canonical way to opt out of SIGPIPE; it touches no Rust state.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Shared, terminal-independent startup — identical to the TUI path.
    let (rt, handle, mut state, mut client) = build_startup(&opts)?;

    // Install the SIGHUP-survive + graceful/double-SIGTERM signal handling and get
    // the flag the SYNC loop polls. Done BEFORE binding the socket so a signal that
    // arrives during startup is already accounted for (it sets the flag the loop
    // checks on its very first tick). The daemon now ignores SIGHUP, so closing the
    // launching terminal can't kill it.
    let shutting_down = install_daemon_signals(&handle);

    // Record the advisory pidfile (diagnostics / `kill`). Best-effort: a write
    // failure must not stop the daemon (the bound socket, not this file, is the
    // liveness oracle), so the error is swallowed. The teardown unlinks it.
    let pid_path = crate::model::store::daemon_pid_path()?;
    let _ = crate::model::store::write_daemon_pid();

    // Sync-loop <-> per-client-task bridge (critique #1/#6). The runner holds the
    // paired `req_tx` (which the accept loop clones into each connection task) for
    // the daemon's lifetime so `req_rx` never observes a premature `Disconnected`
    // before any client connects.
    let (mut hub, req_tx) = DaemonHub::new();

    // Bind the unix listener (this process becomes the live daemon — bind is the
    // liveness oracle) and spawn the accept loop onto the tokio runtime. Each
    // accepted connection gets a per-client task bridging its socket to `req_tx`.
    // `UnixListener::bind` + `handle.spawn` need a tokio reactor in scope, so enter
    // the runtime context for them. The socket path is unlinked at teardown below.
    let sock_path = crate::model::store::daemon_sock_path()?;
    {
        let _enter = handle.enter();
        let listener = crate::ipc::server::bind(&sock_path)?;
        handle.spawn(crate::ipc::server::accept_loop(listener, req_tx));
    }

    // Enter the headless loop: service_all_sessions + service_global + the request-
    // bridge drain (apply mutations) + delta streaming on the adaptive cadence.
    // Returns when a controller's QuitDaemon latches the hub flag OR a signal flips
    // `shutting_down` (both observed each tick).
    daemon_loop(&mut state, &mut client, &handle, &mut hub, &shutting_down);

    // Graceful teardown (QuitDaemon, SIGTERM/SIGINT, or a future self-exit). Dropping
    // the runtime in `shutdown_runtime` cancels the accept loop and every per-client
    // task, so no new client is serviced past this point ("stop accepting new
    // clients"); it also releases every session lock and drops the runtime LAST. Then
    // remove the socket + pidfile so the next spawn binds fresh. (A second SIGTERM
    // during this window hard-exits via the signal task instead of reaching here.)
    shutdown_runtime(&mut state, rt);
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

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
    // state — the same call `run_daemon` makes.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // Dedicated socket so the test never disturbs a live daemon. `UnixListener::bind`
    // needs a tokio reactor, so enter the runtime context for the bind + spawn.
    let sock_path = crate::model::store::base_dir()?.join("daemon-selftest.sock");
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

    // Clean up the socket regardless (best-effort).
    let _ = std::fs::remove_file(&sock_path);

    result?;
    anyhow::ensure!(joined, "daemon_loop did not return after QuitDaemon");
    Ok(())
}

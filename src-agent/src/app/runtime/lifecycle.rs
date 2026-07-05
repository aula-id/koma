use std::io::stdout;
use std::sync::Arc;

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{KeyInputForm, Mode, OnboardState, PickerState};
use crate::app::resolve::resolve_role;
use crate::app::state::{AppState, SessionRuntime};
use crate::config::DEFAULT_MODEL;
use crate::model::app_config::ModelRole;
use crate::model::session::Session;
use crate::model::session_registry;
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
    let mut state = if opts.daemon {
        // Daemon-per-session: `install_daemon_session` (called right after build_startup
        // in run_daemon) owns create/load for this daemon's keyed session id. Do NOT
        // create a throwaway returning-user session here (install would orphan it every
        // launch) and do NOT run the resume picker / `list_sessions` scan — just stash
        // last-used creds on a sessionless placeholder; install_daemon_session sets the
        // real session + mode before the first tick, so the placeholder mode is moot.
        let (lk, lm, lp) = prefill_creds();
        let mut state = AppState::new(Mode::Chat);
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    } else if opts.resume {
        let metas = store::list_sessions()?;
        let (lk, lm, lp) = prefill_creds();
        let mut state = AppState::new(Mode::SessionPicker(PickerState::new(metas)));
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    } else {
        let (lk, lm, lp) = prefill_creds();
        // "Should we onboard?" is now DISTINCT from "does Main resolve?": with the
        // always-usable koma-free Main fallback, `resolve_role(Main)` is basically never
        // unusable, so it can no longer gate the first-run chooser. Instead ask whether the
        // user has configured NOTHING routable yet — no providers/models/OAuth conns in the
        // global `~/.koma/config.json` AND no legacy session api_key. A populated config or a
        // legacy-keyed session both count as configured and skip the chooser. Probe with a
        // Settings reflecting the prefilled legacy creds so the api_key check sees them.
        let probe = Settings {
            api_key: lk.clone().unwrap_or_default(),
            model: lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            provider: lp.clone().unwrap_or_default(),
            ..Default::default()
        };
        let unconfigured = config.is_unconfigured(&probe);
        let mut state = if !unconfigured {
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
                    // Per-session status (C6); startup has the single foreground session.
                    st.rest.fg_mut().status = format!("error: {e}");
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
            // First ever run on this machine: show the connection CHOOSER (koma free /
            // provider / custom) — lazy, no session dir is created until the user picks
            // a path. Each choice routes to its own setup action (see `Mode::Onboard`).
            AppState::new(Mode::Onboard(Box::new(OnboardState { cursor: 0 })))
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

    // Build the MCP client manager from the configured servers and stash it in
    // AppStateRest (cloned into every ToolCtx so `mcp__*` calls can dispatch).
    //
    // In DAEMON mode this is left `None` here: `run_daemon` sets it up right after,
    // PROXYING to the global MCP daemon (with a local fallback) so N session-daemons
    // share one copy of every heavyweight server instead of each spawning their own.
    // Building a LOCAL manager here too would spawn a duplicate set that the proxy
    // then supersedes — so skip it for `--daemon` and let `run_daemon` decide.
    //
    // In the STANDALONE/`--local` (and returning-user TUI) path there is no global
    // daemon, so build the LOCAL manager exactly as before. NON-BLOCKING: `connect_all`
    // returns immediately and connects each enabled server in a background task; tools
    // appear once a server is ready. With no `mcp_servers` configured this spawns
    // nothing and advertises no tools — identical to a build without MCP.
    if opts.daemon {
        state.rest.mcp_manager = None;
    } else {
        state.rest.mcp_manager = Some(crate::app::mcp::McpManager::connect_all(
            &handle,
            &state.rest.config.mcp_servers,
        ));
    }

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
                .is_some_and(|r| r.is_usable())
        })
        .map(|_| build_client());

    // Warm the session (reindex workspace + compute awareness summary) so a
    // cold launch is fully primed before the first keystroke. Picker / first-run
    // paths have no session yet; warm_session is a no-op for them and fires
    // later when a session becomes active (picker-select / creds-confirm / /new).
    warm_session(&mut state, &client, &handle);

    Ok((rt, handle, state, client))
}

/// Create-or-LOAD the session `<id>` and install it as the daemon's SINGLE foreground
/// session (daemon-per-session). Called once at daemon startup, AFTER [`build_startup`]
/// and BEFORE the accept/daemon loops.
///
/// The daemon owns exactly one session — the one keyed to its socket. The client minted
/// `session_id` and passed it via `--session`, so:
/// - if the registry already has `session_id` → [`Session::load`] it from disk (resume,
///   exercised by a LATER commit);
/// - else → create a NEW session WITH THAT id via [`store::create_session_in_with_id`],
///   rooted at the daemon's current working dir (it inherited the client's cwd at spawn,
///   so `current_dir()` is the right pwd bucket). At THIS commit the minted id is always
///   new, so only the create branch runs.
///
/// Construction MIRRORS the Attach-create path `create_session_for_pwd` (inherit
/// last-used creds for a fresh session, acquire the session's lock into a fresh
/// [`SessionRuntime`], reset the flat foreground UI, seed token counters, then either
/// open KeyInput when no usable Main route resolves or land in Chat + warm) — with ONE
/// structural difference: it REPLACES the single foreground slot (`sessions[0]`) instead
/// of appending a tab, because the daemon serves one session, not a multiplexed set. Any
/// lock `build_startup` already grabbed for its placeholder/returning-user session is
/// released first so we never strand a stale lock.
///
/// Best-effort and infallible at the type level: a create/load error degrades to a
/// status line + KeyInput rather than aborting daemon startup, so a bad session can
/// never wedge the daemon before it can even report the problem to a client.
fn install_daemon_session(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    session_id: &str,
) {
    // Release any lock build_startup acquired (its placeholder / returning-user session);
    // we are about to overwrite the foreground slot with OUR keyed session.
    if let Some(old) = state.rest.fg_mut().held_lock.take() {
        store::remove_lock(&old);
    }

    // NOTE: for a returning user, `build_startup` already minted a throwaway session
    // (its `create_session()`), which this replace orphans on disk/registry. That extra
    // empty session is a pre-existing wart (the global daemon's `build_startup` seeded one
    // every launch too) and is harmless; the multiplexing-rip-out commit removes the
    // shared `build_startup` seeding, so it is intentionally left as-is here.

    // --- create-or-load resolution (quote-worthy: the create-vs-load oracle) ---
    // The registry is the source of truth for "does this session already exist?". A
    // present row → load from its on-disk dir; an absent row (or any registry error) →
    // create fresh with this exact id, rooted at the daemon's cwd.
    let loaded: Result<Session> = match session_registry::get(session_id) {
        Ok(Some(row)) => store::session_dir(&row.pwd_hash, session_id)
            .and_then(|dir| Session::load(&dir)),
        _ => {
            let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            store::create_session_in_with_id(&workdir, session_id)
        }
    };

    let mut sess = match loaded {
        Ok(s) => s,
        Err(e) => {
            // Couldn't build the session — surface it on the (placeholder) foreground and
            // drop into KeyInput so the client still reaches a usable screen.
            state.rest.fg_mut().status = format!("error: could not open session: {e}");
            *client = None;
            *state.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(
                state.rest.last_key.clone().unwrap_or_default(),
                state.rest.last_model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                true,  // first_run framing
                false, // not from picker
            ));
            return;
        }
    };

    // A FRESH session inherits the last-used creds (so it drops straight into chat, same
    // as `/new` and the attach-create path); a LOADED session already carries its own
    // persisted creds, so only fill blanks from last-used as a convenience.
    if sess.settings.api_key.is_empty() {
        sess.settings.api_key = state.rest.last_key.clone().unwrap_or_default();
    }
    if sess.settings.provider.is_empty() {
        sess.settings.provider = state.rest.last_provider.clone().unwrap_or_default();
    }
    if sess.settings.model.is_empty() {
        sess.settings.model = state
            .rest
            .last_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    }
    let _ = sess.save();

    // Acquire THIS session's lock into a fresh runtime keyed by the SAME uuid as the
    // socket — so the SessionRuntime id, the session id, and the socket key all agree.
    store::write_lock(&sess.path);
    let mut runtime = SessionRuntime::new();
    runtime.id = session_id.to_string();
    runtime.held_lock = Some(sess.path.clone());

    // Onboarding chooser ONLY when the user has configured NOTHING routable yet (no
    // providers/models/OAuth conns AND no legacy api_key) — NOT merely when Main fails to
    // resolve, since the koma-free Main fallback is always usable now. Computed before
    // `sess` moves in.
    let unconfigured = state.rest.config.is_unconfigured(&sess.settings);
    let sess_path = sess.path.clone();
    runtime.session = Some(sess);

    // Install as the SINGLE foreground session (replace the slot; never append).
    state.rest.sessions = vec![runtime];
    state.rest.foreground = 0;

    // Clean slate for the flat foreground UI (mirror create_session_for_pwd / /new).
    {
        let fg = state.rest.fg_mut();
        fg.input.clear();
        fg.cursor = 0;
        fg.pending_attachments.clear();
    }
    state.rest.reset_scroll();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.fg_mut().status = "ready".into();

    // Seed this session's cumulative token counters from its own (possibly empty) ledger.
    state.rest.load_token_totals(0, &sess_path);

    if unconfigured {
        // Nothing configured yet — show the connection CHOOSER through the client. The
        // client renders `Mode::Onboard` and forwards the pick to the daemon; each
        // choice routes to its own setup action (koma free / provider / custom).
        *client = None;
        *state.mode_mut() = Mode::Onboard(Box::new(OnboardState { cursor: 0 }));
    } else {
        *client = Some(build_client());
        // Land in Chat first, THEN warm (warm_session may upgrade to Loading).
        *state.mode_mut() = Mode::Chat;
        warm_session(state, client, handle);
    }
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

    // Daemon-per-session: `--session <id>` is REQUIRED. This daemon binds the keyed
    // socket `run/<id>.sock` and owns exactly session `<id>` (the client minted the id
    // and passed it here, so both agree on the key). Erroring clearly beats binding a
    // wrong/global socket — the spawn machinery always passes it.
    let session_id = opts.session.clone().ok_or_else(|| {
        anyhow::anyhow!("`koma --daemon` requires `--session <id>` (daemon-per-session)")
    })?;

    // Shared, terminal-independent startup — identical to the TUI path.
    let (rt, handle, mut state, mut client) = build_startup(&opts)?;

    // Take ownership of session `<id>`: create-or-load it, wrap in a SessionRuntime,
    // acquire its lock, and install it as the daemon's single foreground session —
    // BEFORE the daemon loop / accept loop start. This REPLACES whatever placeholder /
    // returning-user session `build_startup` seeded, so the daemon serves exactly the
    // session keyed to its socket. Attach no longer creates a session (the daemon
    // already owns this one); see the hub `Attach` handler.
    install_daemon_session(&mut state, &mut client, &handle, &session_id);

    // MCP for the session-daemon: PROXY to the global MCP daemon when possible, with a
    // LOCAL fallback that is never worse than today. `build_startup` left
    // `mcp_manager = None` in daemon mode so this is the sole owner of the decision.
    //
    // - No `mcp_servers` configured → leave it `None` (no manager, no global daemon
    //   spawned): byte-identical to a build without MCP.
    // - Servers configured → ensure the singleton global MCP daemon is up and connect a
    //   PROXY to it, so N session-daemons share ONE copy of every heavyweight server
    //   (e.g. `serena`) instead of each spawning their own. If EITHER the ensure/spawn OR
    //   the proxy connect fails, FALL BACK to a LOCAL `connect_all` — MCP must always
    //   work; a missing/broken global daemon degrades to today's per-session behaviour.
    if !state.rest.config.mcp_servers.is_empty() {
        let proxy = crate::model::store::mcp_daemon_sock_path().and_then(|sock| {
            super::manage::ensure_mcp_daemon_running()
                .and_then(|()| crate::app::mcp::McpManager::connect_proxy(&handle, sock))
        });
        state.rest.mcp_manager = Some(match proxy {
            // Proxying to the shared global daemon: the dedup win.
            Ok(proxy) => proxy,
            // FALLBACK: any ensure/connect failure ⇒ own the connections locally, so
            // this daemon still has working MCP (just not shared).
            Err(e) => {
                eprintln!("mcp: global daemon unavailable ({e:#}); using local servers");
                crate::app::mcp::McpManager::connect_all(&handle, &state.rest.config.mcp_servers)
            }
        });
    }

    // Install the SIGHUP-survive + graceful/double-SIGTERM signal handling and get
    // the flag the SYNC loop polls. Done BEFORE binding the socket so a signal that
    // arrives during startup is already accounted for (it sets the flag the loop
    // checks on its very first tick). The daemon now ignores SIGHUP, so closing the
    // launching terminal can't kill it.
    let shutting_down = install_daemon_signals(&handle);

    // Record the advisory pidfile (diagnostics / `kill`), keyed by this session.
    // Best-effort: a write failure must not stop the daemon (the bound socket, not this
    // file, is the liveness oracle), so the error is swallowed. The teardown unlinks it.
    let pid_path = crate::model::store::daemon_pid_path(&session_id)?;
    let _ = crate::model::store::write_daemon_pid(&session_id);

    // Sync-loop <-> per-client-task bridge (critique #1/#6). The runner holds the
    // paired `req_tx` (which the accept loop clones into each connection task) for
    // the daemon's lifetime so `req_rx` never observes a premature `Disconnected`
    // before any client connects.
    let (mut hub, req_tx) = DaemonHub::new();

    // Bind the unix listener (this process becomes the live daemon — bind is the
    // liveness oracle) and spawn the accept loop onto the tokio runtime. Each
    // accepted connection gets a per-client task bridging its socket to `req_tx`.
    // `UnixListener::bind` + `handle.spawn` need a tokio reactor in scope, so enter
    // the runtime context for them. The keyed socket path is unlinked at teardown below.
    let sock_path = crate::model::store::daemon_sock_path(&session_id)?;
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

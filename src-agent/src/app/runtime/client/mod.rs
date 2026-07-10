//! Thin attach client — the `koma --attach` core (daemon stage 6).
//!
//! [`client_run`] connects to a running daemon's unix socket, attaches, and then
//! renders the daemon's state + forwards input. It does NONE of the real work:
//! no `service_all_sessions`, no turn machinery, no agent runtime. It maintains a
//! SHADOW [`AppState`] populated PURELY from the daemon's
//! [`DaemonEvent::Snapshot`] / [`DaemonEvent::Delta`] frames and feeds that shadow
//! to the EXISTING [`crate::view::draw`] — so the attach client renders identically
//! to a local TUI, with zero second render path to drift.
//!
//! ## Module layout
//!
//! | Submodule   | Contents                                                         |
//! |-------------|------------------------------------------------------------------|
//! | `connect`   | `Connection` struct + `connect_attach_and_handshake` (sync)     |
//! | `render`    | `render_loop`, `advance_local_animations`, frame-pacing consts  |
//! | `shadow`    | `apply_frame`, `apply_snapshot`, `apply_delta`, seq-gap, clock  |
//! | `input`     | `local_echo`, `QuitConfirmKey`, quit overlay keys               |
//! | `bridge`    | `reader_task`, `writer_task`, transport consts                  |
//! | `diff`      | host-side file-diff + usage-preview computation (GUI panels)    |
//! | `git`       | host-side git-status + git-diff computation (GUI GIT panel)     |
//! | `host`      | GUI host-relay layer (`run_host_relay`, the swapper/attached FSM) |
//! | `host_config` | Pre-session (swapper) config-apply helpers for `host`           |
//! | `push_proto`| GUI push-envelope DTOs (`PushEnvelope` + the one-shot `push_*` fns) |
//! | `push_rows` | The `Push*` row/DTO structs `PushEnvelope`'s variants carry       |
//! | `project`   | GUI snapshot serialization (`serialize_and_push`, `push_hub`, `warm_status_label`) |
//! | `project_config` | GUI config projection (`ConfigProjection`, `push_config`)          |
//! | `push_loop` | The headless attached fold loop (`push_loop`, `PushState`, `HostTransition`) |

#![allow(unused_imports)]
#![allow(dead_code)]

mod connect;
mod render;
mod shadow;
mod input;
mod bridge;
mod swapper;
mod swapper_keys;
mod diff;
mod git;
mod host;
mod host_config;
mod push_proto;
mod push_rows;
mod project;
mod project_config;
mod push_loop;

use std::io::{stdout, Stdout};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{Mode, SessionHub};
use crate::ipc::proto::ClientRequest;
use crate::model::store;

use connect::{connect_attach_and_handshake, Connection};
use bridge::WRITER_FLUSH_TIMEOUT;
use swapper::{build_local_hub, run_swapper, SwapperOutcome};

use crate::app::runtime::terminal::TerminalGuard;

// Re-exported so `gui::mod`'s existing `super::client::run_host_relay` call site
// keeps resolving unchanged after `run_host_relay` moved into the sibling `host`
// module (that call site lives outside `client`, hence the `pub(in ...)` reach).
pub(in crate::app::runtime) use host::run_host_relay;

/// The client's run-loop state: either ATTACHED to a session-daemon (rendering its frames
/// and forwarding input) or running the local detached SWAPPER (the `/resume` picker).
///
/// A swap is "detach-then-pick": entering the swapper from an attached session first tears
/// the connection down (leaving that daemon cooking) so the daemon's snapshots can't
/// clobber the local hub, then runs the swapper STANDALONE; a pick attaches to the chosen
/// daemon, a cancel reconnects to the one just left.
enum ClientState {
    /// Live attached to a session-daemon.
    Attached(Connection),
    /// Detached, showing the local cross-daemon swapper.
    Swapper(SessionHub),
}

/// Restart the stale session-daemon while showing an animated "reopening" spinner.
/// The restart (`manage::restart_daemon`) is blocking (~1s), so it runs on a
/// background thread while THIS (main) thread — which owns the terminal — draws the
/// spinner each frame until the restart completes, then propagates its result.
/// Kept fully silent (quiet=true) so nothing bleeds into the alt-screen.
fn restart_daemon_animated(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    session_id: &str,
) -> Result<()> {
    let sid = session_id.to_string();
    let worker = std::thread::spawn(move || super::manage::restart_daemon(&sid, true));

    // Load the actual user config (dark/light, accent) so the spinner matches the
    // user's real theme rather than always using defaults.
    let cfg = crate::model::app_config::AppConfig::load();
    let palette = crate::view::theme::palette(&cfg);

    let mut frame: u64 = 0;
    // ~80ms per braille glyph → a calm spin; check completion each frame.
    const FRAME: Duration = Duration::from_millis(80);
    while !worker.is_finished() {
        let start = Instant::now();
        let _ = terminal.draw(|f| crate::view::loading::draw_reopening(f, frame, &palette));
        frame = frame.wrapping_add(1);
        if let Some(rem) = FRAME.checked_sub(start.elapsed()) {
            std::thread::sleep(rem);
        }
    }

    // Propagate the restart's result; a panicked worker thread → generic error.
    match worker.join() {
        Ok(res) => res.map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}")),
        Err(_) => Err(anyhow::anyhow!("reopening thread panicked during daemon restart")),
    }
}

/// Attach to a session-daemon, spawning it if needed, and run the build-skew handshake.
///
/// The single attach primitive used everywhere the client connects: the initial
/// non-resume attach, a swapper PICK, and a swapper CANCEL-reconnect. It:
/// 1. ensures the session's daemon is RUNNING ([`super::manage::ensure_daemon_running`],
///    `resume=false`): for a LIVE session this is a no-op (the daemon is already up); for
///    a brand-new minted id it spawns a fresh `--daemon --session <id>` (create branch);
///    for an on-disk history id it spawns one that create-or-LOADs that session;
/// 2. connects + attaches + runs the `Hello` handshake ([`connect_attach_and_handshake`]);
/// 3. on a CONFIRMED build-skew mismatch, restarts that one stale daemon (AT MOST ONCE)
///    via the SAME machinery `koma daemon restart` uses and reconnects.
///
/// # Build-skew auto-restart (task #142)
///
/// The koma daemon outlives a rebuild, so a freshly-built client can attach to a daemon
/// still running OLD code and silently render its stale frames (this caused a phantom
/// `/agents` bug). The client compares its OWN fresh fingerprint
/// ([`store::build_fingerprint`]) against the daemon's reported `Hello`; on a mismatch it
/// restarts the stale daemon and reconnects. LOOP GUARD: the restart fires at most once —
/// if the just-spawned daemon STILL mismatches (it shouldn't, it was built from the current
/// binary) it warns and renders against it rather than restart-looping. A daemon that sends
/// no `Hello` (slow / pre-handshake) is never restarted on that absence alone.
fn attach_session(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    handle: &tokio::runtime::Handle,
    session_id: &str,
) -> Result<Connection> {
    // Make sure a daemon owns this session before we connect. No-op when it is already
    // live (the bind-as-oracle probe inside short-circuits); spawns + waits otherwise.
    super::manage::ensure_daemon_running(session_id, false, None)
        .map_err(|e| anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}"))?;

    let sock_path = store::daemon_sock_path(session_id)?;
    let my_fingerprint = store::build_fingerprint();

    let mut conn = connect_attach_and_handshake(handle, &sock_path)?;
    let mut already_restarted = false;
    while conn
        .daemon_version
        .as_deref()
        .is_some_and(|v| v != my_fingerprint)
    {
        if already_restarted {
            eprintln!(
                "koma: daemon still reports a different build after a restart; \
                 continuing against it"
            );
            break;
        }
        already_restarted = true;

        // Tear down the stale connection's bridge before restarting: drop our request
        // sender (the writer drains + exits) and let the reader observe the daemon's
        // death as EOF. Both old tasks self-terminate; the runtime persists.
        drop(conn.req_tx);
        drop(conn.frame_rx);

        restart_daemon_animated(terminal, session_id)?;

        conn = connect_attach_and_handshake(handle, &sock_path)?;
    }
    Ok(conn)
}

/// Tear a live [`Connection`] down cleanly: queue a polite `Detach`, then drop the request
/// sender and JOIN the writer so the final frame(s) flush before the runtime is touched.
///
/// Run on EVERY exit from an [`ClientState::Attached`] — a plain exit, a render error, AND
/// the detach-then-swap into the swapper — so the source daemon is never left orphaned
/// (socket open, no controller) and never stuck mid-hub. The `/quit` overlay's `[k]`/`[d]`
/// paths may have queued their own `Detach` (and a `QuitDaemon` for `[k]`, which kills the
/// daemon outright) already; this extra `Detach` is then a harmless no-op (the daemon
/// deregistered this client by id, so a second one matches nobody). Dropping `req_tx` closes the outbound channel, which the
/// writer observes as `Disconnected`: it drains EVERY remaining queued request to the
/// socket and returns. We JOIN it (bounded by [`WRITER_FLUSH_TIMEOUT`], so a wedged socket
/// can't hang exit) BEFORE returning, which is what guarantees the shutdown frames land.
/// MUST NOT be called while a tokio runtime context is entered (it `block_on`s).
fn teardown_connection(handle: &tokio::runtime::Handle, conn: Connection) {
    let Connection {
        frame_rx: _,
        req_tx,
        writer_handle,
        prebuffered: _,
        daemon_version: _,
    } = conn;

    let _ = req_tx.send(ClientRequest::Detach);
    drop(req_tx);
    let _ = handle.block_on(async {
        tokio::time::timeout(WRITER_FLUSH_TIMEOUT, writer_handle).await
    });
}

// ─── host-relay: the GUI host IS the daemon client ───────────────────────────────
//
// The desktop GUI (`crate::app::runtime::gui`) runs a `tao`/GTK event loop on its
// main thread and CANNOT host tokio there (`event_loop.run` diverges). So the daemon
// connection + the headless fold loop run HERE on a background client-thread with its
// own tokio runtime — the daemon->JS direction pushes JSON envelopes out through the
// `push` sink (an `EventLoopProxy::send_event` closure the host supplies), and the
// JS->daemon direction arrives as [`HostCtl`] control messages + a shared `live_req`
// the ipc thread forwards `SubmitInput` through.

/// The GUI's read-only STREAM VIEW: which sub-agent / bash job the webview is currently
/// live-streaming into an Explore stream tab. Shared (`Arc<Mutex<_>>`) between the ipc
/// thread — which WRITES it when a `SetStreamView` request arrives — and the host-relay
/// fold ([`render::push_loop`] → [`render::serialize_and_push`]), which READS it every
/// frame to decide whose transcript / output tail to fold into the push (mirrors the
/// shared `live_marks`). At most one of the two is `Some` in practice (the active tab).
/// `Copy` so the fold can snapshot it out of the lock by value each frame.
#[derive(Clone, Copy, Default)]
pub(in crate::app::runtime) struct StreamView {
    /// The id of the sub-agent being streamed, or `None`.
    pub subagent: Option<usize>,
    /// The id of the bash job being streamed, or `None`.
    pub bash: Option<usize>,
}

/// Control messages from the GUI ipc thread (main tao thread) to the host-relay
/// client-thread. `SubmitInput` does NOT ride this channel — it goes straight to the
/// live daemon via the shared `live_req` sender — so this carries only the
/// session-lifecycle intents the client-thread owns.
pub(super) enum HostCtl {
    /// The webview page booted / reloaded: re-push the full authoritative state.
    Ready,
    /// Attach to this existing session UUID (a hub `SelectSession` pick).
    Select(String),
    /// Mint a fresh session UUID + attach (the hub `[+ new session]` row, or the attached
    /// chat view's "new session"). `workdir` is the folder the GUI's native picker chose as
    /// the new session's working dir; `None` (e.g. a non-GUI/empty-state new) falls back to
    /// the host's cwd (base path). `kill` (attached state only) reaps the CURRENT session's
    /// daemon as part of the switch — a graceful `QuitDaemon` on the live conn flushed by the
    /// teardown, plus an off-thread escalating ensure-death — mirroring the TUI `/new kill`;
    /// `kill: false` leaves the old daemon cooking (resumable), exactly as before.
    New {
        workdir: Option<std::path::PathBuf>,
        kill: bool,
    },
    /// Kill the session-daemon `id` (a hub row's KILL button, or the attached chat view's
    /// "kill this session"). Escalating (graceful `QuitDaemon` → SIGTERM → SIGKILL) so a
    /// wedged daemon is actually removed, run OFF the control loop (it blocks up to the grace
    /// budget). Killing the CURRENTLY-ATTACHED session additionally queues a `QuitDaemon` on
    /// the live conn + hands back to the swapper; a background kill just refreshes the hub
    /// once the daemon is confirmed dead.
    KillSession(String),
    /// Physically DELETE the history session `id` (on-disk dir tree + registry row) — a hub
    /// HISTORY row's delete button. History-only: the path is resolved HOST-side from the
    /// uuid ([`store::list_all_sessions`]), and the delete is refused (a no-op refresh) if the
    /// uuid is currently LIVE or its lock is held, never touching a running session.
    DeleteSession(String),
    /// Re-run cross-daemon discovery + push a FRESH `Hub` envelope. Fired when the
    /// React ResumePalette overlay opens (and may re-fire while it stays open).
    /// Handled in BOTH host states: inline in `host_swapper` (nothing renders there,
    /// so the blocking sweep is fine), and OFF the fold thread in `render::push_loop`
    /// while attached (the sweep must not stall the 16ms loop). This keeps the live
    /// session list current instead of frozen at the one cold build-at-boot.
    RefreshHub,
    /// Best-effort CANCEL of a session switch (the React loader's Cancel button): bail to
    /// the hub. An in-flight attach can't be interrupted (the client-thread is blocked in
    /// `attach_session_headless` and never polls this channel then), so this is queued and
    /// acted on once the current/target attach lands — `push_loop` returns
    /// `HostTransition::ToSwapper` and `host_swapper` pushes a fresh `Hub` (clearing the
    /// loader). In the swapper it is a harmless hub re-push.
    ToSwapper,
    /// Apply a config-GLOBAL mutation directly to `~/.koma/config.json` while PRE-SESSION.
    /// The empty-state/onboarding flow runs in the SWAPPER, which holds NO attached daemon
    /// to forward a `ClientRequest` to (the ipc `live_req` slot is `None`), so the theme +
    /// provider + model setters that onboarding drives arrive here instead. Carries the
    /// SAME [`ClientRequest`] the attached path forwards; `host_swapper` loads the config,
    /// applies the config-global subset (provider/model/mcp/theme), saves, and re-pushes a
    /// fresh `Config` (repainting the theme + clearing `needsOnboarding`). Session-scoped
    /// requests (a `SetModel { scope:"local" }`, `SetSessionMain`) are no-ops here — there
    /// is no session to hold a local override yet.
    ConfigMutate(ClientRequest),
    /// UN-ATTACHED live model-id fetch (the GUI Connector ModelForm's model-id picker while
    /// onboarding / in the swapper, where the ipc `live_req` daemon path is `None`). The
    /// swapper resolves `provider` (uuid) from the GLOBAL config, runs the `GET
    /// {endpoint}/models` OFF-thread (a network call must never block the ctl loop), and
    /// pushes the SAME `ModelList` envelope the attached daemon path produces — ALWAYS a
    /// reply (an EMPTY list on an unknown provider or any fetch error), so the React picker's
    /// spinner can never hang.
    ListModels { provider: String },
    /// UN-ATTACHED twin of [`ListModels`](Self::ListModels) for the ROUTE picker: fetch one
    /// model's live provider-route list (`GET {endpoint}/models/{model_id}/endpoints`)
    /// off-thread and push a `RouteList` envelope (echoing `provider` + `model_id`). EMPTY
    /// routes for a non-OpenRouter provider (the endpoints API is OpenRouter-specific) or any
    /// fetch error — again ALWAYS a reply so the form falls back to "Auto" instead of hanging.
    ListRoutes { provider: String, model_id: String },
    /// Host-side FILE DIFF fetch for the Explore "FILE CHANGED" panel's Monaco diff tab
    /// (`path` is a `fileChanges` record's path). Unlike [`ListModels`]/[`ListRoutes`],
    /// this NEVER prefers the attached daemon — the ipc handler routes it here
    /// unconditionally, because the host process already has direct filesystem + git
    /// access and the daemon would have to do the exact same local work anyway. Serviced
    /// off-thread (git + fs are blocking) in both host states; see [`compute_file_diff`].
    FileDiff { path: String },
    /// Host-side GIT STATUS fetch for the Explore "GIT" panel (branch, ahead/behind,
    /// staged/unstaged file lists). Same reasoning as [`FileDiff`](Self::FileDiff):
    /// NEVER touches the daemon regardless of attach state — the host process already
    /// has direct git access. Serviced off-thread (git is blocking) in both host
    /// states; see [`compute_git_status`]. Carries no session — the receiving loop
    /// supplies its OWN foreground-session id (`current`/`current_owned`), exactly
    /// like `FileDiff` does, since the GUI page never needs to say which session (the
    /// host already knows what's attached).
    GitStatus,
    /// Host-side GIT DIFF fetch for the GIT panel's file-row click (`path` is the
    /// clicked entry's path; `staged` selects index-vs-HEAD when `true`, worktree-vs-
    /// index when `false`) to open a Monaco diff tab. Same reasoning + routing as
    /// [`GitStatus`](Self::GitStatus); see [`compute_git_diff`].
    GitDiff { path: String, staged: bool },
    /// Host-side GIT STAGE mutation for the GIT panel's "+" row button / "Stage All"
    /// header action (`paths` repo-root-relative). Same reasoning as
    /// [`GitStatus`](Self::GitStatus) — NEVER touches the daemon regardless of attach
    /// state. Serviced off-thread (git is blocking); the worker pushes a `GitOp` reply
    /// THEN a follow-up [`compute_git_status`] `GitStatus` push so the panel's lists
    /// refresh from authoritative state after the mutation. See [`git_stage`].
    GitStage { paths: Vec<String> },
    /// Host-side GIT UNSTAGE mutation ("−" row button / "Unstage All"). Same
    /// reasoning + reply pattern as [`GitStage`](Self::GitStage); see [`git_unstage`].
    GitUnstage { paths: Vec<String> },
    /// Host-side GIT DISCARD mutation (destructive — the React side gates this behind
    /// a confirm before ever sending it). Same reasoning + reply pattern as
    /// [`GitStage`](Self::GitStage); see [`git_discard`].
    GitDiscard { paths: Vec<String> },
    /// Host-side GIT COMMIT of whatever is currently staged. Same reasoning + reply
    /// pattern as [`GitStage`](Self::GitStage); see [`git_commit`].
    GitCommit { message: String },
    /// UN-ATTACHED GUI Settings-tab fetch (a [`ClientRequest::GetSettings`] serviced by the
    /// swapper, where the ipc `live_req` daemon path is `None`). There is no foreground
    /// session pre-attach, so the swapper answers from the GLOBAL config: the active
    /// `palette` plus [`crate::model::settings::Settings`] DEFAULTS for the toggles, and an
    /// EMPTY `name`/`workdir`. ALWAYS pushes a `SettingsValues` reply (like the attached
    /// path) so the Settings tab's loading state can never hang.
    GetSettings,
    /// UN-ATTACHED GUI /agents fetch (a [`ClientRequest::ListAgents`] serviced by the
    /// swapper / start-screen host, where the ipc `live_req` daemon path is `None`). There
    /// is no foreground session pre-attach, so the host answers from `load_registry(None)`
    /// (built-in + global agents only — no session overlay) + the GLOBAL config catalogue
    /// (`AppConfig::load`). ALWAYS pushes an `AgentsValues` reply (like [`GetSettings`]) so
    /// the dashboard's loading state can never hang.
    GetAgents,
    /// UN-ATTACHED GUI OAuth-screen fetch (a [`ClientRequest::GetOAuthState`] serviced by the
    /// swapper / start-screen host, where the ipc `live_req` daemon path is `None`). The host
    /// answers from `~/.koma/config.json`'s `oauth_conns` (TOKENLESS wire projection) + the
    /// data-driven provider registry. ALWAYS pushes an `OAuthState` reply (phase `"idle"`,
    /// like [`GetAgents`]) so the OAuth screen never hangs.
    GetOAuthState,
    /// UN-ATTACHED GUI OAuth connection delete (a [`ClientRequest::DeleteOAuthConn`] serviced
    /// by the swapper / start-screen host): remove the connection from `~/.koma/config.json`,
    /// persist, evict its token-refresh cache entry, and re-push a fresh `idle` `OAuthState`.
    /// Reachable pre-session so a connection is removable before any session exists (the login
    /// FLOW itself stays attached-only). The `uuid` is the connection to drop.
    DeleteOAuthConn { uuid: String },
    /// Activity-bar "Usage" panel fetch: compute a LAST-7-DAYS preview straight off the
    /// global `~/.koma/usage.sqlite` ledger. Like [`FileDiff`](Self::FileDiff) this NEVER
    /// touches the daemon in either host state — the ledger is a process-local file the
    /// host already has direct access to — so it is routed here unconditionally by the
    /// ipc handler. Serviced off-thread (sqlite I/O is blocking) in both host states; see
    /// [`compute_usage_preview`]. `session` is `Some(uuid)` for the "session" scope
    /// toggle (filters every query to that session's ledger rows) or `None` for the
    /// default "all" (global) scope — the ipc handler forces this to `None`/"all" itself
    /// when "session" was requested with no session to filter by. `scope` is the literal
    /// `"all"`/`"session"` token. BOTH are carried through unchanged so the reply can
    /// echo them back — the React panel drops a reply whose scope no longer matches the
    /// currently-selected one (a rapid toggle racing an in-flight request), OR whose
    /// echoed session id no longer matches the currently-attached session (the
    /// foreground session switched while a "session"-scope request was in flight, which
    /// would otherwise render the OLD session's numbers under the new attach).
    UsagePreview {
        session: Option<String>,
        scope: String,
    },
}

/// Run the thin attach client, with the daemon-per-session SWAPPER.
///
/// A two-state run-loop ([`ClientState`]): ATTACHED (render a daemon's frames + forward
/// input) or SWAPPER (the detached `/resume` picker). Startup depends on `opts.resume`:
///
/// - **`--resume` / `koma agents`** (`opts.resume`): start in the SWAPPER with NO daemon
///   connection and nothing to return to. The user picks a session (live, history, or a
///   fresh `[+ new session]`) and the client attaches to it; cancelling with nothing to
///   return to exits cleanly. (`opts.session` is ignored on this path — main does not mint
///   one for `--resume`.)
/// - **plain `koma` / `--session X`** (no resume): attach immediately to `opts.session`
///   (the uuid main minted + spawned a daemon for), exactly as before.
///
/// Swap is detach-then-pick: a `/resume` IN-session makes the daemon send `OpenSwapper`,
/// the render loop returns [`render::ClientTransition::OpenSwapper`], and here we
/// [`teardown_connection`] (leaving that daemon cooking) before running the swapper
/// STANDALONE — so its snapshots can't clobber the local hub. A PICK attaches the chosen
/// daemon; a CANCEL reconnects the one we left. `rt`/`handle`, the terminal, and the
/// [`TerminalGuard`] are owned across the whole loop, so a swap reuses the same runtime +
/// alt-screen with no re-enter flash. A failed attach degrades to the swapper (rebuilt
/// from fresh discovery) rather than crashing.
pub fn client_run(opts: crate::cli::Opts) -> Result<()> {
    // The client needs the config dirs only to resolve socket paths; it owns no sessions
    // and writes no config (lock ownership belongs to the daemon — see the module header).
    store::ensure_dirs()?;

    // A small multi-thread runtime drives the two socket tasks of whatever connection is
    // live. The render/swapper loops run on THIS thread (synchronous), like the local TUI.
    // Owned across the WHOLE state machine so every attach/reattach reuses it.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // Terminal setup — identical to the local TUI (`run`). Guard first so a failure
    // anywhere after still restores the terminal. The guard persists ACROSS the loop so a
    // detach-then-swap re-attaches without re-entering the alt-screen.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // `current_session_id` is the session we are (or are becoming) attached to;
    // `prev_session` is what a swapper CANCEL returns to. On `--resume` both start empty
    // (the swapper opens cold); otherwise `current` is the minted/`--session` id.
    let mut current_session_id: Option<String> = None;
    let mut prev_session: Option<String> = None;

    // Seed the initial state. Build-skew handling + daemon-spawn live in `attach_session`,
    // so an `Err` here (no daemon could be started, or the initial connect failed) is
    // surfaced to the caller exactly as before — BUT only on the non-resume path, where an
    // attach happens up front. The `--resume` path can't fail here (no attach yet).
    let mut state = if opts.resume {
        // `--resume` / `koma agents`: swapper first, no connection, nothing to return to.
        ClientState::Swapper(build_local_hub(None))
    } else {
        // Plain `koma` / `--session X`: attach immediately to the minted/given id (REQUIRED
        // here — without it there is no socket to reach).
        let id = opts.session.clone().ok_or_else(|| {
            anyhow::anyhow!("internal: client_run requires a session id (--session <id>) without --resume")
        })?;
        let conn = attach_session(&mut terminal, &handle, &id)?;
        current_session_id = Some(id);
        ClientState::Attached(conn)
    };

    // The loop's terminal outcome (Ok on a clean exit, or a render error captured so the
    // active connection's teardown still runs before it is returned).
    let mut render_result: Result<()> = Ok(());

    loop {
        state = match state {
            // --- ATTACHED: render the daemon's frames + forward input ---
            ClientState::Attached(mut conn) => {
                // Take the handshake's prebuffered frames OUT for this render pass (the
                // loop consumes them); leaves an empty Vec so `conn` stays intact for the
                // teardown / detach below.
                let prebuffered = std::mem::take(&mut conn.prebuffered);

                // Run the render loop with the runtime context entered on THIS thread,
                // SCOPED so the `EnterGuard` drops the instant it returns — BEFORE any
                // `teardown_connection`'s `block_on` (which panics under an entered
                // context). The context is needed only so a snapshot rebuild can mint the
                // inert `AbortHandle` a reconstructed shadow `SubAgent` carries.
                let transition = {
                    let _rt_ctx = handle.enter();
                    render::render_loop(
                        &mut terminal,
                        &conn.frame_rx,
                        &conn.req_tx,
                        prebuffered,
                    )
                };

                match transition {
                    // Leave the client: tear this connection down (flush the Detach) and
                    // break out of the loop. No connection survives, so the post-loop has
                    // nothing more to detach.
                    Ok(render::ClientTransition::Exit) => {
                        teardown_connection(&handle, conn);
                        break;
                    }
                    // `/resume`: DETACH from this daemon (leaving it cooking) and open the
                    // local swapper STANDALONE. Record where to return on cancel, then
                    // build the hub from fresh cross-daemon discovery (flagging the row we
                    // just left as `is_foreground`).
                    Ok(render::ClientTransition::OpenSwapper) => {
                        teardown_connection(&handle, conn);
                        prev_session = current_session_id.take();
                        ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                    }
                    // `/new` (`kill = false`) / `/new kill` (`kill = true`): DETACH from this
                    // daemon and attach a BRAND-NEW one. In the daemon-per-session world a
                    // daemon owns exactly ONE session, so `/new` makes another DAEMON, not a
                    // tab. On `kill` we queue `QuitDaemon` on the request channel BEFORE
                    // teardown so the writer flushes it (the old daemon releases its lock,
                    // drops its session, and unlinks its socket) ahead of the polite `Detach`;
                    // on plain `/new` we only `Detach` and leave the old daemon cooking
                    // (resumable via the swapper). Then mint a fresh uuid and attach its daemon
                    // (spawned on demand by `attach_session`). `prev_session` is deliberately
                    // LEFT as-is — a `/new` is not a swapper cancel, so there is nothing to
                    // "return to". If the attach of the new session fails we DEGRADE to the
                    // swapper (rebuilt from fresh discovery) rather than crash, matching the
                    // failed-Pick degrade below.
                    Ok(render::ClientTransition::NewSession { kill }) => {
                        if kill {
                            // Reap the old daemon: queue QuitDaemon BEFORE teardown so the
                            // writer drains it (then the Detach) before the socket closes.
                            // The client is its daemon's controller, so QuitDaemon is accepted.
                            let _ = conn.req_tx.send(ClientRequest::QuitDaemon);
                        }
                        teardown_connection(&handle, conn);
                        let new_id = uuid::Uuid::new_v4().to_string();
                        match attach_session(&mut terminal, &handle, &new_id) {
                            Ok(conn) => {
                                current_session_id = Some(new_id);
                                ClientState::Attached(conn)
                            }
                            Err(e) => {
                                eprintln!(
                                    "koma: could not start a new session {new_id}: {e:#}"
                                );
                                // Degrade to the swapper (fresh discovery). Don't disturb
                                // `prev_session`; the old daemon (if not killed) is still in
                                // the discovered list.
                                ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                            }
                        }
                    }
                    // A render error ends the loop like an exit, but the error is carried
                    // out and returned AFTER this connection's teardown so the daemon is
                    // never orphaned by an early return.
                    Err(e) => {
                        teardown_connection(&handle, conn);
                        render_result = Err(e);
                        break;
                    }
                }
            }

            // --- SWAPPER: the detached `/resume` picker ---
            ClientState::Swapper(mut hub) => match run_swapper(&mut terminal, &mut hub, prev_session.as_deref())? {
                // Picked a target session: attach to its daemon (spawning if needed). On
                // success it becomes the foreground; on failure DEGRADE to the swapper
                // rebuilt from fresh discovery (the dead/unreachable session drops out)
                // rather than crash — the user can pick again.
                SwapperOutcome::Pick(target) => match attach_session(&mut terminal, &handle, &target) {
                    Ok(conn) => {
                        current_session_id = Some(target);
                        ClientState::Attached(conn)
                    }
                    Err(e) => {
                        eprintln!("koma: could not attach to session {target}: {e:#}");
                        ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                    }
                },
                // Cancelled: reconnect to the previous session if there was one; otherwise
                // (a `--resume` cold start with nothing to return to) exit cleanly. A
                // failed reconnect to a since-died previous daemon also degrades back to
                // the swapper instead of crashing.
                SwapperOutcome::Cancel => match prev_session.take() {
                    Some(prev) => match attach_session(&mut terminal, &handle, &prev) {
                        Ok(conn) => {
                            current_session_id = Some(prev);
                            ClientState::Attached(conn)
                        }
                        Err(e) => {
                            eprintln!("koma: could not reconnect to session {prev}: {e:#}");
                            ClientState::Swapper(build_local_hub(None))
                        }
                    },
                    None => break,
                },
            },
        };
    }

    // Every live connection was already torn down inside the `Attached` arm it exited from
    // (so there is no double-detach and no connection to clean up here). A break straight
    // out of the swapper has no connection at all. Drop the runtime LAST so the active
    // connection's reader task (if any) is cancelled after exit.
    drop(rt);

    render_result
}

/// Run the `/select` transcript dump on the CLIENT's terminal.
///
/// Re-exported so `runtime/mod.rs` can optionally reference it; the actual
/// implementation lives in `render::client_select_dump` (called from `render_loop`).
pub(super) fn client_select_dump(
    terminal: &mut ratatui::Terminal<CrosstermBackend<std::io::Stdout>>,
    shadow: &crate::app::state::AppState,
) -> Result<()> {
    render::client_select_dump(terminal, shadow)
}

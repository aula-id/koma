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
//! | `git_remote`| host-side git remote sync (fetch/pull/push) + key assignment    |
//! | `git_graph` | host-side commit-graph + commit-detail + commit-diff computation |
//! | `git_activity` | host-side per-commit activity (author/date/lines-changed) computation (GK5a) |
//! | `git_branch`| host-side branch list + checkout + create-branch computation     |
//! | `git_destructive` | host-side cherry-pick/revert/reset/merge/rebase/abort/continue (G5b) |
//! | `git_stash` | host-side stash push/pop/list (GK4a)                              |
//! | `keys`      | host-side SSH key vault (GUI Settings "SSH Keys" section)       |
//! | `git_host`  | off-thread GIT/key `HostCtl` bodies shared by `host` + `push_loop` |
//! | `git_host_mut` | `git_host`'s G5b destructive spawn flavors, split out for size |
//! | `store_host` | off-thread extension-STORE browse/detail/installed-list `HostCtl` bodies shared by `host` + `push_loop` |
//! | `host`      | GUI host-relay layer (`run_host_relay`, the swapper/attached FSM) |
//! | `host_catalogue` | un-attached model/route/agents/oauth catalogue builders for `host` |
//! | `host_config` | Pre-session (swapper) config-apply helpers for `host`           |
//! | `push_proto`| GUI push-envelope DTOs (`PushEnvelope` + the one-shot non-git `push_*` fns) |
//! | `push_proto_git` | GIT/SSH-key-vault one-shot `push_*` fns (split out of `push_proto`) |
//! | `push_rows` | The `Push*` row/DTO structs `PushEnvelope`'s variants carry       |
//! | `project`   | GUI snapshot serialization (`serialize_and_push`, `push_hub`, `warm_status_label`) |
//! | `project_config` | GUI config projection (`ConfigProjection`, `push_config`)          |
//! | `push_intercept` | one-shot `DaemonEvent` -> `PushEnvelope` re-push checks for `push_loop` |
//! | `push_loop` | The headless attached fold loop (`push_loop`, `PushState`, `HostTransition`) |
//! | `git_drain` | `push_loop`'s GIT/SSH-key-vault off-thread reply drain (`drain_git_replies`) |
//! | `file_ops`  | host-side Coding panel file tree/read/save/create/rename/delete  |

#![allow(unused_imports)]
#![allow(dead_code)]

mod bridge;
mod connect;
mod diff;
mod file_ops;
mod git;
mod git_activity;
mod git_branch;
mod git_destructive;
mod git_drain;
mod git_graph;
mod git_host;
mod git_host_mut;
pub(super) mod git_remote;
mod git_repos;
mod git_stash;
mod host;
mod host_catalogue;
mod host_config;
#[cfg(feature = "linker")]
mod import_graph;
mod input;
mod keys;
mod project;
mod project_config;
mod push_intercept;
mod push_loop;
mod push_proto;
mod push_proto_git;
mod push_rows;
pub(crate) mod remote;
mod remote_ctl;
mod render;
mod shadow;
mod store_host;
mod swapper;
mod swapper_keys;

#[cfg(test)]
#[path = "analytics_test.rs"]
mod analytics_test;

use std::io::{stdout, Stdout};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{Mode, SessionHub};
use crate::ipc::proto::ClientRequest;
use crate::model::store;

use bridge::WRITER_FLUSH_TIMEOUT;
use connect::{connect_attach_and_handshake, Connection};
use swapper::{build_local_hub, build_remote_hub, run_swapper, DiscoverySource, SwapperOutcome};

use crate::app::runtime::terminal::TerminalGuard;

// Re-exported so `gui::mod`'s existing `super::client::run_host_relay` call site
// keeps resolving unchanged after `run_host_relay` moved into the sibling `host`
// module (that call site lives outside `client`, hence the `pub(in ...)` reach).
pub(in crate::app::runtime) use host::run_host_relay;
// Re-export ClientTransition so `remote::client` can reference it without making
// the entire `render` module public.
pub(crate) use render::ClientTransition;

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
/// The restart (`manage::restart_daemon`) is blocking (~1s), so it runs on a background
/// thread while THIS (main) thread — which owns the terminal — draws the spinner each
/// frame until it completes, then propagates its result. Kept silent (quiet=true).
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
        Ok(res) => {
            res.map_err(|e| anyhow::anyhow!("failed to restart the stale koma daemon: {e:#}"))
        }
        Err(_) => Err(anyhow::anyhow!(
            "reopening thread panicked during daemon restart"
        )),
    }
}

/// Attach to a session-daemon, spawning it if needed, and run the build-skew handshake.
///
/// The single attach primitive used everywhere the client connects: the initial
/// non-resume attach, a swapper PICK, and a swapper CANCEL-reconnect. Ensures the
/// session's daemon is running ([`super::manage::ensure_daemon_running`]), connects +
/// handshakes ([`connect_attach_and_handshake`]), then on a CONFIRMED build-skew
/// mismatch (the daemon outlives a rebuild — task #142, this caused a phantom
/// `/agents` bug) restarts that ONE stale daemon (AT MOST ONCE, via the same
/// machinery `koma daemon restart` uses) and reconnects — comparing its own fresh
/// [`store::build_fingerprint`] against the daemon's reported `Hello`. A daemon that
/// sends no `Hello` (slow / pre-handshake) is never restarted on that absence alone.
fn attach_session(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    handle: &tokio::runtime::Handle,
    session_id: &str,
) -> Result<Connection> {
    // Make sure a daemon owns this session before we connect. No-op when it is already
    // live (the bind-as-oracle probe inside short-circuits); spawns + waits otherwise.
    super::manage::ensure_daemon_running(session_id, false, None).map_err(|e| {
        anyhow::anyhow!("could not start the koma daemon for session {session_id}: {e:#}")
    })?;

    let sock_path = store::daemon_sock_path(session_id)?;
    let my_fingerprint = store::build_fingerprint();

    let mut conn = connect_attach_and_handshake(handle, &sock_path, session_id)?;
    let mut already_restarted = false;
    while conn
        .daemon_version
        .as_deref()
        .is_some_and(|v| v != my_fingerprint)
    {
        if already_restarted {
            crate::model::store::append_global_error_log(
                "client",
                "daemon still reports a different build after a restart; continuing against it",
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

        conn = connect_attach_and_handshake(handle, &sock_path, session_id)?;
    }
    Ok(conn)
}

/// Tear a live [`Connection`] down cleanly: queue a polite `Detach`, then drop the
/// request sender and JOIN the writer so the final frame(s) flush before the runtime
/// is touched. Run on EVERY exit from [`ClientState::Attached`] (plain exit, render
/// error, or detach-then-swap) so the source daemon is never left orphaned; a `/quit`
/// overlay `Detach` already queued makes this a harmless no-op. Dropping `req_tx`
/// closes the outbound channel — the writer drains every queued request then returns;
/// we JOIN it (bounded by [`WRITER_FLUSH_TIMEOUT`]) so a wedged socket can't hang exit.
/// MUST NOT be called while a tokio runtime context is entered (it `block_on`s).
fn teardown_connection(handle: &tokio::runtime::Handle, conn: Connection) {
    let Connection {
        frame_rx: _,
        req_tx,
        writer_handle,
        prebuffered: _,
        daemon_version: _,
        transport: _,
    } = conn;

    let _ = req_tx.send(ClientRequest::Detach);
    drop(req_tx);
    let _ =
        handle.block_on(async { tokio::time::timeout(WRITER_FLUSH_TIMEOUT, writer_handle).await });
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
#[derive(Clone)]
pub(super) enum HostCtl {
    /// The webview page booted / reloaded: re-push the full authoritative state.
    Ready,
    /// Attach to this existing session UUID (a hub `SelectSession` pick).
    Select(String),
    /// Mint a fresh session UUID + attach (the hub `[+ new session]` row, or the attached
    /// chat view's "new session"). `workdir` is the folder the GUI's native picker chose;
    /// `None` falls back to the host's cwd. `kill` (attached state only) reaps the CURRENT
    /// session's daemon as part of the switch — mirroring the TUI `/new kill`; `kill: false`
    /// leaves the old daemon cooking (resumable).
    New {
        workdir: Option<std::path::PathBuf>,
        kill: bool,
    },
    /// Kill the session-daemon `id` (a hub row's KILL button, or the attached chat view's
    /// "kill this session"). Escalating (graceful `QuitDaemon` → SIGTERM → SIGKILL), run OFF
    /// the control loop. Killing the CURRENTLY-ATTACHED session additionally queues a
    /// `QuitDaemon` on the live conn + hands back to the swapper; a background kill just
    /// refreshes the hub once the daemon is confirmed dead.
    KillSession(String),
    /// Physically DELETE the history session `id` (on-disk dir tree + registry row) — a hub
    /// HISTORY row's delete button. The delete is refused (a no-op refresh) if the uuid is
    /// currently LIVE or its lock is held, never touching a running session.
    DeleteSession(String),
    /// Re-run cross-daemon discovery + push a FRESH `Hub` envelope. Fired when the React
    /// ResumePalette overlay opens (and may re-fire while it stays open). Handled inline in
    /// `host_swapper`, and OFF the fold thread in `render::push_loop` while attached (the
    /// blocking sweep must not stall the 16ms loop).
    RefreshHub,
    /// Best-effort CANCEL of a session switch (the React loader's Cancel button): bail to
    /// the hub. An in-flight attach can't be interrupted, so this is queued and acted on
    /// once the current/target attach lands. In the swapper it is a harmless hub re-push.
    ToSwapper,
    /// Apply a config-GLOBAL mutation directly to `~/.koma/config.json` while PRE-SESSION.
    /// The empty-state/onboarding flow runs in the SWAPPER, which holds NO attached daemon
    /// to forward a `ClientRequest` to (the ipc `live_req` slot is `None`), so the theme +
    /// provider + model setters onboarding drives arrive here instead. Carries the SAME
    /// [`ClientRequest`] the attached path forwards; `host_swapper` applies the
    /// config-global subset (provider/model/mcp/theme), saves, and re-pushes a fresh
    /// `Config`. Session-scoped requests are no-ops here — there is no session yet.
    ConfigMutate(ClientRequest),
    /// UN-ATTACHED live model-id fetch (the GUI Connector ModelForm's model-id picker while
    /// onboarding / in the swapper). The swapper resolves `provider` (uuid) from the GLOBAL
    /// config, runs the `GET {endpoint}/models` OFF-thread, and pushes the SAME `ModelList`
    /// envelope the attached daemon path produces — ALWAYS a reply so the picker's spinner
    /// can never hang.
    ListModels { provider: String },
    /// UN-ATTACHED twin of [`ListModels`](Self::ListModels) for the ROUTE picker: fetch one
    /// model's live provider-route list off-thread and push a `RouteList` envelope (echoing
    /// `provider` + `model_id`). EMPTY routes for a non-OpenRouter provider or any fetch
    /// error — again ALWAYS a reply so the form falls back to "Auto" instead of hanging.
    ListRoutes { provider: String, model_id: String },
    /// Host-side FILE DIFF fetch for the Explore "FILE CHANGED" panel's Monaco diff tab
    /// (`path` is a `fileChanges` record's path). Unlike [`ListModels`]/[`ListRoutes`], this
    /// NEVER prefers the attached daemon — the host already has direct filesystem + git
    /// access. Serviced off-thread in both host states; see [`compute_file_diff`].
    FileDiff { path: String },
    /// Host-side GIT STATUS fetch for the Explore "GIT" panel (branch, ahead/behind,
    /// staged/unstaged file lists). NEVER touches the daemon regardless of attach
    /// state — the host already has direct git access. Serviced off-thread (git is
    /// blocking); see [`compute_git_status`]. Carries no session — the receiving
    /// loop supplies its OWN foreground-session id (`current`/`current_owned`).
    GitStatus,
    /// Host-side GIT DIFF fetch for the GIT panel's file-row click (`path` is the
    /// clicked entry's path; `staged` selects index-vs-HEAD when `true`, worktree-vs-
    /// index when `false`). Same reasoning + routing as [`GitStatus`](Self::GitStatus);
    /// see [`compute_git_diff`].
    GitDiff { path: String, staged: bool },
    /// Host-side GIT STAGE mutation for the GIT panel's "+" row button / "Stage All"
    /// (`paths` repo-root-relative). Same reasoning as [`GitStatus`](Self::GitStatus);
    /// the worker pushes a `GitOp` reply THEN a follow-up `GitStatus` push so the
    /// panel's lists refresh after the mutation. See [`git_stage`].
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
    /// Host-side COMMIT GRAPH fetch for a GitKraken-style commit-graph panel view
    /// (`limit` rows starting `skip` back, across every ref). Same reasoning as
    /// [`GitStatus`](Self::GitStatus); see [`git_graph::compute_git_graph`].
    GitGraph { limit: u32, skip: u32 },
    /// Host-side COMMIT DETAIL fetch for a commit-graph row click (full metadata +
    /// changed-file list). Same reasoning + routing as
    /// [`GitGraph`](Self::GitGraph); see [`git_graph::compute_commit_detail`].
    GitCommitDetail { sha: String },
    /// Host-side COMMIT DIFF fetch for a commit-detail file-row click (`path` at
    /// commit `sha` vs its first parent). Same reasoning + routing as
    /// [`GitGraph`](Self::GitGraph); see [`git_graph::compute_commit_diff`].
    GitCommitDiff { sha: String, path: String },
    /// UN-ATTACHED GUI Settings-tab fetch (a [`ClientRequest::GetSettings`] serviced by the
    /// swapper). There is no foreground session pre-attach, so the swapper answers from the
    /// GLOBAL config: the active `palette` plus [`crate::model::settings::Settings`]
    /// DEFAULTS, and an EMPTY `name`/`workdir`. ALWAYS pushes a `SettingsValues` reply so
    /// the Settings tab's loading state can never hang.
    GetSettings,
    /// UN-ATTACHED GUI /agents fetch (a [`ClientRequest::ListAgents`] serviced by the
    /// swapper / start-screen host). The host answers from `load_registry(None)` (built-in +
    /// global agents only) + the GLOBAL config catalogue. ALWAYS pushes an `AgentsValues`
    /// reply (like [`GetSettings`]) so the dashboard's loading state can never hang.
    GetAgents,
    /// UN-ATTACHED GUI OAuth-screen fetch (a [`ClientRequest::GetOAuthState`] serviced by the
    /// swapper / start-screen host). The host answers from `~/.koma/config.json`'s
    /// `oauth_conns` (TOKENLESS wire projection) + the provider registry. ALWAYS pushes an
    /// `OAuthState` reply (phase `"idle"`) so the OAuth screen never hangs.
    GetOAuthState,
    /// UN-ATTACHED GUI OAuth connection delete: remove the connection from
    /// `~/.koma/config.json`, persist, evict its token-refresh cache entry, and re-push a
    /// fresh `idle` `OAuthState`. Reachable pre-session (the login FLOW itself stays
    /// attached-only). The `uuid` is the connection to drop.
    DeleteOAuthConn { uuid: String },
    /// UN-ATTACHED GUI OAuth login START (the home-screen / Settings-pre-session "Sign
    /// in to koma.run" etc. button): there is no attached daemon to run the flow on, so
    /// the swapper runs it HOST-side — the exact same `service::oauth::flow::run_flow`
    /// dispatcher the attached `Action::OAuthStart` spawns, just with the host's push
    /// sink standing in for the daemon's per-client `OAuthState` reply. `provider` is the
    /// wire string (`"codex"`/`"kilocode"`/`"xai"`/`"claudeai"`/`"komarun"`), resolved via
    /// [`crate::model::app_config::OAuthProvider::from_wire_id`]. Progress streams back as
    /// `waiting_url`/`waiting_code` `OAuthState` pushes, ending in `success` (after the
    /// connection is appended to the GLOBAL config + persisted) or `failed`. Superseding a
    /// flow already in flight aborts it first, mirroring `handle_oauth_start`'s supersede.
    StartOAuth { provider: String },
    /// UN-ATTACHED GUI OAuth login CANCEL: abort whatever host-local flow
    /// [`StartOAuth`](Self::StartOAuth) started (a no-op if none is in flight) and
    /// re-push a fresh `idle` `OAuthState` so the Cancel button always lands somewhere
    /// rather than leaving the wait screen stranded.
    CancelOAuth,
    /// Activity-bar "Usage" panel fetch: compute a LAST-7-DAYS preview straight off the
    /// global `~/.koma/usage.sqlite` ledger. Like [`FileDiff`](Self::FileDiff) this NEVER
    /// touches the daemon in either host state. Serviced off-thread; see
    /// [`compute_usage_preview`]. `session` is `Some(uuid)` for the "session" scope toggle
    /// or `None` for "all"; `scope` is the literal `"all"`/`"session"` token. BOTH are
    /// echoed back so the React panel can drop a stale reply (a rapid scope toggle, or the
    /// foreground session switching mid-flight) instead of rendering the wrong numbers.
    UsagePreview {
        session: Option<String>,
        scope: String,
    },
    /// Analytics tab: compute a host-side usage dashboard projection (KPI totals,
    /// time series, per-model table, main-vs-sub role split) straight off the
    /// global `~/.koma/usage.sqlite` ledger. Like [`UsagePreview`](Self::UsagePreview)
    /// this NEVER touches the daemon in either host state. Serviced off-thread;
    /// see [`compute_analytics`]. Correlation inputs (`req_seq`/`scope`/`session`/
    /// `range`/`metric`) are all echoed back so the React tab can drop a stale
    /// reply across rapid filter/session changes.
    Analytics {
        req_seq: u64,
        session: Option<String>,
        scope: String,
        range: String,
        metric: String,
    },
    /// Host-side SSH KEY VAULT list fetch for the Settings "SSH Keys" section
    /// (`<~/.koma>/keys/`). Same reasoning as [`GitStatus`](Self::GitStatus) — a
    /// GUI-only, manual, user-owned key vault, entirely separate from the model's
    /// own git credential machinery (`git_cred.rs`/`git_operator.rs`). See
    /// [`keys::list_keys`].
    KeyList,
    /// Host-side SSH KEY GENERATE mutation (a fresh passphrase-less ed25519
    /// keypair). Same reasoning + reply pattern as [`GitStage`](Self::GitStage) —
    /// the worker pushes a `KeyOp` reply THEN a follow-up `KeyList` push. See
    /// [`keys::generate_key`].
    KeyGenerate { name: String, comment: String },
    /// Host-side SSH KEY IMPORT mutation (an existing pasted private key). Same
    /// reasoning + reply pattern as [`KeyGenerate`](Self::KeyGenerate); see
    /// [`keys::import_key`].
    KeyImport { name: String, private_key: String },
    /// Host-side SSH KEY REVEAL fetch ("Copy public key" / "Reveal private key").
    /// Same reasoning as [`GitDiff`](Self::GitDiff) — ALWAYS a one-shot `KeyReveal`
    /// reply, never touches the daemon. See [`keys::reveal_key`].
    KeyReveal { name: String, private: bool },
    /// Host-side SSH KEY DELETE mutation. Same reasoning + reply pattern as
    /// [`KeyGenerate`](Self::KeyGenerate); see [`keys::delete_key`].
    KeyDelete { name: String },
    /// Source Control panel's key-picker changed: assign (`Some(name)`) or clear
    /// (`None`, "Default (system ssh)") the foreground session's repo's SSH key for
    /// remote ops. Same reasoning as [`GitStatus`](Self::GitStatus); see
    /// [`git_remote::set_current_key`]. Carries no reply of its own — the worker
    /// pushes a follow-up [`GitStatus`](Self::GitStatus) reflecting the new
    /// assignment (`GitStatusResult.key_name`).
    SetGitKey { name: Option<String> },
    /// Source Control panel's Fetch button: `git fetch --prune`, using the repo's
    /// assigned key's `GIT_SSH_COMMAND` override if one is set. Same reasoning +
    /// reply pattern as [`GitStage`](Self::GitStage); see [`git_remote::git_fetch`].
    GitFetch,
    /// Source Control panel's Pull button: `git pull --ff-only` (fails loudly on
    /// divergence rather than merging/leaving a half-merged tree). Same reasoning +
    /// reply pattern as [`GitFetch`](Self::GitFetch); see [`git_remote::git_pull`].
    GitPull,
    /// Source Control panel's Push button. Same reasoning + reply pattern as
    /// [`GitFetch`](Self::GitFetch); see [`git_remote::git_push`].
    GitPush {
        mode: Option<git_remote::GitPushMode>,
        root: Option<String>,
    },
    /// Branch-switcher popover (footer/GitPanel) or graph context menu opened:
    /// fetch every local + remote-tracking branch. Host-local, never the daemon,
    /// like [`GitStatus`](Self::GitStatus); see [`git_branch::git_branch_list`].
    GitBranchList { request_id: Option<u64> },
    /// Source Control multi-repo picker opened: discover every git repo across the
    /// session's workdirs. Host-local, never the daemon, like
    /// [`GitBranchList`](Self::GitBranchList); see [`git_repos::discover_repos`].
    /// Reply lands as a `RepoList` push.
    GitRepos,
    /// Source Control repo picker changed: set the session's ACTIVE repo to `root`
    /// (an absolute toplevel path from a prior `RepoList`). Host-local, never the
    /// daemon, like [`SetGitKey`](Self::SetGitKey) — no reply of its own; the worker
    /// pushes a follow-up [`GitStatus`](Self::GitStatus) for the newly-active repo.
    SetActiveRepo { root: String },
    /// Branch-switcher pick / graph "Checkout" (SAFE only, never `--force`): switch
    /// (or detach onto) `ref_name` (a branch or a sha). Same reply pattern as
    /// [`GitStage`](Self::GitStage) — React also fires a client-local
    /// `refreshGraph()` once it lands. See [`git_branch::git_checkout`].
    GitCheckout {
        ref_name: String,
        root: Option<String>,
    },
    /// Branch-switcher "+ Create new branch" / graph "Create branch here…" (SAFE
    /// only). `start` is the commit-ish to branch from (`None` = HEAD); `checkout`
    /// switches to it immediately. Same reply pattern as
    /// [`GitCheckout`](Self::GitCheckout). See [`git_branch::git_create_branch`].
    GitCreateBranch {
        name: String,
        start: Option<String>,
        checkout: bool,
        root: Option<String>,
    },
    /// Commit-graph row context menu "Cherry-pick" (G5b). May leave the tree
    /// conflicted — the follow-up `GitStatus` reports that via `inProgress`/
    /// `conflicted`, not this reply's `error` alone. See
    /// [`git_destructive::git_cherry_pick`].
    GitCherryPick { sha: String },
    /// Commit-graph row context menu "Revert" (G5b). Same conflict reasoning as
    /// [`GitCherryPick`](Self::GitCherryPick). See [`git_destructive::git_revert`].
    GitRevert { sha: String },
    /// Commit-graph row context menu "Reset branch to here" (G5b). `mode` is
    /// `"soft"`/`"mixed"`/`"hard"` — `hard` is destructive; the React confirm is the
    /// gate, not this handler. See [`git_destructive::git_reset`].
    GitReset { sha: String, mode: String },
    /// Branch-switcher / graph context menu "Merge into current branch" (G5b). May
    /// conflict, same reasoning as [`GitCherryPick`](Self::GitCherryPick). See
    /// [`git_destructive::git_merge`].
    GitMerge { ref_name: String },
    /// Rebase onto `upstream` (G5b/G6). `branch: Some(name)` is the GitKraken-style
    /// drag-to-rebase (checks out + rebases `branch`, not the current branch);
    /// `None` rebases the current branch. May conflict. See
    /// [`git_destructive::git_rebase`].
    GitRebase {
        upstream: String,
        branch: Option<String>,
    },
    /// The conflict banner's Abort button (G5b). `kind` is `"merge"`/`"rebase"`/
    /// `"cherry-pick"`/`"revert"`. See [`git_destructive::git_op_abort`].
    GitOpAbort { kind: String },
    /// The conflict banner's Continue button (G5b) — runs with `GIT_EDITOR=true` so
    /// a `--continue` never hangs on an editor. See
    /// [`git_destructive::git_op_continue`].
    GitOpContinue { kind: String },
    /// Source Control toolbar Stash button (GK4a): `git stash push`. Same reply
    /// pattern as [`GitStage`](Self::GitStage). See [`git_stash::git_stash`].
    GitStash,
    /// Source Control toolbar Pop button (GK4a): `git stash pop`. May conflict,
    /// same reasoning as [`GitCherryPick`](Self::GitCherryPick). See
    /// [`git_stash::git_stash_pop`].
    GitStashPop,
    /// Stash count/indicator fetch (GK4a): `git stash list`. Host-local, never
    /// the daemon, like [`GitStatus`](Self::GitStatus). See [`git_stash::git_stash_list`].
    GitStashList,
    /// Per-commit ACTIVITY fetch for the bubble/activity chart (GK5a): author/date/
    /// lines-changed for `limit` commits on `HEAD`, optionally scoped to `path`.
    /// Host-local, never the daemon, like [`GitGraph`](Self::GitGraph); see
    /// [`git_activity::compute_git_activity`].
    GitActivity { path: Option<String>, limit: u32 },
    /// Extension-STORE browse (Store tab search/filter, or a mount-time fetch on the
    /// home screen): fetch the koma.run catalogue. NEVER touches the daemon regardless
    /// of attach state — the host does the PUBLIC (no-auth) network fetch itself, same
    /// reasoning as [`GitStatus`](Self::GitStatus)/[`FileDiff`](Self::FileDiff) (this is
    /// a stateless read, unlike install/uninstall, which mutate live daemon runtime
    /// state and stay daemon-forwarded — see [`ExtNoSession`](Self::ExtNoSession)). See
    /// `store_host::fetch_catalogue`.
    StoreBrowse {
        query: Option<String>,
        category: Option<String>,
    },
    /// Extension-STORE detail (a catalogue card click): fetch one extension's full
    /// detail. Same host-local reasoning as [`StoreBrowse`](Self::StoreBrowse); see
    /// `store_host::fetch_detail`.
    StoreDetail { id: String },
    /// Extension-STORE "Installed" section fetch: read the local
    /// `~/.koma/config.json` registry. Same host-local reasoning as
    /// [`StoreBrowse`](Self::StoreBrowse) (a config read, not a daemon call); see
    /// `store_host::installed_extensions`.
    ListInstalledExtensions,
    /// Fetch full detail of one locally-installed extension: registry fields +
    /// on-disk manifest contributions (tools/models/panels/sub-agents). Same
    /// host-local reasoning as [`ListInstalledExtensions`] — see
    /// `store_host::get_installed_detail`.
    GetInstalledExtensionDetail { id: String },
    /// PRE-SESSION install: `GuiReq::InstallExtension` arrived with NO attached daemon
    /// (the ipc `live_req` slot is `None` — the home screen / swapper). Runs the SAME
    /// KomaRun sign-in check + download + verify/unpack pipeline the daemon's
    /// `requests_ext::install_extension`/`finish_install` use (the bearer lives in the
    /// GLOBAL `AppConfig`, not anything session-scoped), but SKIPS the session-scoped
    /// tail — MCP tool registration, ext-daemon auto-start, workspace-root injection —
    /// since there is no live `ext_manager`/`mcp_manager`/foreground session pre-session.
    /// That tail self-heals: `lifecycle::build_startup` re-runs `ensure_started` +
    /// `register_contributions` for every enabled daemon-kind extension on EVERY daemon
    /// boot, and re-derives the workspace-root injection from the CURRENT enabled set on
    /// every boot too; a not-yet-started daemon-kind extension also lazily auto-starts on
    /// its first opened panel (see `requests_ext::panel_start_decision`). See
    /// `store_host::spawn_install`.
    InstallExtension { id: String, version: Option<String> },
    /// PRE-SESSION uninstall — same host-local reasoning as [`InstallExtension`]:
    /// purges the on-disk package + registry entry. No live `ext_manager`/`mcp_manager`
    /// to purge contributions from or stop a running child — nothing is registered
    /// pre-session in the first place — so there is nothing to undo. See
    /// `store_host::spawn_uninstall`.
    UninstallExtension { id: String },
    /// Coding panel: list a directory's immediate children.
    FileTree {
        root: String,
        path: String,
        request_id: String,
    },
    /// Coding panel: read a text file.
    FileRead {
        root: String,
        path: String,
        request_id: String,
    },
    /// Coding panel: save a text file with stale-fingerprint protection.
    FileSave {
        root: String,
        path: String,
        content: String,
        expected_fingerprint: String,
        request_id: String,
    },
    /// Coding panel: create a new file or directory.
    FileCreate {
        root: String,
        path: String,
        kind: String,
        request_id: String,
    },
    /// Coding panel: rename within the same workspace root.
    FileRename {
        root: String,
        old_path: String,
        new_path: String,
        request_id: String,
    },
    /// Coding panel: delete a file or directory.
    FileDelete {
        root: String,
        path: String,
        request_id: String,
    },
    /// Import-graph visualization: call the linker daemon's
    /// `Visualization` query and push the result back as an `ImportGraph` envelope.
    #[cfg(feature = "linker")]
    ImportGraph {
        path: Option<String>,
        depth: u32,
        direction: crate::ipc::linker_proto::GraphDirection,
        filter_roots: Option<Vec<String>>,
        filter_languages: Option<Vec<String>>,
        /// Session id for wire correlation — the result carries this so the
        /// frontend can reject stale replies after a session switch.
        session_id: Option<String>,
        /// Request id for correlation — the result echoes this so the GUI can
        /// match replies to originating requests and reject stale ones.
        request_id: Option<String>,
    },
    /// Impact analysis for a file (off-thread linker IPC).
    /// `configured_roots` resolved by host handler from session workdirs.
    #[cfg(feature = "linker")]
    ImportGraphImpact {
        path: String,
        depth: u32,
        request_id: String,
        /// Session id for wire correlation.
        session_id: Option<String>,
    },
    /// Manual reindex: reconcile/register the foreground session's current
    /// workdirs with the linker daemon, issue Rescan, poll until the scan
    /// completes, then refresh the scoped visualization. Handled entirely
    /// off-thread; `request_id` is echoed back so the GUI can correlate.
    #[cfg(feature = "linker")]
    ImportGraphReindex { request_id: Option<String> },

    // ─── Remote host management (host-local CRUD, fast file I/O) ──────────────
    /// Request/list/confirm/cancel a remote working directory. These are serviced
    /// over the retained SSH transport; they never inspect the local filesystem.
    RequestRemotePath,
    ListRemotePath { path: String },
    ConfirmRemotePath { path: String },
    CancelRemotePath,
    /// Fetch the saved remote hosts list and push a RemoteHosts envelope.
    GetRemoteHosts,
    /// Add a new remote host and push the updated list.
    AddRemoteHost {
        name: String,
        user: String,
        host: String,
        port: u16,
        key_path: Option<String>,
    },
    /// Edit an existing remote host by id and push the updated list.
    EditRemoteHost {
        id: String,
        name: String,
        user: String,
        host: String,
        port: u16,
        key_path: Option<String>,
    },
    /// Delete a remote host by id and push the updated list.
    DeleteRemoteHost { id: String },

    // ─── Remote host connect/disconnect ──────────────────────────────────────
    /// Connect to a remote host via SSH (off-thread, blocking).
    ConnectRemote { host_id: String },
    /// Disconnect from the current remote host.
    DisconnectRemote,
    /// Submit a password for in-progress remote host authentication.
    SubmitRemotePassword { password: String },
    /// Cancel an in-progress remote connect attempt.
    CancelRemoteConnect,
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
    let mut _guard = TerminalGuard::enter()?;
    // Enable mouse capture so scroll events arrive as Event::Mouse. Auto resolves
    // to ON (the default). If the session's mouse_capture is Off, the first
    // Snapshot in render_loop will re-apply it via a one-shot sync.
    crate::app::runtime::actions::apply_mouse_capture(crate::model::settings::MouseCapture::Auto);
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // `current_session_id` is the session we are (or are becoming) attached to;
    // `prev_session` is what a swapper CANCEL returns to. On `--resume` both start empty
    // (the swapper opens cold); otherwise `current` is the minted/`--session` id.
    let mut current_session_id: Option<String> = None;
    let mut prev_session: Option<String> = None;
    // When the user exits a remote session via `/resume`, we store the target+password
    // so the swapper can use a remote discovery source and picks can re-SSH.
    let mut remote_resume: Option<(
        crate::remote::RemoteTarget,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = None;

    // Seed the initial state. Build-skew handling + daemon-spawn live in `attach_session`,
    // so an `Err` here (no daemon could be started, or the initial connect failed) is
    // surfaced to the caller exactly as before — BUT only on the non-resume path, where an
    // attach happens up front. The `--resume` path can't fail here (no attach yet).
    let mut state = if opts.remote_entry {
        let target = opts
            .remote_target
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("remote entry requires a target"))?;
        drop(terminal);
        drop(_guard);
        let result = remote_attach(target, opts.remote_key.as_deref(), None, false, None)?;
        _guard = TerminalGuard::enter()?;
        crate::app::runtime::actions::apply_mouse_capture(
            crate::model::settings::MouseCapture::Auto,
        );
        let backend = CrosstermBackend::new(stdout());
        terminal = Terminal::new(backend)?;
        terminal.clear()?;
        match result {
            crate::remote::client::RemoteExit::Resume { context } => {
                let mut target = context.target;
                if target.key.is_none() {
                    target.key = context.key_hint;
                }
                let password = context.password;
                let session_id = context.session_id;
                remote_resume = Some((
                    target.clone(),
                    password.clone(),
                    session_id.clone(),
                    context.cwd,
                ));
                ClientState::Swapper(build_remote_hub(
                    &target,
                    password.as_deref(),
                    session_id.as_deref(),
                ))
            }
            crate::remote::client::RemoteExit::Exit => return Ok(()),
            crate::remote::client::RemoteExit::NewSession { .. } => {
                return Err(anyhow::anyhow!("remote session handoff escaped remote loop"));
            }
        }
    } else if opts.resume {
        // `--resume` / `koma agents`: swapper first, no connection, nothing to return to.
        ClientState::Swapper(build_local_hub(None))
    } else {
        // Plain `koma` / `--session X`: attach immediately to the minted/given id (REQUIRED
        // here — without it there is no socket to reach).
        let id = opts.session.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "internal: client_run requires a session id (--session <id>) without --resume"
            )
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
                    render::render_loop(&mut terminal, &conn.frame_rx, &conn.req_tx, prebuffered)
                };

                match transition {
                    // Leave the client: tear this connection down (flush the Detach) and
                    // break out of the loop. No connection survives, so the post-loop has
                    // nothing more to detach.
                    Ok(render::ClientTransition::Exit { kill }) => {
                        teardown_connection(&handle, conn);
                        if kill {
                            if let Some(id) = current_session_id.as_deref() {
                                // Block until the daemon is confirmed dead so a reopened
                                // session never reattaches to the dying process.
                                let _ = crate::app::runtime::manage::kill_session_daemon(id);
                            }
                        }
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
                        if kill {
                            // Wait for the old daemon to die so the new attach never races it.
                            if let Some(id) = current_session_id.as_deref() {
                                let _ = crate::app::runtime::manage::kill_session_daemon(id);
                            }
                        }
                        let new_id = uuid::Uuid::new_v4().to_string();
                        match attach_session(&mut terminal, &handle, &new_id) {
                            Ok(conn) => {
                                current_session_id = Some(new_id);
                                ClientState::Attached(conn)
                            }
                            Err(e) => {
                                crate::model::store::append_global_error_log(
                                    "client",
                                    &format!("could not start a new session {new_id}: {e:#}"),
                                );
                                // Degrade to the swapper (fresh discovery). Don't disturb
                                // `prev_session`; the old daemon (if not killed) is still in
                                // the discovered list.
                                ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                            }
                        }
                    }
                    // `/remote`: DETACH from this daemon (leaving it cooking) and connect to
                    // a remote host via SSH. The remote client owns its own full terminal
                    // lifecycle (enter alt-screen, render, exit alt-screen), so we drop OUR
                    // terminal guard before the call and re-enter after. On return, check
                    // whether the user opened the swapper inside the remote session — if so,
                    // build a remote hub so the local swapper shows remote sessions.
                    Ok(render::ClientTransition::ConnectRemote {
                        target,
                        key,
                        new_session,
                        session_id,
                    }) => {
                        // Drop the terminal + guard to restore the normal terminal
                        // BEFORE the remote call so the remote client can own it.
                        drop(terminal);
                        drop(_guard);

                        let result = crate::remote::client::run_remote_client_target(
                            &target,
                            key.as_deref(),
                            None,
                            new_session,
                            session_id.as_deref(),
                        );

                        let remote_context = match &result {
                            Ok(crate::remote::client::RemoteExit::Resume { context }) => {
                                Some(context.clone())
                            }
                            _ => None,
                        };

                        if let Err(e) = &result {
                            crate::model::store::append_global_error_log(
                                "client",
                                &format!("remote connection failed: {e:#}"),
                            );
                            // Surface the error to the user via toast BEFORE
                            // teardown drops the request channel.
                            let _ = conn.req_tx.send(ClientRequest::ConnectFailed {
                                error: format!("{e:#}"),
                            });
                        }

                        teardown_connection(&handle, conn);

                        // Re-enter the terminal for the local swapper/attach loop.
                        _guard = TerminalGuard::enter()?;
                        crate::app::runtime::actions::apply_mouse_capture(
                            crate::model::settings::MouseCapture::Auto,
                        );
                        let backend = CrosstermBackend::new(stdout());
                        terminal = Terminal::new(backend)?;
                        terminal.clear()?;

                        // Return to the swapper — local or remote depending on outcome.
                        prev_session = current_session_id.take();
                        if let Some(mut context) = remote_context {
                            let mut rt = context.target;
                            if rt.key.is_none() {
                                rt.key = context.key_hint.take();
                            }
                            let password = context.password.clone();
                            remote_resume = Some((
                                rt.clone(),
                                password.clone(),
                                context.session_id.clone(),
                                context.cwd.clone(),
                            ));
                            let hub = build_remote_hub(
                                &rt,
                                password.as_deref(),
                                context.session_id.as_deref(),
                            );
                            ClientState::Swapper(hub)
                        } else {
                            remote_resume = None;
                            ClientState::Swapper(build_local_hub(prev_session.as_deref()))
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
            ClientState::Swapper(mut hub) => {
                // Use remote discovery when resuming from a remote session.
                let source = match &remote_resume {
                    Some((target, password, _, _)) => DiscoverySource::Remote {
                        target: target.clone(),
                        password: password.clone(),
                    },
                    None => DiscoverySource::Local,
                };
                match run_swapper(&mut terminal, &mut hub, prev_session.as_deref(), source)? {
                    // Picked a target session: attach to its daemon (spawning if needed),
                    // or re-SSH for remote picks. On success it becomes the foreground; on
                    // failure DEGRADE to the swapper rebuilt from fresh discovery rather
                    // than crash — the user can pick again.
                    SwapperOutcome::Pick {
                        session_id,
                        remote_host,
                        new_session,
                    } => {
                        if let Some(host) = remote_host {
                            // Remote pick: drop the terminal, run the remote session
                            // (it owns its own terminal lifecycle), then handle the
                            // exit outcome — Resume goes back to the remote swapper,
                            // Exit goes back to the local swapper.
                            drop(terminal);
                            drop(_guard);

                            let (attach_target, attach_key, attach_password) = remote_resume
                                .as_ref()
                                .map(|(target, password, _, _)| {
                                    let address = match target.port {
                                        Some(port) => {
                                            format!("{}@{}:{}", target.user, target.host, port)
                                        }
                                        None => format!("{}@{}", target.user, target.host),
                                    };
                                    (address, target.key.as_deref(), password.as_deref())
                                })
                                .unwrap_or((host.clone(), None, None));
                            let result = remote_attach(
                                &attach_target,
                                attach_key,
                                attach_password,
                                new_session,
                                if new_session { None } else { Some(session_id.as_str()) },
                            );

                            // Re-enter the terminal for the local swapper/attach loop.
                            _guard = TerminalGuard::enter()?;
                            crate::app::runtime::actions::apply_mouse_capture(
                                crate::model::settings::MouseCapture::Auto,
                            );
                            let backend = CrosstermBackend::new(stdout());
                            terminal = Terminal::new(backend)?;
                            terminal.clear()?;

                            match result {
                                Ok(crate::remote::client::RemoteExit::Resume { .. })
                                | Ok(crate::remote::client::RemoteExit::NewSession { .. }) => {
                                    // The user opened the swapper inside the remote
                                    // session — rebuild the remote hub.
                                    if let Ok(rt) = crate::remote::parse_target(&host) {
                                        // Keep the existing target/auth context when returning
                                        // from a remote session; a password prompt must not be
                                        // repeated merely because the user opened `/resume`.
                                        let (rt, password, remote_id, remote_cwd) =
                                            match remote_resume.take() {
                                                Some((saved, password, id, cwd)) => {
                                                    (saved, password, id, cwd)
                                                }
                                                None => (rt, None, None, None),
                                            };
                                        remote_resume = Some((
                                            rt.clone(),
                                            password.clone(),
                                            remote_id.clone(),
                                            remote_cwd,
                                        ));
                                        prev_session = current_session_id.take();
                                        let hub = build_remote_hub(
                                            &rt,
                                            password.as_deref(),
                                            remote_id.as_deref(),
                                        );
                                        ClientState::Swapper(hub)
                                    } else {
                                        // Can't parse target — degrade to local swapper.
                                        prev_session = current_session_id.take();
                                        remote_resume = None;
                                        ClientState::Swapper(build_local_hub(
                                            prev_session.as_deref(),
                                        ))
                                    }
                                }
                                Ok(crate::remote::client::RemoteExit::Exit) | Err(_) => {
                                    if let Some((rt, password, remote_id, _cwd)) =
                                        remote_resume.take()
                                    {
                                        remote_resume = Some((
                                            rt.clone(),
                                            password.clone(),
                                            remote_id.clone(),
                                            None,
                                        ));
                                        ClientState::Swapper(build_remote_hub(
                                            &rt,
                                            password.as_deref(),
                                            remote_id.as_deref(),
                                        ))
                                    } else {
                                        prev_session = current_session_id.take();
                                        ClientState::Swapper(build_local_hub(
                                            prev_session.as_deref(),
                                        ))
                                    }
                                }
                            }
                        } else {
                            // Local pick: existing path.
                            match attach_session(&mut terminal, &handle, &session_id) {
                                Ok(conn) => {
                                    current_session_id = Some(session_id);
                                    ClientState::Attached(conn)
                                }
                                Err(e) => {
                                    crate::model::store::append_global_error_log(
                                        "client",
                                        &format!("could not attach to session {session_id}: {e:#}"),
                                    );
                                    ClientState::Swapper(build_local_hub(prev_session.as_deref()))
                                }
                            }
                        }
                    }
                    // Cancelled: reconnect to the previous session if there was one; otherwise
                    // (a `--resume` cold start with nothing to return to) exit cleanly. A
                    // failed reconnect to a since-died previous daemon also degrades back to
                    // the swapper instead of crashing.
                    SwapperOutcome::Cancel => {
                        if let Some((target, password, session_id, _cwd)) = remote_resume.clone() {
                            let address = match target.port {
                                Some(port) => format!("{}@{}:{}", target.user, target.host, port),
                                None => format!("{}@{}", target.user, target.host),
                            };
                            drop(terminal);
                            drop(_guard);
                            let result = remote_attach(
                                &address,
                                target.key.as_deref(),
                                password.as_deref(),
                                false,
                                session_id.as_deref(),
                            );
                            _guard = TerminalGuard::enter()?;
                            crate::app::runtime::actions::apply_mouse_capture(
                                crate::model::settings::MouseCapture::Auto,
                            );
                            let backend = CrosstermBackend::new(stdout());
                            terminal = Terminal::new(backend)?;
                            terminal.clear()?;
                            match result {
                                Ok(crate::remote::client::RemoteExit::Resume { .. })
                                | Ok(crate::remote::client::RemoteExit::NewSession { .. }) => {
                                    ClientState::Swapper(build_remote_hub(
                                        &target,
                                        password.as_deref(),
                                        session_id.as_deref(),
                                    ))
                                }
                                Ok(crate::remote::client::RemoteExit::Exit) | Err(_) => {
                                    ClientState::Swapper(build_remote_hub(
                                        &target,
                                        password.as_deref(),
                                        session_id.as_deref(),
                                    ))
                                }
                            }
                        } else {
                            match prev_session.take() {
                                Some(prev) => match attach_session(&mut terminal, &handle, &prev) {
                                    Ok(conn) => {
                                        current_session_id = Some(prev);
                                        ClientState::Attached(conn)
                                    }
                                    Err(e) => {
                                        crate::model::store::append_global_error_log(
                                            "client",
                                            &format!("could not reconnect to session {prev}: {e:#}"),
                                        );
                                        ClientState::Swapper(build_local_hub(None))
                                    }
                                },
                                None => break,
                            }
                        }
                    },
                }
            }
        };
    }

    // Every live connection was already torn down inside the `Attached` arm it exited from
    // (so there is no double-detach and no connection to clean up here). A break straight
    // out of the swapper has no connection at all. Drop the runtime LAST so the active
    // connection's reader task (if any) is cancelled after exit.
    drop(rt);

    render_result
}

/// Attach to a session on a remote host over SSH.
///
/// Parses `target_str`, probes key auth, then runs a self-contained remote TUI
/// session (the same path as `koma remote`). On return, the caller re-enters
/// the terminal and handles the [`RemoteExit`] outcome.
///
/// This does NOT return a [`Connection`] because the SSH child process lifecycle
/// is managed internally by [`crate::remote::client::run_remote_client`]. The
/// remote session owns its own terminal, render loop, and cleanup.
fn remote_attach(
    target_str: &str,
    key: Option<&str>,
    password: Option<&str>,
    new_session: bool,
    session_id: Option<&str>,
) -> Result<crate::remote::client::RemoteExit> {
    // Apply the key hint if provided.
    let mut target = crate::remote::parse_target(target_str)?;
    if let Some(k) = key {
        target.key = Some(k.to_string());
    }

    // Probe key-based auth first (fast, silent), prompt for password if needed.
    let ssh_auth = match password {
        Some(password) => Some(crate::remote::auth::SshAuth::new(password.to_string())?),
        None => match crate::remote::auth::probe_key_auth(&target) {
            crate::remote::auth::AuthProbe::KeyReady => None,
            crate::remote::auth::AuthProbe::PasswordRequired => {
                eprintln!("Key-based authentication failed. Password required.");
                let password = crate::remote::auth::prompt_password(&target.user, &target.host)?;
                Some(crate::remote::auth::SshAuth::new(password)?)
            }
        },
    };

    let retained_password = ssh_auth
        .as_ref()
        .map(|auth| auth.password().to_string());
    let auth_ref = ssh_auth.as_ref();
    let cwd = if new_session {
        crate::remote::client::prompt_remote_cwd(&target, auth_ref)?
    } else {
        None
    };

    if new_session && cwd.is_none() {
        return Ok(crate::remote::client::RemoteExit::Resume {
            context: crate::remote::client::RemoteContext {
                target: target.clone(),
                key_hint: target.key.clone(),
                password: retained_password,
                session_id: None,
                cwd: None,
            },
        });
    }

    // Bootstrap koma on the remote host (ensures compatibility).
    eprintln!("Checking remote Koma version...");
    let _ = crate::remote::bootstrap::ensure_koma_compatible(&target, auth_ref)?;

    let mut requested_session_id = session_id.map(str::to_string);
    loop {
        match crate::remote::client::run_remote_client_with_cwd(
            &target,
            auth_ref,
            requested_session_id.as_deref(),
            cwd.as_deref(),
        )? {
            crate::remote::client::RemoteExit::NewSession { kill } => {
                let _ = kill;
                requested_session_id = None;
            }
            outcome => return Ok(outcome),
        }
    }
}

/// Run the TUI render loop for a remote connection.
///
/// Used by `koma remote` to display the remote koma server's UI over SSH.
/// Takes a pre-built [`Connection`] (from [`remote::connect_remote`]) and drives
/// the same render loop a local thin-client uses — so the remote UI is
/// byte-for-byte identical.
pub(crate) fn run_remote_render_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut conn: Connection,
    handle: &tokio::runtime::Handle,
) -> Result<render::ClientTransition> {
    let prebuffered = std::mem::take(&mut conn.prebuffered);
    let frame_rx = &conn.frame_rx;
    let req_tx = &conn.req_tx;

    let transition = {
        let _rt_ctx = handle.enter();
        render::render_loop(terminal, frame_rx, req_tx, prebuffered)
    };

    // A remote `/new kill` must reach the remote daemon before the SSH bridge is
    // consumed. Plain `/new` only detaches and leaves the old daemon resumable.
    if matches!(
        &transition,
        Ok(render::ClientTransition::NewSession { kill: true })
    ) {
        let _ = conn.req_tx.send(ClientRequest::QuitDaemon);
    }

    // Tear down the connection on every path — the caller owns the outcome.
    teardown_connection(handle, conn);

    transition
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

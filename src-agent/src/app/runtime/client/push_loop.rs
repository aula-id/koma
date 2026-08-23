//! The headless push-fold loop for the GUI host-relay bridge (the ATTACHED
//! twin of [`super::render::render_loop`]) — split out of `render.rs` for file
//! size (pure code motion, no behaviour change).
//!
//! [`push_loop`] drains the daemon's frames, folds them into a shadow `AppState`,
//! and pushes whatever changed via [`super::project::serialize_and_push`] /
//! [`super::project::push_hub`] / [`super::project::push_config`]. [`PushState`]
//! is the per-connection dedup memory both host states share (also read/written
//! directly by `project.rs`'s serialize/push helpers — hence its fields are
//! `pub(super)`, not just the struct itself). [`HostTransition`] is what the
//! caller ([`super::host::run_host_relay`]) does next.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use crate::app::mode::{Mode, SessionHub};
use crate::app::state::AppState;
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame};

use super::git_drain::drain_git_replies;
use super::git_host;
use super::project::{push_hub, serialize_and_push};
use super::project_config::{push_config, ConfigProjection};
use super::push_intercept;
use super::push_proto::{
    push_analytics, push_ext_op_result, push_file_diff, push_installed_extensions,
    push_remote_state, push_store_catalogue, push_store_detail, push_switching, push_tutorial_chat_done,
    push_usage_preview,
};
use super::render::{advance_local_animations, ConnectRemoteRequest, FRAME_BUDGET};
use super::shadow::apply_frame;
use super::store_host;
use super::tutorial_host;

/// Snapshot of the full `Status` envelope payload.
/// `(working, toast, toast_kind, tokens_in, tokens_cached, tokens_out, cost, mode)`.
type StatusSnapshot = (
    bool,
    Option<String>,
    Option<&'static str>,
    u64,
    u64,
    u64,
    f64,
    String,
);

/// Per-connection dedup memory for the push pipeline: the last values pushed, so
/// [`serialize_and_push`] / [`push_hub`] only emit an envelope when something
/// actually changed (the fold loop calls them every ~16ms).
pub(super) struct PushState {
    /// Fingerprint of the last `Snapshot` (session + messages + title + palette).
    pub(super) snapshot_fp: Option<u64>,
    /// Last streaming buffer pushed (`None` once cleared).
    pub(super) stream: Option<String>,
    /// Last reasoning buffer pushed (empty once cleared).
    pub(super) reasoning: String,
    /// Last `(working, toast, toast_kind, tokens_in, tokens_cached, tokens_out, cost,
    /// mode)` pushed — the full `Status` envelope payload, so a counter tick, a
    /// mode flip, or a working/toast change each independently re-emit `Status`.
    /// `cost` is `f64`; plain `!=` (`PartialEq`, not `Eq`) is fine here — this tuple
    /// is only ever compared, never hashed or used as a map key.
    pub(super) status: Option<StatusSnapshot>,
    /// Last serialised `Hub` JSON (the swapper is diffed as a whole).
    pub(super) hub_json: Option<String>,
    /// Last serialised `Config` JSON (the global config catalogue, diffed as a whole so
    /// an unchanged config emits nothing). Cleared by [`reset`](Self::reset) on `Ready`
    /// so a page reload re-emits the full current catalogue.
    pub(super) config_json: Option<String>,
    /// Last `(active, workspace, awareness)` pushed as a `Loading` envelope. `None`
    /// until the foreground session first enters `Mode::Loading`. Compared as a
    /// whole triple so ANY phase change (workspace or awareness ticking
    /// pending/running/done/skipped/failed) re-emits, and — critically — its
    /// `active` flag is read back by `serialize_and_push` to detect the Loading →
    /// non-Loading transition (the last emitted frame is the only record of
    /// whether the webview still thinks a splash is up).
    pub(super) last_loading: Option<(bool, String, String)>,
}

impl PushState {
    pub(super) fn new() -> Self {
        Self {
            snapshot_fp: None,
            stream: None,
            reasoning: String::new(),
            status: None,
            hub_json: None,
            config_json: None,
            last_loading: None,
        }
    }

    /// Forget every last-pushed value so the next [`serialize_and_push`] re-emits the
    /// FULL current state. Called on a `Ready` (the page (re)booted and needs a fresh
    /// authoritative snapshot), so a webview reload never renders against stale deltas.
    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }
}

/// What [`push_loop`] resolved to — the instruction the host-relay state machine in
/// [`super::run_host_relay`] acts on next. Mirrors [`ClientTransition`] for the
/// headless GUI host (no terminal): leave, fall back to the swapper, or attach a
/// different session.
pub(super) enum HostTransition {
    /// Leave the host entirely (the control channel closed — the window is gone).
    Exit,
    /// Detach and show the local session swapper (the daemon's socket closed, or it
    /// signalled `OpenSwapper`). `run_host_relay` rebuilds the hub from discovery.
    ToSwapper,
    /// Host is authenticated/bootstrapped; show the remote hub (no SSH session child).
    ToRemoteHub {
        ctx: Box<super::remote_ctl::RemoteCtx>,
    },
    /// SSH-attach a specific remote session (existing id, or a freshly minted one with cwd).
    RemoteAttach {
        ctx: Box<super::remote_ctl::RemoteCtx>,
        session_id: String,
        cwd: Option<String>,
    },
    /// Live SSH-bridged remote session (after session attach succeeded).
    Remote {
        connection: Box<super::remote_ctl::ActiveRemote>,
        session_id: String,
    },
    /// Drop remote context and return to the local swapper.
    DisconnectRemote,
    /// Attach to this local session UUID (a hub `SelectSession`/`NewSession`, or a daemon
    /// `NewSession` hand-off). A minted uuid for a new session; an existing id otherwise.
    /// `workdir` is the folder a GUI `[+ new session]` native picker chose (the new
    /// session's working dir); `None` for every other attach inherits the host's cwd.
    /// Local-only — never used while a remote ctx is live.
    Attach {
        id: String,
        workdir: Option<std::path::PathBuf>,
    },
}

/// The HEADLESS twin of [`render_loop`]: fold the daemon's frames into the shadow and
/// PUSH the resulting state to the webview instead of drawing it to a terminal. Same
/// 16ms cadence, same non-blocking frame drain + local-animation advance + toast
/// sweep, but the crossterm input poll is gone (input arrives as `HostCtl` from the
/// ipc thread and `SubmitInput` goes straight to the daemon over `req_tx`).
///
/// Each frame, in order: (0) drain `ctl_rx` — `Ready` forces a full re-push, a
/// `Select`/`New` returns an [`HostTransition::Attach`]; (a) drain every queued
/// [`DaemonFrame`] and apply it (an `OpenSwapper`/`NewSession` hand-off returns the
/// matching transition, a closed socket returns [`HostTransition::ToSwapper`]); (b)
/// advance the local-clock animations + sweep the toast; (c) serialise the shadow and
/// push whatever changed; then pace to the frame budget. Returns when a transition is
/// resolved.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(super) fn push_loop(
    push: &dyn Fn(String),
    frame_rx: &Receiver<DaemonFrame>,
    req_tx: &Sender<ClientRequest>,
    prebuffered: Vec<DaemonFrame>,
    ctl_tx: &Sender<super::HostCtl>,
    ctl_rx: &Receiver<super::HostCtl>,
    last: &mut PushState,
    current_session: Option<&str>,
    live_marks: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    live_view: &std::sync::Arc<std::sync::Mutex<super::StreamView>>,
    terminal_manager: &std::sync::Arc<std::sync::Mutex<super::terminal_host::TerminalManager>>,
    lsp_manager: &std::sync::Arc<std::sync::Mutex<crate::lsp::LspManager>>,
    // When set, this fold is an SSH-bridged remote session: leave/kill returns to
    // the remote hub (not the local swapper), and KillSession uses remote SSH kill.
    remote_ctx: Option<&super::remote_ctl::RemoteCtx>,
    // When set, Coding panel File* ops go to `koma remote-fs` over SSH (local
    // session_workdirs_for is wrong for remote attach).
    remote_fs: Option<&super::remote_fs_client::RemoteFsClient>,
    // When set, Source Control Git* ops go to `koma remote-git` over SSH (local
    // repo_root_for is wrong for remote attach).
    remote_git: Option<&super::remote_git_client::RemoteGitClient>,
    // When set, Import Graph ops go to `koma remote-linker` over SSH (local
    // session_workdirs_for / local linker daemon are wrong for remote attach).
    #[cfg(feature = "linker")]
    remote_linker: Option<&super::remote_linker_client::RemoteLinkerClient>,
) -> HostTransition {
    use std::sync::mpsc::TryRecvError;

    // The shadow is a real AppState reconstructed purely from frames (identical to
    // `render_loop`); the first Snapshot replaces the neutral placeholder.
    let mut shadow = AppState::new(Mode::Chat);
    shadow.rest.fg_mut().status = "attaching…".into();

    let mut expected: u64 = 0;
    let mut seeded = false;
    let mut awaiting_resync = false;

    // Latest authoritative config catalogue, cached off each incoming full snapshot so
    // `push_config` can (re)emit the `Config` envelope every frame (dedup'd) — including
    // after a `Ready` reset, without waiting for the daemon to resend a snapshot.
    let mut current_config: Option<ConfigProjection> = None;

    // --- attached-state hub refresh (RefreshHub) ---
    // Cross-daemon discovery (`build_local_hub` → `list_live_sessions`) BLOCKS on a
    // per-socket Status probe, so it must NOT run inline on this 16ms fold loop (it
    // would stall frame folding + animation for the whole multi-socket sweep). Instead
    // a `RefreshHub` spawns a ONE-SHOT worker thread that runs the blocking sweep off
    // this thread and ships the built `SessionHub` back over `hub_rx`; the loop drains
    // it non-blocking and calls `push_hub` (which diffs `last.hub_json`, so a no-change
    // refresh is silent and repeated palette-opens are cheap). `refresh_inflight`
    // coalesces bursts — React may re-emit RefreshHub on an interval while the palette
    // stays open — so at most one sweep runs at a time. `current_owned` flags the
    // attached row as `is_foreground` in the rebuilt hub.
    let (hub_tx, hub_rx) = std::sync::mpsc::channel::<SessionHub>();
    let mut refresh_inflight = false;
    let current_owned: Option<String> = current_session.map(str::to_string);

    // --- FILE CHANGED diff fetch (FileDiff) ---
    // `compute_file_diff` shells out to git + reads the file, both blocking, so — same
    // reasoning as `RefreshHub` above — it runs on a one-shot worker thread rather than
    // inline on this 16ms fold loop; the loop drains completed results non-blocking and
    // pushes each as a `FileDiff` envelope. Unlike the hub refresh there is no "latest
    // wins" coalescing: each request is for a (possibly different) path, so every
    // completed result is pushed, not just the newest.
    let (file_diff_tx, file_diff_rx) = std::sync::mpsc::channel::<super::diff::FileDiffResult>();

    // --- GIT STATUS / DIFF / OP / GRAPH / COMMIT DETAIL / COMMIT DIFF fetches ---
    // Each shells out to git (blocking, same reasoning as `FileDiff` above), so each
    // runs on a one-shot worker thread; the loop drains completed results
    // non-blocking and pushes each as its own envelope (see `git_drain`'s module
    // doc). No coalescing needed — each is a self-contained, per-request reply. A
    // `GitOp` mutation's worker ALSO recomputes + resends over `git_status_tx`
    // (reusing the channel below), refreshing the panel right after every op.
    let (git_status_tx, git_status_rx) = std::sync::mpsc::channel::<super::git::GitStatusResult>();
    let (git_diff_tx, git_diff_rx) = std::sync::mpsc::channel::<super::git::GitDiffResult>();
    let (git_op_tx, git_op_rx) = std::sync::mpsc::channel::<super::git::GitOpResult>();
    let (git_graph_tx, git_graph_rx) =
        std::sync::mpsc::channel::<super::git_graph::GitGraphResult>();
    let (commit_detail_tx, commit_detail_rx) =
        std::sync::mpsc::channel::<super::git_graph::CommitDetailResult>();
    let (commit_diff_tx, commit_diff_rx) =
        std::sync::mpsc::channel::<super::git_graph::CommitDiffResult>();

    // --- USAGE PANEL preview fetch (UsagePreview) ---
    // `compute_usage_preview` hits sqlite, blocking, so — same reasoning as `FileDiff`
    // above — it runs on a one-shot worker thread; the loop drains completed results
    // non-blocking and pushes each as a `UsagePreview` envelope. The `String` riding
    // alongside is the request's `scope` ("all"/"session"); the `Option<String>` is the
    // `session` uuid that was ACTUALLY queried (only `Some` for a real "session" scope).
    // Both are echoed back unchanged so React can drop a reply whose scope OR session id
    // no longer matches what's currently selected/attached — a rapid toggle, OR a
    // foreground session switch, racing an in-flight request must never render the
    // wrong session's numbers.
    let (usage_preview_tx, usage_preview_rx) =
        std::sync::mpsc::channel::<(super::diff::UsagePreviewResult, String, Option<String>)>();

    // --- ANALYTICS DASHBOARD fetch (Analytics) ---
    // Same reasoning as UsagePreview: sqlite is blocking, so it runs on a one-
    // shot worker thread; the loop drains completed results non-blocking and
    // pushes each as an `Analytics` envelope. Correlation fields ride inside
    // `AnalyticsResult` so React can drop a stale reply across rapid filter /
    // session changes.
    let (analytics_tx, analytics_rx) = std::sync::mpsc::channel::<super::diff::AnalyticsResult>();

    // --- BRANCH LIST (GitBranchList) --- one-shot worker thread (blocking `git
    // for-each-ref`), like `GitGraph` above. `GitCheckout`/`GitCreateBranch`
    // reuse the EXISTING `git_op_tx`/`git_status_tx` channels below instead of a
    // dedicated channel (same `GitOp` + follow-up `GitStatus` reply pattern).
    let (branch_list_tx, branch_list_rx) =
        std::sync::mpsc::channel::<super::git_branch::BranchListResult>();

    // --- REPO LIST (GitRepos) --- multi-repo picker discovery, one-shot worker
    // thread (blocking filesystem walk), like `GitBranchList` above.
    // `SetActiveRepo` carries no dedicated channel — it reuses the EXISTING
    // `git_status_tx` below (a follow-up `GitStatus` for the newly-active repo,
    // same pattern as `SetGitKey`).
    let (repo_list_tx, repo_list_rx) =
        std::sync::mpsc::channel::<super::git_repos::RepoListResult>();

    // --- STASH (GitStashList) --- one-shot worker thread (blocking `git stash
    // list`), like `GitBranchList` above (GK4a). `GitStash`/`GitStashPop` reuse the
    // EXISTING `git_op_tx`/`git_status_tx` channels instead (same `GitOp` + follow-up
    // `GitStatus` reply pattern, since stashing changes the working tree).
    let (stash_list_tx, stash_list_rx) =
        std::sync::mpsc::channel::<super::git_stash::StashListResult>();

    // --- ACTIVITY (GitActivity, GK5a) --- one-shot worker thread (blocking `git log
    // --numstat`), like `GitGraph` above — a self-contained per-request reply, no
    // follow-up `GitStatus` needed (read-only, changes nothing).
    let (activity_tx, activity_rx) =
        std::sync::mpsc::channel::<super::git_activity::ActivityResult>();

    // --- SSH KEY VAULT (KeyList/KeyGenerate/KeyImport/KeyDelete/KeyReveal) ---
    // Every op shells `ssh-keygen`/touches the filesystem (blocking), same
    // reasoning as the GIT channels above. A mutation (generate/import/delete)
    // ALSO resends a refreshed list over `key_list_tx`, mirroring the GIT
    // mutation's status re-fetch.
    let (key_list_tx, key_list_rx) = std::sync::mpsc::channel::<Vec<super::keys::KeyInfo>>();
    let (key_reveal_tx, key_reveal_rx) = std::sync::mpsc::channel::<super::keys::KeyRevealResult>();
    let (key_op_tx, key_op_rx) = std::sync::mpsc::channel::<super::keys::KeyOpResult>();

    // --- Language servers (LspStatus/LspInstall/LspUninstall) --- host-local
    // under ~/.koma/lsp/. Install is blocking network + toolchain work, so it
    // runs off-thread; progress + final status drain via `lsp_tx`.
    let (lsp_tx, lsp_rx) = std::sync::mpsc::channel::<super::lsp_host::LspReply>();

    // --- IMPORT GRAPH (ImportGraph) --- off-thread linker daemon IPC, same reasoning
    // as the GIT channels above (blocking IPC to the linker daemon).
    #[cfg(feature = "linker")]
    let (import_graph_tx, import_graph_rx) =
        std::sync::mpsc::channel::<super::import_graph::ImportGraphResult>();

    // --- IMPORT GRAPH IMPACT (ImportGraphImpact) --- off-thread linker daemon
    // IPC for transitive impact analysis. Blocking IPC must never run on the
    // 16ms fold loop; a dedicated channel drains non-blocking like ImportGraph.
    #[cfg(feature = "linker")]
    let (impact_tx, impact_rx) =
        std::sync::mpsc::channel::<super::push_proto::ImportGraphImpactResult>();

    // --- Extension STORE (StoreBrowse/StoreDetail/ListInstalledExtensions) ---
    // Browse/detail are a blocking `reqwest` GET (koma.run, PUBLIC/no-auth) and the
    // installed-list is a blocking `~/.koma/config.json` read, so — same reasoning as
    // the GIT/key channels above — each runs on a one-shot worker thread via the
    // shared `store_host` bodies (also used by the detached `host_swapper` twin);
    // NEVER touches the daemon in either host state (unlike `ListModels`/`ListRoutes`
    // above, which DO forward to the daemon while attached).
    let (store_catalogue_tx, store_catalogue_rx) =
        std::sync::mpsc::channel::<(Vec<crate::ipc::proto::StoreItemWire>, Option<String>)>();
    let (store_detail_tx, store_detail_rx) =
        std::sync::mpsc::channel::<(Option<crate::ipc::proto::StoreDetailWire>, Option<String>)>();
    let (installed_ext_tx, installed_ext_rx) =
        std::sync::mpsc::channel::<Vec<crate::ipc::proto::InstalledExtWire>>();
    let (installed_detail_tx, installed_detail_rx) = std::sync::mpsc::channel::<(
        String,
        Option<crate::ipc::proto::InstalledExtensionDetailWire>,
        Option<String>,
    )>();

    // --- GUI Tutorial chat (TutorialChat) ---
    // Blocking koma-free HTTP; host-local regardless of attach state.
    let (tutorial_chat_tx, tutorial_chat_rx) =
        std::sync::mpsc::channel::<tutorial_host::TutorialChatResult>();

    // --- REMOTE HOST CONNECT/DISCONNECT ---
    // Worker thread pushes state transitions (resolving → auth_required →
    // bootstrapping → ready, etc.) over this channel; the loop drains and
    // pushes them as `RemoteState` envelopes. Host connect stops at ready
    // (no SSH session child); session attach is a separate HostStep.
    let (remote_state_tx, remote_state_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::RemoteStateUpdate>();
    let (remote_ready_tx, remote_ready_rx) =
        std::sync::mpsc::channel::<super::remote_ctl::ReadyRemote>();
    let remote_shared = std::sync::Arc::new(super::remote_ctl::RemoteSessionShared::new());

    // Fold the handshake's prebuffered frames first, through the SAME `apply_frame`
    // path (seq seeding stays gap-free). The select/swapper/new latches can't fire
    // this early, so the throwaways here are never acted on.
    {
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        let mut connect_remote_requested: Option<ConnectRemoteRequest> = None;
        for frame in prebuffered {
            // Cache the config off any prebuffered full snapshot (normally none — Hello
            // is first, so the attach Snapshot lands in the live drain — but stay safe).
            if let DaemonEvent::Snapshot(snap) = &frame.event {
                let proj = ConfigProjection::from_global(&snap.global);
                current_config = Some(proj);
            }
            apply_frame(
                frame,
                &mut shadow,
                &mut expected,
                &mut seeded,
                &mut awaiting_resync,
                &mut select_requested,
                &mut open_swapper_requested,
                &mut new_session_requested,
                &mut connect_remote_requested,
                req_tx,
            );
        }
    }

    loop {
        let frame_start = Instant::now();

        // --- (0) control messages from the ipc thread (NON-BLOCKING) ---
        loop {
            match ctl_rx.try_recv() {
                // The page (re)booted: re-push the full authoritative state this frame.
                Ok(super::HostCtl::Ready) => last.reset(),
                // A hub pick / new-session request: signal swap-START (so React raises the
                // loader BEFORE this attached push_loop returns + the connection is torn
                // down — the ONLY seam still holding a live socket), then hand back to the
                // state machine to detach + attach the chosen (or freshly minted) session.
                //
                // While remote-attached, Select/New MUST stay on the remote host
                // (RemoteAttach) — never fall through to a local Attach, which would
                // target a local socket for a remote session id.
                Ok(super::HostCtl::Select(id)) => {
                    push_switching(push, &id);
                    if let Some(ctx) = remote_ctx {
                        return HostTransition::RemoteAttach {
                            ctx: Box::new(ctx.clone()),
                            session_id: id,
                            cwd: None,
                        };
                    }
                    return HostTransition::Attach { id, workdir: None };
                }
                // `[+ new session]` while attached: the GUI picker already confirmed a
                // folder (a cancel sends no `New`), so carry it into the fresh session. On
                // `kill` reap the CURRENT daemon as part of the switch — queue a graceful
                // QuitDaemon on the live conn (flushed by the upcoming teardown, mirroring the
                // TUI `/new kill`) and ensure its death OFF-thread so the fresh attach never
                // waits on the old daemon's corpse. `kill: false` leaves the old daemon
                // cooking (resumable), exactly as before.
                //
                // Remote: open the remote path picker instead of a local folder dialog —
                // ConfirmRemotePath mints the id + cwd and returns RemoteAttach.
                Ok(super::HostCtl::New { workdir, kill }) => {
                    if let Some(ctx) = remote_ctx {
                        if kill {
                            if let Some(old) = current_owned.clone() {
                                let _ = req_tx.send(ClientRequest::QuitDaemon);
                                super::host::spawn_remote_kill_and_refresh(
                                    ctl_tx.clone(),
                                    ctx.target.clone(),
                                    ctx.password.clone(),
                                    old,
                                );
                            }
                        }
                        // Local `workdir` from a native picker is meaningless remotely.
                        let _ = workdir;
                        // Open the path picker as soon as the remote hub loop starts.
                        let _ = ctl_tx.send(super::HostCtl::RequestRemotePath);
                        return HostTransition::ToRemoteHub {
                            ctx: Box::new(ctx.clone()),
                        };
                    }
                    if kill {
                        if let Some(old) = current_owned.clone() {
                            let _ = req_tx.send(ClientRequest::QuitDaemon);
                            super::host::spawn_ensure_dead(old);
                        }
                    }
                    let new_id = uuid::Uuid::new_v4().to_string();
                    push_switching(push, &new_id);
                    return HostTransition::Attach {
                        id: new_id,
                        workdir,
                    };
                }
                // Remote path controls: while remote-attached, leave to the remote hub so
                // the path picker can run there (hub owns list_dirs over SSH).
                // Re-queue RequestRemotePath so the hub opens the picker immediately
                // (same pattern as HostCtl::New above).
                Ok(super::HostCtl::RequestRemotePath) => {
                    if let Some(ctx) = remote_ctx {
                        let _ = ctl_tx.send(super::HostCtl::RequestRemotePath);
                        return HostTransition::ToRemoteHub {
                            ctx: Box::new(ctx.clone()),
                        };
                    }
                    let envelope = serde_json::json!({
                        "k": "RemotePathPicker",
                        "state": "error",
                        "error": "active session is not remote"
                    });
                    if let Ok(json) = serde_json::to_string(&envelope) {
                        push(json);
                    }
                }
                Ok(super::HostCtl::ListRemotePath { .. })
                | Ok(super::HostCtl::ConfirmRemotePath { .. })
                | Ok(super::HostCtl::CancelRemotePath) => {
                    // Dismiss the picker. Do NOT leave state=error — that keeps
                    // the overlay open and freezes chrome under a dead dialog.
                    let close = serde_json::json!({
                        "k": "RemotePathPicker",
                        "state": "cancelled",
                    });
                    if let Ok(json) = serde_json::to_string(&close) {
                        push(json);
                    }
                }
                // KILL the daemon `id`. Killing the CURRENTLY-ATTACHED session: queue a
                // graceful QuitDaemon on the live conn (flushed by teardown), ensure its death
                // OFF-thread — a harmless double-QuitDaemon that ALSO fires a follow-up
                // RefreshHub so the hub we're about to land in drops the row the instant
                // it is gone — then hand back to the hub. Remote attach returns to the
                // remote hub; local returns to the local swapper. A BACKGROUND kill just
                // escalates OFF-thread and refreshes the hub once the daemon is confirmed dead.
                Ok(super::HostCtl::KillSession(id)) => {
                    let leave_current = current_owned.as_deref() == Some(id.as_str());
                    if leave_current {
                        let _ = req_tx.send(ClientRequest::QuitDaemon);
                    }
                    if let Some(ctx) = remote_ctx {
                        super::host::spawn_remote_kill_and_refresh(
                            ctl_tx.clone(),
                            ctx.target.clone(),
                            ctx.password.clone(),
                            id,
                        );
                        if leave_current {
                            return HostTransition::ToRemoteHub {
                                ctx: Box::new(ctx.clone()),
                            };
                        }
                    } else {
                        super::host::spawn_kill_and_refresh(ctl_tx.clone(), id);
                        if leave_current {
                            return HostTransition::ToSwapper;
                        }
                    }
                }
                // Physically DELETE a history session OFF-thread (guarded host-side against
                // deleting a live/locked session), then RefreshHub. A history row is never the
                // attached session, so there is no live-conn interaction here.
                // Remote attach → SSH `daemon delete`; local → laptop store delete.
                Ok(super::HostCtl::DeleteSession(id)) => {
                    if let Some(ctx) = remote_ctx {
                        super::host::spawn_remote_delete_and_refresh(
                            ctl_tx.clone(),
                            ctx.target.clone(),
                            ctx.password.clone(),
                            id,
                        );
                    } else {
                        super::host::spawn_delete_and_refresh(ctl_tx.clone(), id);
                    }
                }
                // Cancel-switch (best-effort): the swap in flight can't be interrupted, so
                // this simply drops to the hub AFTER the current/queued attach resolves.
                // While remote-attached, cancel returns to the *remote* hub (keep
                // ControlMaster + host ctx) — never full host disconnect.
                Ok(super::HostCtl::ToSwapper) => {
                    return if let Some(ctx) = remote_ctx {
                        HostTransition::ToRemoteHub {
                            ctx: Box::new(ctx.clone()),
                        }
                    } else {
                        HostTransition::ToSwapper
                    };
                }
                // The ResumePalette opened: kick a hub refresh OFF this thread (the
                // discovery sweep blocks). Coalesced by `refresh_inflight` so a burst of
                // RefreshHubs while the palette stays open runs at most one sweep; the
                // result is drained + pushed below.
                Ok(super::HostCtl::RefreshHub) => {
                    if !refresh_inflight {
                        refresh_inflight = true;
                        let tx = hub_tx.clone();
                        let cur = current_owned.clone();
                        if let Some(ctx) = remote_ctx {
                            let target = ctx.target.clone();
                            let password = ctx.password.clone();
                            std::thread::spawn(move || {
                                let hub = super::swapper::build_remote_hub(
                                    &target,
                                    password.as_deref(),
                                    cur.as_deref(),
                                );
                                let _ = tx.send(hub);
                            });
                        } else {
                            std::thread::spawn(move || {
                                let hub = super::swapper::build_local_hub(cur.as_deref());
                                let _ = tx.send(hub);
                            });
                        }
                    }
                }
                // A config mutation raced in while attached (the ipc handler normally
                // routes these straight to the daemon via `live_req` when a session is
                // attached; this only lands here if the attach state flipped between the
                // check and the send). Forward the carried request to the daemon — it owns
                // the authoritative config and re-pushes a fresh `Config` on the change.
                Ok(super::HostCtl::ConfigMutate(req)) => {
                    let _ = req_tx.send(req);
                }
                // A live model / route fetch raced in while attached (the ipc handler routes
                // these straight to the daemon via `live_req` when attached; they only land
                // here if the attach state flipped between the detached-check and the send).
                // Forward the equivalent daemon request — the daemon fetches + replies
                // out-of-band and the `ModelList`/`ModelRoutes` frame is re-pushed above — so
                // the reply is never dropped on the race.
                Ok(super::HostCtl::ListModels { provider }) => {
                    let _ = req_tx.send(ClientRequest::ListModels { provider });
                }
                Ok(super::HostCtl::ListRoutes { provider, model_id }) => {
                    let _ = req_tx.send(ClientRequest::ListRoutes { provider, model_id });
                }
                // GUI Settings fetch raced in while attached (the ipc handler routes it to
                // the daemon via `live_req` when attached; it only lands here if the attach
                // state flipped between the detached-check and the send). Forward the daemon
                // request — the daemon replies with `SettingsValues`, re-pushed above.
                Ok(super::HostCtl::GetSettings) => {
                    let _ = req_tx.send(ClientRequest::GetSettings);
                }
                // GUI /agents fetch raced in while attached (routed to the daemon via
                // `live_req` normally; only lands here on an attach-state flip). Forward the
                // daemon request — it replies with `AgentsValues`, re-pushed above.
                Ok(super::HostCtl::GetAgents) => {
                    let _ = req_tx.send(ClientRequest::ListAgents);
                }
                // GUI OAuth fetch / delete raced in while attached (dual-routed to the daemon
                // via `live_req` normally; only lands here on an attach-state flip). Forward
                // the daemon request — it replies with `OAuthState`, re-pushed above.
                Ok(super::HostCtl::GetOAuthState) => {
                    let _ = req_tx.send(ClientRequest::GetOAuthState);
                }
                Ok(super::HostCtl::DeleteOAuthConn { uuid }) => {
                    let _ = req_tx.send(ClientRequest::DeleteOAuthConn { uuid });
                }
                // GUI OAuth start/cancel raced in while attached (same race window as
                // `GetOAuthState`/`DeleteOAuthConn` above — the ipc handler routes these to
                // the daemon via `live_req` normally, so this only lands here on an
                // attach-state flip). Forward the equivalent daemon request rather than
                // running the host-local flow while a session IS attached — the daemon
                // owns the flow in that case, exactly as before this wave.
                Ok(super::HostCtl::StartOAuth { provider }) => {
                    let _ = req_tx.send(ClientRequest::StartOAuth { provider });
                }
                Ok(super::HostCtl::CancelOAuth) => {
                    let _ = req_tx.send(ClientRequest::CancelOAuth);
                }
                // FILE CHANGED diff fetch: NEVER touches the daemon (host-side only,
                // regardless of attach state) — spawn the blocking git+fs work off this
                // thread; the result is drained + pushed below at (b-quat).
                Ok(super::HostCtl::FileDiff { path }) => {
                    let tx = file_diff_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let result = super::diff::compute_file_diff(&path, cur.as_deref());
                        let _ = tx.send(result);
                    });
                }
                // Explore GIT panel: host-local for local sessions; remote attach
                // routes through `koma remote-git` (never laptop tooling against remote paths).
                // SSH-key vault (Key*) stays host-local below — keys live in ~/.koma/keys
                // on the GUI host.
                Ok(ctl @ super::HostCtl::GitStatus)
                | Ok(ctl @ super::HostCtl::GitDiff { .. })
                | Ok(ctl @ super::HostCtl::GitStage { .. })
                | Ok(ctl @ super::HostCtl::GitUnstage { .. })
                | Ok(ctl @ super::HostCtl::GitDiscard { .. })
                | Ok(ctl @ super::HostCtl::GitCommit { .. })
                | Ok(ctl @ super::HostCtl::SetGitKey { .. })
                | Ok(ctl @ super::HostCtl::GitFetch)
                | Ok(ctl @ super::HostCtl::GitPull)
                | Ok(ctl @ super::HostCtl::GitPush { .. })
                | Ok(ctl @ super::HostCtl::GitStash)
                | Ok(ctl @ super::HostCtl::GitStashPop)
                | Ok(ctl @ super::HostCtl::GitStashList)
                | Ok(ctl @ super::HostCtl::GitBranchList { .. })
                | Ok(ctl @ super::HostCtl::GitRepos)
                | Ok(ctl @ super::HostCtl::SetActiveRepo { .. })
                | Ok(ctl @ super::HostCtl::GitCheckout { .. })
                | Ok(ctl @ super::HostCtl::GitCreateBranch { .. })
                | Ok(ctl @ super::HostCtl::GitCherryPick { .. })
                | Ok(ctl @ super::HostCtl::GitRevert { .. })
                | Ok(ctl @ super::HostCtl::GitReset { .. })
                | Ok(ctl @ super::HostCtl::GitMerge { .. })
                | Ok(ctl @ super::HostCtl::GitRebase { .. })
                | Ok(ctl @ super::HostCtl::GitOpAbort { .. })
                | Ok(ctl @ super::HostCtl::GitOpContinue { .. })
                | Ok(ctl @ super::HostCtl::GitGraph { .. })
                | Ok(ctl @ super::HostCtl::GitCommitDetail { .. })
                | Ok(ctl @ super::HostCtl::GitCommitDiff { .. })
                | Ok(ctl @ super::HostCtl::GitActivity { .. }) => {
                    if let Some(rg) = remote_git {
                        rg.handle_git_ctl(&ctl, push);
                    } else if remote_ctx.is_some() {
                        // Remote attach but remote-git child failed — never local tooling.
                        push_remote_git_unavailable(&ctl, push);
                    } else {
                        match ctl {
                            super::HostCtl::GitStatus => {
                                git_host::spawn_git_status_attached(
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::GitDiff { path, staged } => {
                                git_host::spawn_git_diff_attached(
                                    git_diff_tx.clone(),
                                    current_owned.clone(),
                                    path,
                                    staged,
                                );
                            }
                            super::HostCtl::GitStage { paths } => {
                                git_host::spawn_git_stage_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    paths,
                                );
                            }
                            super::HostCtl::GitUnstage { paths } => {
                                git_host::spawn_git_unstage_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    paths,
                                );
                            }
                            super::HostCtl::GitDiscard { paths } => {
                                git_host::spawn_git_discard_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    paths,
                                );
                            }
                            super::HostCtl::GitCommit { message } => {
                                git_host::spawn_git_commit_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    message,
                                );
                            }
                            super::HostCtl::SetGitKey { name } => {
                                git_host::spawn_set_git_key_attached(
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    name,
                                );
                            }
                            super::HostCtl::GitFetch => {
                                git_host::spawn_git_fetch_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::GitPull => {
                                git_host::spawn_git_pull_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::GitPush { mode, root } => {
                                git_host::spawn_git_push_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    mode,
                                    root,
                                );
                            }
                            super::HostCtl::GitStash => {
                                git_host::spawn_git_stash_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::GitStashPop => {
                                git_host::spawn_git_stash_pop_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::GitStashList => {
                                git_host::spawn_git_stash_list_attached(
                                    stash_list_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::GitBranchList { request_id } => {
                                git_host::spawn_git_branch_list_attached(
                                    branch_list_tx.clone(),
                                    current_owned.clone(),
                                    request_id,
                                );
                            }
                            super::HostCtl::GitRepos => {
                                git_host::spawn_git_repos_attached(
                                    repo_list_tx.clone(),
                                    current_owned.clone(),
                                );
                            }
                            super::HostCtl::SetActiveRepo { root } => {
                                git_host::spawn_set_active_repo_attached(
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    root,
                                );
                            }
                            super::HostCtl::GitCheckout { ref_name, root } => {
                                git_host::spawn_git_checkout_attached(
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                    ref_name,
                                    root,
                                );
                            }
                            super::HostCtl::GitCreateBranch {
                                name,
                                start,
                                checkout,
                                root,
                            } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_create_branch_attached(
                                    ot, st, cur, name, start, checkout, root,
                                );
                            }
                            super::HostCtl::GitCherryPick { sha } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_cherry_pick_attached(ot, st, cur, sha);
                            }
                            super::HostCtl::GitRevert { sha } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_revert_attached(ot, st, cur, sha);
                            }
                            super::HostCtl::GitReset { sha, mode } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_reset_attached(ot, st, cur, sha, mode);
                            }
                            super::HostCtl::GitMerge { ref_name } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_merge_attached(ot, st, cur, ref_name);
                            }
                            super::HostCtl::GitRebase { upstream, branch } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_rebase_attached(ot, st, cur, upstream, branch);
                            }
                            super::HostCtl::GitOpAbort { kind } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_op_abort_attached(ot, st, cur, kind);
                            }
                            super::HostCtl::GitOpContinue { kind } => {
                                let (ot, st, cur) = (
                                    git_op_tx.clone(),
                                    git_status_tx.clone(),
                                    current_owned.clone(),
                                );
                                git_host::spawn_git_op_continue_attached(ot, st, cur, kind);
                            }
                            super::HostCtl::GitGraph { limit, skip } => {
                                git_host::spawn_git_graph_attached(
                                    git_graph_tx.clone(),
                                    current_owned.clone(),
                                    limit,
                                    skip,
                                );
                            }
                            super::HostCtl::GitCommitDetail { sha } => {
                                git_host::spawn_commit_detail_attached(
                                    commit_detail_tx.clone(),
                                    current_owned.clone(),
                                    sha,
                                );
                            }
                            super::HostCtl::GitCommitDiff { sha, path } => {
                                git_host::spawn_commit_diff_attached(
                                    commit_diff_tx.clone(),
                                    current_owned.clone(),
                                    sha,
                                    path,
                                );
                            }
                            super::HostCtl::GitActivity { path, limit } => {
                                git_host::spawn_git_activity_attached(
                                    activity_tx.clone(),
                                    current_owned.clone(),
                                    path,
                                    limit,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                                // USAGE PANEL preview fetch: NEVER touches the daemon (host-side ledger
                // read only, regardless of attach state) — spawn the blocking sqlite work
                // off this thread; the result is drained + pushed below at (b-quin).
                // `scope` AND `session` both ride along so the reply can echo them.
                Ok(super::HostCtl::UsagePreview { session, scope }) => {
                    let tx = usage_preview_tx.clone();
                    std::thread::spawn(move || {
                        let result = super::diff::compute_usage_preview(session.as_deref());
                        let _ = tx.send((result, scope, session));
                    });
                }
                // ANALYTICS DASHBOARD fetch: NEVER touches the daemon (host-side
                // ledger read only, regardless of attach state) — spawn the
                // blocking sqlite work off this thread; the result is drained +
                // pushed below. All correlation inputs ride inside the result.
                Ok(super::HostCtl::Analytics {
                    req_seq,
                    session,
                    scope,
                    range,
                    metric,
                }) => {
                    let tx = analytics_tx.clone();
                    std::thread::spawn(move || {
                        let result =
                            super::diff::compute_analytics(req_seq, scope, session, range, metric);
                        let _ = tx.send(result);
                    });
                }
                Ok(super::HostCtl::KeyList) => {
                    git_host::spawn_key_list_attached(key_list_tx.clone());
                }
                Ok(super::HostCtl::KeyGenerate { name, comment }) => {
                    git_host::spawn_key_generate_attached(
                        key_op_tx.clone(),
                        key_list_tx.clone(),
                        name,
                        comment,
                    );
                }
                Ok(super::HostCtl::KeyImport { name, private_key }) => {
                    git_host::spawn_key_import_attached(
                        key_op_tx.clone(),
                        key_list_tx.clone(),
                        name,
                        private_key,
                    );
                }
                Ok(super::HostCtl::KeyDelete { name }) => {
                    git_host::spawn_key_delete_attached(
                        key_op_tx.clone(),
                        key_list_tx.clone(),
                        name,
                    );
                }
                Ok(super::HostCtl::KeyReveal { name, private }) => {
                    git_host::spawn_key_reveal_attached(key_reveal_tx.clone(), name, private);
                }
                Ok(super::HostCtl::LspStatus) => {
                    super::lsp_host::spawn_lsp_status_attached(lsp_tx.clone());
                }
                Ok(super::HostCtl::LspInstall { id, all, force }) => {
                    super::lsp_host::spawn_lsp_install_attached(lsp_tx.clone(), id, all, force);
                }
                Ok(super::HostCtl::LspUninstall { id }) => {
                    super::lsp_host::spawn_lsp_uninstall_attached(lsp_tx.clone(), id);
                }


                Ok(super::HostCtl::LspDidOpen { root, path, language_id, text }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspDidOpen { root, path, language_id, text },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspDidChange { root, path, text }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspDidChange { root, path, text },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspDidSave { root, path, text }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspDidSave { root, path, text },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspDidClose { root, path }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspDidClose { root, path },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspCompletion { root, path, line, character, request_id }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspCompletion { root, path, line, character, request_id },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspHover { root, path, line, character, request_id }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspHover { root, path, line, character, request_id },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspDefinition { root, path, line, character, request_id }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspDefinition { root, path, line, character, request_id },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspReferences {
                    root,
                    path,
                    line,
                    character,
                    include_declaration,
                    request_id,
                }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspReferences {
                            root,
                            path,
                            line,
                            character,
                            include_declaration,
                            request_id,
                        },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }

                Ok(super::HostCtl::LspDocumentSymbol { root, path, request_id }) => {

                    super::lsp_host::handle_client_ctl(

                        super::HostCtl::LspDocumentSymbol { root, path, request_id },

                        std::sync::Arc::clone(lsp_manager),

                    );

                }
                // Extension STORE browse/detail/installed-list: NEVER touches the
                // daemon (host-side only, regardless of attach state) — spawn the
                // blocking network/config work off this thread via the shared
                // `store_host` bodies (also used by the detached `host_swapper`
                // twin); results are drained + pushed below.
                Ok(super::HostCtl::StoreBrowse { query, category }) => {
                    store_host::spawn_store_browse_attached(
                        store_catalogue_tx.clone(),
                        query,
                        category,
                    );
                }
                Ok(super::HostCtl::TutorialChat { id, messages }) => {
                    tutorial_host::spawn_tutorial_chat_attached(
                        tutorial_chat_tx.clone(),
                        id,
                        messages,
                    );
                }
                Ok(super::HostCtl::StoreDetail { id }) => {
                    store_host::spawn_store_detail_attached(store_detail_tx.clone(), id);
                }
                Ok(super::HostCtl::ListInstalledExtensions) => {
                    store_host::spawn_list_installed_attached(installed_ext_tx.clone());
                }
                Ok(super::HostCtl::GetInstalledExtensionDetail { id }) => {
                    store_host::spawn_get_installed_detail_attached(
                        installed_detail_tx.clone(),
                        id,
                    );
                }
                // Install/uninstall raced in with no daemon attached (in practice this
                // can't happen here — an ATTACHED push_loop always has a live `req_tx`,
                // so `dispatch.rs` always forwards via `ClientRequest` instead of routing
                // through `ctl` — but the arm must exist for the match to stay
                // exhaustive; push a graceful failure rather than silently drop it).
                Ok(super::HostCtl::InstallExtension { id, .. })
                | Ok(super::HostCtl::UninstallExtension { id }) => {
                    push_ext_op_result(push, id, false, Some("no active koma session".to_string()));
                }
                Ok(ctl @ super::HostCtl::FileTree { .. })
                | Ok(ctl @ super::HostCtl::FileRead { .. })
                | Ok(ctl @ super::HostCtl::FileSave { .. })
                | Ok(ctl @ super::HostCtl::FileCreate { .. })
                | Ok(ctl @ super::HostCtl::FileRename { .. })
                | Ok(ctl @ super::HostCtl::FileDelete { .. })
                | Ok(ctl @ super::HostCtl::FileWriteBytes { .. })
                | Ok(ctl @ super::HostCtl::FileDownloadBytes { .. })
                | Ok(ctl @ super::HostCtl::FileContentSearch { .. })
                | Ok(ctl @ super::HostCtl::FileContentReplace { .. }) => {
                    if let Some(fs) = remote_fs {
                        fs.handle_file_ctl(&ctl, push);
                    } else if remote_ctx.is_some() {
                        // Remote attach but remote-fs child failed to start — never
                        // fall back to the laptop filesystem (paths are remote).
                        push_remote_fs_unavailable(&ctl, push);
                    } else {
                        let workdirs = current_owned
                            .as_deref()
                            .and_then(super::diff::session_workdirs_for)
                            .unwrap_or_default();
                        match &ctl {
                            super::HostCtl::FileContentSearch { .. }
                            | super::HostCtl::FileContentReplace { .. } => {
                                super::content_search::handle_content_ctl(
                                    &ctl,
                                    push,
                                    &workdirs,
                                    current_owned.as_deref(),
                                );
                            }
                            _ => {
                                super::file_ops::handle_file_ctl(
                                    &ctl,
                                    push,
                                    &workdirs,
                                    current_owned.as_deref(),
                                );
                            }
                        }
                    }
                }
                #[cfg(feature = "linker")]
                Ok(ctl @ super::HostCtl::ImportGraph { .. })
                | Ok(ctl @ super::HostCtl::ImportGraphImpact { .. })
                | Ok(ctl @ super::HostCtl::ImportGraphReindex { .. }) => {
                    if let Some(linker) = remote_linker {
                        linker.handle_import_ctl(&ctl, push, current_owned.as_deref());
                    } else if remote_ctx.is_some() {
                        // Remote attach but remote-linker child failed to start —
                        // never fall back to the laptop linker daemon.
                        super::remote_linker_client::push_remote_linker_unavailable(&ctl, push);
                    } else {
                        match ctl {
                            super::HostCtl::ImportGraph {
                                path,
                                depth,
                                direction,
                                filter_roots,
                                filter_languages,
                                session_id,
                                request_id,
                            } => {
                                let wds = current_owned
                                    .as_deref()
                                    .and_then(super::diff::session_workdirs_for)
                                    .unwrap_or_default();
                                let configured_roots =
                                    crate::linker::client::canonical_roots(&wds);
                                let configured_root_map =
                                    crate::linker::client::configured_root_map(&wds);
                                let resolved_session =
                                    session_id.or_else(|| current_owned.clone());
                                super::import_graph::spawn_import_graph_attached(
                                    import_graph_tx.clone(),
                                    super::import_graph::ImportGraphJob {
                                        path,
                                        depth,
                                        direction,
                                        filter_roots,
                                        filter_languages,
                                        configured_roots,
                                        configured_root_map,
                                        session_id: resolved_session,
                                        request_id,
                                    },
                                );
                            }
                            super::HostCtl::ImportGraphImpact {
                                path,
                                depth,
                                request_id,
                                session_id,
                            } => {
                                let configured_roots = current_owned
                                    .as_deref()
                                    .and_then(super::diff::session_workdirs_for)
                                    .map(|wds| crate::linker::client::canonical_roots(&wds))
                                    .unwrap_or_default();
                                let resolved_session =
                                    session_id.or_else(|| current_owned.clone());
                                super::import_graph::spawn_import_graph_impact_attached(
                                    impact_tx.clone(),
                                    path,
                                    depth,
                                    request_id,
                                    configured_roots,
                                    resolved_session,
                                );
                            }
                            super::HostCtl::ImportGraphReindex { request_id } => {
                                let session_id =
                                    current_owned.as_deref().unwrap_or_default().to_string();
                                let wds = current_owned
                                    .as_deref()
                                    .and_then(super::diff::session_workdirs_for)
                                    .unwrap_or_default();
                                let configured_roots =
                                    crate::linker::client::canonical_roots(&wds);
                                let configured_root_map =
                                    crate::linker::client::configured_root_map(&wds);
                                super::import_graph::spawn_import_graph_reindex_attached(
                                    import_graph_tx.clone(),
                                    session_id,
                                    configured_roots,
                                    configured_root_map,
                                    None,
                                    None,
                                    request_id,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                // ─── Remote host management (host-local, fast file I/O) ────
                Ok(ctl @ super::HostCtl::GetRemoteHosts)
                | Ok(ctl @ super::HostCtl::AddRemoteHost { .. })
                | Ok(ctl @ super::HostCtl::EditRemoteHost { .. })
                | Ok(ctl @ super::HostCtl::DeleteRemoteHost { .. }) => {
                    let mut hosts = crate::remote::hosts::load_hosts();
                    let mutated = match ctl {
                        super::HostCtl::AddRemoteHost {
                            name,
                            user,
                            host,
                            port,
                            key_path,
                        } => {
                            let new_id = crate::model::app_config::new_uuid();
                            crate::remote::hosts::upsert_host(
                                &mut hosts,
                                crate::remote::hosts::RemoteHost {
                                    id: new_id,
                                    name,
                                    user,
                                    host,
                                    port,
                                    key_path,
                                    last_connected: None,
                                    tags: vec![],
                                },
                            );
                            true
                        }
                        super::HostCtl::EditRemoteHost {
                            id,
                            name,
                            user,
                            host,
                            port,
                            key_path,
                        } => {
                            if let Some(h) = crate::remote::hosts::host_by_id(&hosts, &id) {
                                let updated = crate::remote::hosts::RemoteHost {
                                    id: h.id.clone(),
                                    name,
                                    user,
                                    host,
                                    port,
                                    key_path,
                                    last_connected: h.last_connected,
                                    tags: h.tags.clone(),
                                };
                                crate::remote::hosts::upsert_host(&mut hosts, updated);
                                true
                            } else {
                                false
                            }
                        }
                        super::HostCtl::DeleteRemoteHost { id } => {
                            let deleted = crate::remote::hosts::delete_host(&mut hosts, &id);
                            if deleted {
                                let _ = crate::remote::secrets::delete_remote_password(&id);
                            }
                            deleted
                        }
                        _ => false, // GetRemoteHosts — read-only
                    };
                    if mutated {
                        let _ = crate::remote::hosts::save_hosts(&hosts);
                    }
                    let wire_hosts: Vec<serde_json::Value> = hosts
                        .hosts
                        .iter()
                        .map(|h| {
                            serde_json::json!({
                                "id": h.id, "name": h.name, "user": h.user, "host": h.host,
                                "port": h.port, "keyPath": h.key_path,
                                "connected": false,
                                "lastConnected": h.last_connected,
                                "tags": h.tags,
                            })
                        })
                        .collect();
                    let envelope = serde_json::json!({ "k": "RemoteHosts", "hosts": wire_hosts });
                    if let Ok(json) = serde_json::to_string(&envelope) {
                        push(json);
                    }
                }
                // ─── Remote host connect/disconnect ────────────────────────
                Ok(super::HostCtl::ConnectRemote { host_id }) => {
                    // Create a password exchange channel. The worker returns the
                    // ready host context through `remote_ready_tx` (no session yet).
                    let (pw_tx, pw_rx) = std::sync::mpsc::channel::<String>();
                    let (attempt_id, cancelled) = remote_shared.begin(pw_tx);
                    push_remote_state(
                        push,
                        "resolving",
                        Some(&host_id),
                        None,
                        None,
                        None,
                        None,
                        &[],
                    );
                    super::remote_ctl::spawn_connect_worker(
                        attempt_id,
                        host_id,
                        remote_state_tx.clone(),
                        remote_ready_tx.clone(),
                        pw_rx,
                        cancelled,
                        std::sync::Arc::clone(&remote_shared),
                        tokio::runtime::Handle::current(),
                    );
                }
                Ok(super::HostCtl::DisconnectRemote) | Ok(super::HostCtl::CancelRemoteConnect) => {
                    // During connect/auth, dropping the password sender releases the
                    // worker. Once a remote session is live, we must leave the attach
                    // entirely — otherwise the GUI keeps a stale session.id and stays
                    // stuck on ChatView over a dead SSH bridge.
                    remote_shared.cancel();
                    if remote_ctx.is_some() {
                        return HostTransition::DisconnectRemote;
                    }
                    push_remote_state(push, "disconnected", None, None, None, None, None, &[]);
                }
                Ok(super::HostCtl::SubmitRemotePassword { password }) => {
                    remote_shared.submit_password(password);
                }
                // ─── GUI terminal view (host-local PTY lifecycle) ─────
                // Terminal sessions are managed host-side via the shared
                // TerminalManager. On a live remote attach, spawn ssh -t into
                // the remote host (same mux/auth as the agent bridge).
                Ok(super::HostCtl::TerminalCreate { id, cwd }) => {
                    if let Ok(mut mgr) = terminal_manager.lock() {
                        let result = if let Some(ctx) = remote_ctx {
                            // Prefer explicit cwd from the UI; else session workdir
                            // is unknown here — remote $HOME via interactive shell.
                            mgr.create_remote(id, &ctx.target, ctx.password(), cwd.as_deref())
                        } else {
                            mgr.create(id, cwd)
                        };
                        if let Err(e) = result {
                            crate::model::store::append_global_error_log(
                                "terminal",
                                &format!("terminal create failed: {e}"),
                            );
                        }
                    }
                }
                Ok(super::HostCtl::TerminalInput { id, data }) => {
                    if let Ok(mut mgr) = terminal_manager.lock() {
                        mgr.input(&id, &data);
                    }
                }
                Ok(super::HostCtl::TerminalResize { id, cols, rows }) => {
                    if let Ok(mut mgr) = terminal_manager.lock() {
                        mgr.resize(&id, cols, rows);
                    }
                }
                Ok(super::HostCtl::TerminalKill { id }) => {
                    if let Ok(mut mgr) = terminal_manager.lock() {
                        mgr.kill(&id);
                    }
                }
                Err(TryRecvError::Empty) => break,
                // The ipc side hung up (window gone) — leave the host.
                Err(TryRecvError::Disconnected) => return HostTransition::Exit,
            }
        }

        // --- (a) drain every queued incoming frame (NON-BLOCKING) ---
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        let mut connect_remote_requested: Option<ConnectRemoteRequest> = None;
        loop {
            match frame_rx.try_recv() {
                Ok(frame) => {
                    // Every one-shot non-visual `DaemonEvent` reply (FileSearchResults,
                    // ModelList, ModelRoutes, SettingsValues, AgentsValues, EffortOptions,
                    // OAuthState) is re-pushed to JS as its own `PushEnvelope` HERE, BEFORE
                    // folding — see `push_intercept` (split out for file size; pure code
                    // motion, no behaviour change).
                    push_intercept::repush_before_fold(&frame, push);
                    // Keep remote-fs sandbox roots in sync with the remote session's
                    // workdirs (from SettingsValues or Snapshot). Local session_workdirs_for
                    // must not be used when remote_fs is set.
                    if let Some(fs) = remote_fs {
                        if let DaemonEvent::SettingsValues { workdir, .. } = &frame.event {
                            let _ = fs.set_roots(workdir.clone());
                        }
                    }
                    // Same for remote-linker: workdirs are absolute remote paths.
                    #[cfg(feature = "linker")]
                    if let Some(linker) = remote_linker {
                        if let DaemonEvent::SettingsValues { workdir, .. } = &frame.event {
                            let _ = linker.set_roots(workdir.clone());
                        }
                    }
                    // Cache the authoritative config off every full snapshot so the
                    // `Config` envelope can be (re)emitted below (a config edit forces a
                    // full snapshot — see `ipc::snapshot::diff`).
                    if let DaemonEvent::Snapshot(snap) = &frame.event {
                        let proj = ConfigProjection::from_global(&snap.global);
                        current_config = Some(proj);
                    }
                    apply_frame(
                        frame,
                        &mut shadow,
                        &mut expected,
                        &mut seeded,
                        &mut awaiting_resync,
                        &mut select_requested,
                        &mut open_swapper_requested,
                        &mut new_session_requested,
                        &mut connect_remote_requested,
                        req_tx,
                    );
                }
                Err(TryRecvError::Empty) => break,
                // The reader task dropped its sender: the daemon's socket closed. Fall
                // back to the hub so the user can pick another session.
                Err(TryRecvError::Disconnected) => {
                    return if let Some(ctx) = remote_ctx {
                        HostTransition::ToRemoteHub {
                            ctx: Box::new(ctx.clone()),
                        }
                    } else {
                        HostTransition::ToSwapper
                    };
                }
            }
        }

        // `/resume` hand-off from the daemon: detach + show the hub.
        if open_swapper_requested {
            return if let Some(ctx) = remote_ctx {
                HostTransition::ToRemoteHub {
                    ctx: Box::new(ctx.clone()),
                }
            } else {
                HostTransition::ToSwapper
            };
        }
        // `/new` hand-off from the daemon. Local: mint id + Attach. Remote: return to
        // the remote hub and open the path picker (never local Attach for a remote id).
        if new_session_requested.is_some() {
            if let Some(ctx) = remote_ctx {
                let _ = ctl_tx.send(super::HostCtl::RequestRemotePath);
                return HostTransition::ToRemoteHub {
                    ctx: Box::new(ctx.clone()),
                };
            }
            let new_id = uuid::Uuid::new_v4().to_string();
            // Same swap-START loader signal as a hub `New` — this is a daemon-driven attach
            // gap, equally frozen until the new session's first Snapshot.
            push_switching(push, &new_id);
            // Daemon-driven hand-off carries no picked folder — inherit the host cwd.
            return HostTransition::Attach {
                id: new_id,
                workdir: None,
            };
        }
        // `/select` transcript dump needs a terminal the host does not own — ignore it.

        // --- (b) advance LOCAL-clock animations + sweep the toast ---
        advance_local_animations(&mut shadow);
        {
            let fg = shadow.rest.fg_mut();
            if let Some((_, until, _)) = fg.toast.as_ref() {
                if Instant::now() >= *until {
                    fg.toast = None;
                }
            }
        }

        // --- (b-bis) attached-state hub refresh: push any completed off-thread sweep ---
        // Drain the worker channel to the NEWEST built hub (non-blocking), clear the
        // in-flight latch, and push it. `push_hub` diffs `last.hub_json`, so an
        // unchanged live set emits nothing. This is what keeps the React ResumePalette's
        // cooking/history current while ATTACHED, not frozen at the cold boot build.
        {
            let mut latest_hub: Option<SessionHub> = None;
            while let Ok(hub) = hub_rx.try_recv() {
                latest_hub = Some(hub);
                refresh_inflight = false;
            }
            if let Some(hub) = latest_hub {
                push_hub(&hub, push, last);
            }
        }

        // --- (b-quat) FILE CHANGED diff fetch: push any completed off-thread diffs ---
        // Drain ALL completed results (not just the newest — see the channel's doc
        // comment above) and push each as its own one-shot `FileDiff` envelope.
        while let Ok(result) = file_diff_rx.try_recv() {
            push_file_diff(push, result);
        }

        // --- (b-quin) USAGE PANEL: push any completed off-thread preview fetch ---
        while let Ok((result, scope, session_id)) = usage_preview_rx.try_recv() {
            push_usage_preview(push, result, scope, session_id);
        }

        // --- Analytics dashboard: push any completed off-thread fetch ---
        while let Ok(result) = analytics_rx.try_recv() {
            push_analytics(push, result);
        }

        // --- (b-sex)..(b-duodec) GIT / SSH-key-vault panels: push any completed
        // off-thread status/diff/op/branch-list/graph/detail/commit-diff/key-list/
        // key-reveal/key-op fetches, in the SAME order as before — split out into
        // `git_drain::drain_git_replies` for file size (pure code motion, no
        // behaviour change).
        drain_git_replies(
            push,
            &git_status_rx,
            &git_diff_rx,
            &git_op_rx,
            &branch_list_rx,
            &repo_list_rx,
            &git_graph_rx,
            &commit_detail_rx,
            &commit_diff_rx,
            &key_list_rx,
            &key_reveal_rx,
            &key_op_rx,
            &stash_list_rx,
            &activity_rx,
        );

        // --- Language servers: push any completed status/install frames ---
        super::lsp_host::drain_lsp_replies(&lsp_rx, push);

        // --- Extension STORE: push any completed off-thread browse/detail/installed-
        // list fetches. No coalescing needed — each is a self-contained, per-request
        // reply, like the GIT channels above.
        while let Ok((items, error)) = store_catalogue_rx.try_recv() {
            push_store_catalogue(push, items, error);
        }
        while let Ok((detail, error)) = store_detail_rx.try_recv() {
            push_store_detail(push, detail, error);
        }
        while let Ok(items) = installed_ext_rx.try_recv() {
            push_installed_extensions(push, items);
        }
        while let Ok((id, detail, error)) = installed_detail_rx.try_recv() {
            super::push_proto::push_installed_ext_detail(push, id, detail, error);
        }

        while let Ok(result) = tutorial_chat_rx.try_recv() {
            push_tutorial_chat_done(push, result.id, result.text, result.tour, result.error);
        }

        // --- REMOTE HOST CONNECT: push state transitions, then hand off to remote hub ---
        while let Ok(update) = remote_state_rx.try_recv() {
            if !remote_shared.is_current(update.attempt_id) {
                continue;
            }
            push_remote_state(
                push,
                &update.state,
                update.host_id.as_deref(),
                update.user.as_deref(),
                update.host.as_deref(),
                update.session_id.as_deref(),
                update.error.as_deref(),
                &update.sessions,
            );
        }
        while let Ok(ready) = remote_ready_rx.try_recv() {
            if !remote_shared.is_current(ready.attempt_id) {
                continue;
            }
            push_remote_state(
                push,
                "ready",
                Some(&ready.ctx.host_id),
                Some(&ready.ctx.target.user),
                Some(&ready.ctx.target.host),
                None,
                None,
                &ready.sessions,
            );
            // Leave the current (local) session and enter the remote hub.
            return HostTransition::ToRemoteHub {
                ctx: Box::new(ready.ctx),
            };
        }

        // --- IMPORT GRAPH: push any completed off-thread linker daemon visualization ---
        #[cfg(feature = "linker")]
        while let Ok(result) = import_graph_rx.try_recv() {
            super::render::emit(push, &super::push_proto::PushEnvelope::ImportGraph(result));
        }

        // --- IMPORT GRAPH IMPACT: push any completed off-thread impact analysis ---
        #[cfg(feature = "linker")]
        while let Ok(result) = impact_rx.try_recv() {
            super::render::emit(
                push,
                &super::push_proto::PushEnvelope::ImportGraphImpact(result),
            );
        }

        // --- (b-ter) mirror the staged-attachment markers for the ipc Submit append ---
        // The ipc thread appends these `[Image #N]` markers to a chat send so the daemon's
        // submit-time reconcile keeps the staged images (React's text carries no markers).
        if let Ok(mut marks) = live_marks.lock() {
            marks.clear();
            marks.extend(
                shadow
                    .rest
                    .fg()
                    .pending_attachments
                    .iter()
                    .map(|a| a.marker_n),
            );
        }

        // --- (c) serialise + push whatever changed (the draw seam) ---
        // Snapshot the current stream view (Copy) out of the shared lock so the fold folds
        // the viewed sub-agent's transcript / viewed bash job's output tail into the push.
        let view = live_view.lock().map(|v| *v).unwrap_or_default();
        serialize_and_push(&shadow, push, last, view);
        // Config catalogue (Connector + MCP panels): emit whenever it changed since the
        // last frame, or re-emit after a `Ready` reset. Independent of the per-session
        // draw so a page reload always re-pushes the current global config.
        push_config(current_config.as_ref(), push, last);

        // --- frame pacing: sleep the remainder of the ~16ms budget ---
        if let Some(rem) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }
}

/// Push a Git* error envelope when remote attach has no live remote-git child.
fn push_remote_git_unavailable(ctl: &super::HostCtl, push: &dyn Fn(String)) {
    use super::push_proto_git::{
        push_activity, push_branch_list, push_commit_detail, push_commit_diff, push_git_diff,
        push_git_graph, push_git_op, push_git_status, push_repo_list, push_stash_list,
    };
    const ERR: &str = "remote-git unavailable";
    match ctl {
        super::HostCtl::GitDiff { path, staged } => {
            push_git_diff(
                push,
                super::git::GitDiffResult {
                    path: path.clone(),
                    staged: *staged,
                    original: String::new(),
                    modified: String::new(),
                    error: Some(ERR.into()),
                    binary: false,
                },
            );
        }
        super::HostCtl::GitStage { .. }
        | super::HostCtl::GitUnstage { .. }
        | super::HostCtl::GitDiscard { .. }
        | super::HostCtl::GitCommit { .. }
        | super::HostCtl::GitFetch
        | super::HostCtl::GitPull
        | super::HostCtl::GitPush { .. }
        | super::HostCtl::GitStash
        | super::HostCtl::GitStashPop
        | super::HostCtl::GitCheckout { .. }
        | super::HostCtl::GitCreateBranch { .. }
        | super::HostCtl::GitCherryPick { .. }
        | super::HostCtl::GitRevert { .. }
        | super::HostCtl::GitReset { .. }
        | super::HostCtl::GitMerge { .. }
        | super::HostCtl::GitRebase { .. }
        | super::HostCtl::GitOpAbort { .. }
        | super::HostCtl::GitOpContinue { .. } => {
            push_git_op(
                push,
                super::git::GitOpResult {
                    ok: false,
                    op: "remote".into(),
                    error: Some(ERR.into()),
                    message: None,
                },
            );
        }
        super::HostCtl::GitGraph { .. } => {
            push_git_graph(
                push,
                super::git_graph::GitGraphResult {
                    commits: Vec::new(),
                    head: None,
                    has_more: false,
                    error: Some(ERR.into()),
                },
            );
        }
        super::HostCtl::GitCommitDetail { sha } => {
            push_commit_detail(
                push,
                super::git_graph::CommitDetailResult {
                    sha: sha.clone(),
                    author: String::new(),
                    email: String::new(),
                    date: String::new(),
                    subject: String::new(),
                    body: String::new(),
                    parents: Vec::new(),
                    files: Vec::new(),
                    error: Some(ERR.into()),
                },
            );
        }
        super::HostCtl::GitCommitDiff { sha, path } => {
            push_commit_diff(
                push,
                super::git_graph::CommitDiffResult {
                    sha: sha.clone(),
                    path: path.clone(),
                    original: String::new(),
                    modified: String::new(),
                    error: Some(ERR.into()),
                    binary: false,
                },
            );
        }
        super::HostCtl::GitBranchList { request_id } => {
            push_branch_list(
                push,
                super::git_branch::BranchListResult {
                    branches: Vec::new(),
                    error: Some(ERR.into()),
                    root: None,
                    request_id: *request_id,
                },
            );
        }
        super::HostCtl::GitRepos => {
            push_repo_list(
                push,
                super::git_repos::RepoListResult {
                    repos: Vec::new(),
                    active: None,
                },
            );
        }
        super::HostCtl::GitStashList => {
            push_stash_list(
                push,
                super::git_stash::StashListResult {
                    entries: Vec::new(),
                    error: Some(ERR.into()),
                },
            );
        }
        super::HostCtl::GitActivity { path, .. } => {
            push_activity(
                push,
                super::git_activity::ActivityResult {
                    commits: Vec::new(),
                    path: path.clone(),
                    error: Some(ERR.into()),
                },
            );
        }
        // GitStatus, SetGitKey, SetActiveRepo — status-shaped reply.
        _ => {
            push_git_status(
                push,
                super::git::GitStatusResult {
                    root: None,
                    branch: None,
                    detached: false,
                    ahead: None,
                    behind: None,
                    staged: Vec::new(),
                    unstaged: Vec::new(),
                    error: Some(ERR.into()),
                    key_name: None,
                    in_progress: None,
                    conflicted: Vec::new(),
                    push_mode: None,
                },
            );
        }
    }
}

/// Push a File* error envelope when remote attach has no live remote-fs child.
fn push_remote_fs_unavailable(ctl: &super::HostCtl, push: &dyn Fn(String)) {
    use super::push_proto::PushEnvelope;
    const ERR: &str = "remote-fs unavailable";
    let env = match ctl {
        super::HostCtl::FileTree {
            root,
            path,
            request_id,
        } => PushEnvelope::FileTree {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            entries: Vec::new(),
            error: Some(ERR.into()),
        },
        super::HostCtl::FileRead {
            root,
            path,
            request_id,
        } => PushEnvelope::FileRead {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            content: None,
            fingerprint: String::new(),
            binary: false,
            too_large: false,
            error: Some(ERR.into()),
        },
        super::HostCtl::FileSave {
            root,
            path,
            request_id,
            ..
        } => PushEnvelope::FileSave {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            fingerprint: String::new(),
            error: Some(ERR.into()),
        },
        super::HostCtl::FileCreate {
            root,
            path,
            request_id,
            ..
        } => PushEnvelope::FileCreate {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            error: Some(ERR.into()),
        },
        super::HostCtl::FileRename {
            root,
            old_path,
            new_path,
            request_id,
        } => PushEnvelope::FileRename {
            root: root.clone(),
            old_path: old_path.clone(),
            new_path: new_path.clone(),
            request_id: request_id.clone(),
            error: Some(ERR.into()),
        },
        super::HostCtl::FileDelete {
            root,
            path,
            request_id,
        } => PushEnvelope::FileDelete {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            error: Some(ERR.into()),
        },
        super::HostCtl::FileWriteBytes {
            root,
            path,
            request_id,
            ..
        } => PushEnvelope::FileWriteBytes {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            error: Some(ERR.into()),
        },
        super::HostCtl::FileDownloadBytes {
            root,
            path,
            request_id,
        } => PushEnvelope::FileDownloadBytes {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            bytes_b64: None,
            size: 0,
            too_large: false,
            error: Some(ERR.into()),
        },
        super::HostCtl::FileContentSearch {
            root,
            path,
            request_id,
            ..
        } => PushEnvelope::FileContentSearch {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            results: Vec::new(),
            error: Some(ERR.into()),
            truncated: false,
        },
        super::HostCtl::FileContentReplace {
            root,
            path,
            request_id,
            ..
        } => PushEnvelope::FileContentReplace {
            root: root.clone(),
            path: path.clone(),
            request_id: request_id.clone(),
            files_changed: 0,
            match_count: 0,
            error: Some(ERR.into()),
            truncated: false,
        },
        _ => return,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        push(json);
    }
}

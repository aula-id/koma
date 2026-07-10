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

use super::project::{push_hub, serialize_and_push};
use super::project_config::{push_config, ConfigProjection};
use super::push_proto::{
    push_file_diff, push_git_diff, push_git_op, push_git_status, push_switching,
    push_usage_preview, PushEnvelope, PushRoute,
};
use super::render::{advance_local_animations, FRAME_BUDGET};
use super::shadow::apply_frame;

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
    pub(super) status: Option<(
        bool,
        Option<String>,
        Option<&'static str>,
        u64,
        u64,
        u64,
        f64,
        String,
    )>,
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
    /// Attach to this session UUID (a hub `SelectSession`/`NewSession`, or a daemon
    /// `NewSession` hand-off). A minted uuid for a new session; an existing id otherwise.
    /// `workdir` is the folder a GUI `[+ new session]` native picker chose (the new
    /// session's working dir); `None` for every other attach inherits the host's cwd.
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

    // --- GIT STATUS fetch (GitStatus) ---
    // `compute_git_status` shells out to `git status`, blocking — same reasoning as
    // `FileDiff` above — so it runs on a one-shot worker thread; the loop drains
    // completed results non-blocking and pushes each as a `GitStatus` envelope. No
    // "latest wins" coalescing needed (a `GitStatus` fetch is rare enough, and each
    // reply is self-contained), mirroring `FileDiff`'s per-request-pushed rule.
    let (git_status_tx, git_status_rx) = std::sync::mpsc::channel::<super::git::GitStatusResult>();

    // --- GIT DIFF fetch (GitDiff) ---
    // `compute_git_diff` shells out to `git show` (+ a disk read), blocking — same
    // reasoning as `FileDiff` above — so it runs on a one-shot worker thread; the loop
    // drains completed results non-blocking and pushes each as a `GitDiff` envelope.
    let (git_diff_tx, git_diff_rx) = std::sync::mpsc::channel::<super::git::GitDiffResult>();

    // --- GIT OP mutation (GitStage/GitUnstage/GitDiscard/GitCommit) ---
    // Each mutation shells out to git (blocking), same reasoning as `GitStatus`/
    // `GitDiff` above — so it runs on a one-shot worker thread; the loop drains
    // completed results non-blocking and pushes each as a `GitOp` envelope. The SAME
    // worker thread ALSO recomputes the status right after the mutation and sends it
    // over `git_status_tx` (reusing the channel above), so the panel's lists refresh
    // from authoritative state immediately after every op — no separate coalescing
    // needed for that follow-up.
    let (git_op_tx, git_op_rx) = std::sync::mpsc::channel::<super::git::GitOpResult>();

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

    // Fold the handshake's prebuffered frames first, through the SAME `apply_frame`
    // path (seq seeding stays gap-free). The select/swapper/new latches can't fire
    // this early, so the throwaways here are never acted on.
    {
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        for frame in prebuffered {
            // Cache the config off any prebuffered full snapshot (normally none — Hello
            // is first, so the attach Snapshot lands in the live drain — but stay safe).
            if let DaemonEvent::Snapshot(snap) = &frame.event {
                current_config = Some(ConfigProjection::from_global(&snap.global));
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
                Ok(super::HostCtl::Select(id)) => {
                    push_switching(push, &id);
                    return HostTransition::Attach { id, workdir: None };
                }
                // `[+ new session]` while attached: the GUI picker already confirmed a
                // folder (a cancel sends no `New`), so carry it into the fresh session. On
                // `kill` reap the CURRENT daemon as part of the switch — queue a graceful
                // QuitDaemon on the live conn (flushed by the upcoming teardown, mirroring the
                // TUI `/new kill`) and ensure its death OFF-thread so the fresh attach never
                // waits on the old daemon's corpse. `kill: false` leaves the old daemon
                // cooking (resumable), exactly as before.
                Ok(super::HostCtl::New { workdir, kill }) => {
                    if kill {
                        if let Some(old) = current_owned.clone() {
                            let _ = req_tx.send(ClientRequest::QuitDaemon);
                            super::host::spawn_ensure_dead(old);
                        }
                    }
                    let new_id = uuid::Uuid::new_v4().to_string();
                    push_switching(push, &new_id);
                    return HostTransition::Attach { id: new_id, workdir };
                }
                // KILL the daemon `id`. Killing the CURRENTLY-ATTACHED session: queue a
                // graceful QuitDaemon on the live conn (flushed by teardown), ensure its death
                // OFF-thread — a harmless double-QuitDaemon that ALSO fires a follow-up
                // RefreshHub so the swapper we're about to land in drops the row the instant
                // it is gone (its entry push may briefly show it for <1s) — then hand back to
                // the swapper (the same path `ToSwapper` takes). A BACKGROUND kill just
                // escalates OFF-thread and refreshes the hub once the daemon is confirmed dead
                // (the off-thread sweep drained at (b-bis) pushes the rebuilt hub).
                Ok(super::HostCtl::KillSession(id)) => {
                    if current_owned.as_deref() == Some(id.as_str()) {
                        let _ = req_tx.send(ClientRequest::QuitDaemon);
                        super::host::spawn_kill_and_refresh(ctl_tx.clone(), id);
                        return HostTransition::ToSwapper;
                    }
                    super::host::spawn_kill_and_refresh(ctl_tx.clone(), id);
                }
                // Physically DELETE a history session OFF-thread (guarded host-side against
                // deleting a live/locked session), then RefreshHub. A history row is never the
                // attached session, so there is no live-conn interaction here.
                Ok(super::HostCtl::DeleteSession(id)) => {
                    super::host::spawn_delete_and_refresh(ctl_tx.clone(), id);
                }
                // Cancel-switch (best-effort): the swap in flight can't be interrupted, so
                // this simply drops to the hub AFTER the current/queued attach resolves —
                // `host_swapper` then pushes a fresh `Hub`, and the loader clears on it.
                Ok(super::HostCtl::ToSwapper) => return HostTransition::ToSwapper,
                // The ResumePalette opened: kick a hub refresh OFF this thread (the
                // discovery sweep blocks). Coalesced by `refresh_inflight` so a burst of
                // RefreshHubs while the palette stays open runs at most one sweep; the
                // result is drained + pushed below.
                Ok(super::HostCtl::RefreshHub) => {
                    if !refresh_inflight {
                        refresh_inflight = true;
                        let tx = hub_tx.clone();
                        let cur = current_owned.clone();
                        std::thread::spawn(move || {
                            let hub = super::build_local_hub(cur.as_deref());
                            let _ = tx.send(hub);
                        });
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
                // Explore GIT panel: NEVER touches the daemon (host-side only,
                // regardless of attach state) — spawn the blocking git work off this
                // thread; the result is drained + pushed below at (b-sex).
                Ok(super::HostCtl::GitStatus) => {
                    let tx = git_status_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let result = super::git::compute_git_status(cur.as_deref());
                        let _ = tx.send(result);
                    });
                }
                // GIT panel file-row click: same reasoning as `GitStatus` above; the
                // result is drained + pushed below at (b-sept).
                Ok(super::HostCtl::GitDiff { path, staged }) => {
                    let tx = git_diff_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let result = super::git::compute_git_diff(&path, staged, cur.as_deref());
                        let _ = tx.send(result);
                    });
                }
                // GIT panel mutations: NEVER touch the daemon (host-side only,
                // regardless of attach state) — spawn the blocking git work off this
                // thread. The worker sends the `GitOp` result over `git_op_tx` (drained
                // + pushed below at (b-oct)), THEN recomputes + sends the refreshed
                // status over the EXISTING `git_status_tx` (drained + pushed at
                // (b-sex)) so the panel's lists reflect authoritative state right after
                // the mutation.
                Ok(super::HostCtl::GitStage { paths }) => {
                    let op_tx = git_op_tx.clone();
                    let status_tx = git_status_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let _ = op_tx.send(super::git::git_stage(&paths, cur.as_deref()));
                        let _ = status_tx.send(super::git::compute_git_status(cur.as_deref()));
                    });
                }
                Ok(super::HostCtl::GitUnstage { paths }) => {
                    let op_tx = git_op_tx.clone();
                    let status_tx = git_status_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let _ = op_tx.send(super::git::git_unstage(&paths, cur.as_deref()));
                        let _ = status_tx.send(super::git::compute_git_status(cur.as_deref()));
                    });
                }
                Ok(super::HostCtl::GitDiscard { paths }) => {
                    let op_tx = git_op_tx.clone();
                    let status_tx = git_status_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let _ = op_tx.send(super::git::git_discard(&paths, cur.as_deref()));
                        let _ = status_tx.send(super::git::compute_git_status(cur.as_deref()));
                    });
                }
                Ok(super::HostCtl::GitCommit { message }) => {
                    let op_tx = git_op_tx.clone();
                    let status_tx = git_status_tx.clone();
                    let cur = current_owned.clone();
                    std::thread::spawn(move || {
                        let _ = op_tx.send(super::git::git_commit(&message, cur.as_deref()));
                        let _ = status_tx.send(super::git::compute_git_status(cur.as_deref()));
                    });
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
                Err(TryRecvError::Empty) => break,
                // The ipc side hung up (window gone) — leave the host.
                Err(TryRecvError::Disconnected) => return HostTransition::Exit,
            }
        }

        // --- (a) drain every queued incoming frame (NON-BLOCKING) ---
        let mut select_requested = false;
        let mut open_swapper_requested = false;
        let mut new_session_requested: Option<bool> = None;
        loop {
            match frame_rx.try_recv() {
                Ok(frame) => {
                    // Omnisearch reply: intercept the one-shot `FileSearchResults` and
                    // re-push it to JS as a `SearchResults` envelope BEFORE folding (the
                    // fold treats it as a non-visual no-op, keeping the seq gap-free).
                    if let DaemonEvent::FileSearchResults { query, items } = &frame.event {
                        let env = PushEnvelope::SearchResults {
                            query: query.clone(),
                            items: items.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Cache the authoritative config off every full snapshot so the
                    // `Config` envelope can be (re)emitted below (a config edit forces a
                    // full snapshot — see `ipc::snapshot::diff`).
                    if let DaemonEvent::Snapshot(snap) = &frame.event {
                        current_config = Some(ConfigProjection::from_global(&snap.global));
                    }
                    // Live model-id catalogue reply (Connector model picker): re-push it as
                    // a `ModelList` envelope BEFORE folding (the fold treats it as a
                    // non-visual no-op, keeping the seq gap-free).
                    if let DaemonEvent::ModelList { provider, models } = &frame.event {
                        let env = PushEnvelope::ModelList {
                            provider: provider.clone(),
                            models: models.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Live provider-route reply (Connector ModelForm route picker): re-push
                    // it as a `RouteList` envelope BEFORE folding (a non-visual fold no-op),
                    // flattening each wire route to the camelCase `PushRoute` JS contract.
                    if let DaemonEvent::ModelRoutes { provider, model_id, routes } = &frame.event {
                        let env = PushEnvelope::RouteList {
                            provider: provider.clone(),
                            model_id: model_id.clone(),
                            routes: routes
                                .iter()
                                .map(|r| PushRoute {
                                    name: r.name.clone(),
                                    provider_name: r.provider_name.clone(),
                                    price_prompt: r.price_prompt.clone(),
                                    price_completion: r.price_completion.clone(),
                                    uptime_last_30m: r.uptime_last_30m,
                                })
                                .collect(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // GUI Settings-tab reply (GetSettings / post-SetSessionPrefs re-push):
                    // re-push it as a `SettingsValues` envelope BEFORE folding (a non-visual
                    // fold no-op, keeping the seq gap-free), same as the ModelList/RouteList
                    // intercepts above.
                    if let DaemonEvent::SettingsValues {
                        name,
                        workdir,
                        short_send,
                        sliding_cache,
                        bash_saving,
                        internet_mode,
                        palette,
                        effort,
                    } = &frame.event
                    {
                        let env = PushEnvelope::SettingsValues {
                            name: name.clone(),
                            workdir: workdir.clone(),
                            short_send: *short_send,
                            sliding_cache: *sliding_cache,
                            bash_saving: *bash_saving,
                            internet_mode: internet_mode.clone(),
                            palette: palette.clone(),
                            effort: effort.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // GUI /agents-dashboard reply (GetAgents / post-SetAgent / -DeleteAgent
                    // re-push): re-push it as an `AgentsValues` envelope BEFORE folding (a
                    // non-visual fold no-op, keeping the seq gap-free), same as the
                    // SettingsValues intercept above.
                    if let DaemonEvent::AgentsValues {
                        agents,
                        catalogue_models,
                        catalogue_providers,
                        available_tools,
                    } = &frame.event
                    {
                        let env = PushEnvelope::AgentsValues {
                            agents: agents.clone(),
                            catalogue_models: catalogue_models.clone(),
                            catalogue_providers: catalogue_providers.clone(),
                            available_tools: available_tools.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Composer EFFORT-picker reply (GetEffortOptions): re-push it as an
                    // `EffortOptions` envelope BEFORE folding (a non-visual fold no-op,
                    // keeping the seq gap-free), same as the SettingsValues intercept above.
                    if let DaemonEvent::EffortOptions {
                        options,
                        selected,
                        note,
                        state,
                    } = &frame.event
                    {
                        let env = PushEnvelope::EffortOptions {
                            options: options.clone(),
                            selected: *selected,
                            note: note.clone(),
                            state: state.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
                    }
                    // Streaming GUI OAuth reply (GetOAuthState / StartOAuth progress /
                    // SubmitOAuthPaste / CancelOAuth / DeleteOAuthConn): re-push it as an
                    // `OAuthState` envelope BEFORE folding (a non-visual fold no-op, keeping
                    // the seq gap-free), same as the SettingsValues/AgentsValues intercepts.
                    if let DaemonEvent::OAuthState {
                        phase,
                        url,
                        user_code,
                        verification_url,
                        error,
                        conns,
                        providers,
                    } = &frame.event
                    {
                        let env = PushEnvelope::OAuthState {
                            phase: phase.clone(),
                            url: url.clone(),
                            user_code: user_code.clone(),
                            verification_url: verification_url.clone(),
                            error: error.clone(),
                            conns: conns.clone(),
                            providers: providers.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&env) {
                            push(json);
                        }
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
                        req_tx,
                    );
                }
                Err(TryRecvError::Empty) => break,
                // The reader task dropped its sender: the daemon's socket closed. Fall
                // back to the swapper so the user can pick another session.
                Err(TryRecvError::Disconnected) => return HostTransition::ToSwapper,
            }
        }

        // `/resume` hand-off from the daemon: detach + show the swapper.
        if open_swapper_requested {
            return HostTransition::ToSwapper;
        }
        // `/new` hand-off from the daemon: attach a freshly minted session. (The `kill`
        // flag is a daemon-side reap the headless host does not drive in W0; a plain
        // detach-then-attach is fine — the old daemon keeps cooking, resumable.)
        if new_session_requested.is_some() {
            let new_id = uuid::Uuid::new_v4().to_string();
            // Same swap-START loader signal as a hub `New` — this is a daemon-driven attach
            // gap, equally frozen until the new session's first Snapshot.
            push_switching(push, &new_id);
            // Daemon-driven hand-off carries no picked folder — inherit the host cwd.
            return HostTransition::Attach { id: new_id, workdir: None };
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

        // --- (b-sex) GIT panel: push any completed off-thread status fetch ---
        while let Ok(result) = git_status_rx.try_recv() {
            push_git_status(push, result);
        }

        // --- (b-sept) GIT panel: push any completed off-thread diff fetch ---
        while let Ok(result) = git_diff_rx.try_recv() {
            push_git_diff(push, result);
        }

        // --- (b-oct) GIT panel: push any completed off-thread mutation result ---
        // The worker also sent a follow-up status over `git_status_tx`, drained at
        // (b-sex) above — same frame or the next, whichever the loop happens to reach
        // first (harmless either order: both are one-shot, self-contained pushes).
        while let Ok(result) = git_op_rx.try_recv() {
            push_git_op(push, result);
        }

        // --- (b-ter) mirror the staged-attachment markers for the ipc Submit append ---
        // The ipc thread appends these `[Image #N]` markers to a chat send so the daemon's
        // submit-time reconcile keeps the staged images (React's text carries no markers).
        if let Ok(mut marks) = live_marks.lock() {
            marks.clear();
            marks.extend(shadow.rest.fg().pending_attachments.iter().map(|a| a.marker_n));
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

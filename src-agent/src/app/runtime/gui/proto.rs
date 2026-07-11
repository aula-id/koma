//! Wire types for the GUI ipc bridge: the tao-event-loop's internal
//! [`UserEvent`]/[`WinCmd`], and the native-React client<->host request
//! protocol ([`ClientMsg`]/[`GuiReq`]). Split out of [`super`] (the `gui`
//! module) for file size — pure code motion, no behaviour change. All four
//! types are bumped to `pub(super)` (were private) so both `gui::mod` (the
//! tao event loop + ipc-handler wiring) and the sibling `gui::dispatch`
//! module (the extracted `GuiReq` match) can name them.

/// Events delivered to the main `tao` event loop from the ipc handler (window
/// commands) or the host-relay client-thread (state pushes).
pub(super) enum UserEvent {
    /// A custom-titlebar window command posted from the webview.
    Win(WinCmd),
    /// A ready-to-inject JSON envelope from the host-relay client-thread. The main
    /// thread hands it to `window.__komaClient.push(...)` via `evaluate_script`. The
    /// payload is a COMPLETE JSON object (tagged on `k` — `Snapshot`/`StreamMsg`/
    /// `Reasoning`/`Status`/`Hub`), so it is embedded verbatim (not quoted).
    Push(String),
}

/// Window-management commands the HTML titlebar (drag region, minimize /
/// maximize / close buttons, edge resize handles) posts over ipc, since the
/// window is undecorated (`with_decorations(false)`) and has no native
/// titlebar to drive these.
#[derive(Clone, Copy)]
pub(super) enum WinCmd {
    Drag,
    Minimize,
    ToggleMax,
    Close,
    Resize(tao::window::ResizeDirection),
}

/// Messages posted from `koma.js` via `window.ipc.postMessage(JSON.stringify(..))`.
/// Internally tagged on `t`; unknown tags / malformed JSON fail to deserialize
/// and are ignored (the ipc handler must never panic).
#[derive(serde::Deserialize)]
#[serde(tag = "t")]
pub(super) enum ClientMsg {
    /// Custom-titlebar window command: drag / minimize / toggle-maximize / close.
    #[serde(rename = "win")]
    Win { a: String },
    /// Custom edge/corner resize-handle drag; `dir` is one of
    /// `e`/`w`/`n`/`s`/`ne`/`nw`/`se`/`sw`.
    #[serde(rename = "winresize")]
    WinResize { dir: String },
    /// The native-React client protocol (host-relay bridge). Tagged `"req"` on the
    /// outer `t`; the inner [`GuiReq`] carries the actual request keyed on `r`
    /// (`Ready` / `Submit` / `SelectSession` / `NewSession`). This is the ONLY
    /// inbound channel once the PTY-for-chat path is retired — the page drives the
    /// daemon through it, and the host pushes authoritative state back via
    /// `window.__komaClient.push(...)`.
    #[serde(rename = "req")]
    Req(GuiReq),
}

/// The native-React client -> host request, carried inside [`ClientMsg::Req`] and
/// internally tagged on `r`. Mirrors the JS→Rust half of the host-relay bridge
/// contract exactly:
///   - `Ready` — the page booted; the host sends its first push (a `Hub` if it is
///     in the swapper, else a `Snapshot`).
///   - `Submit { text }` — a chat send; forwarded to the attached daemon as
///     [`ClientRequest::SubmitInput`].
///   - `SelectSession { id }` — a hub pick; the host-thread attaches to that daemon.
///   - `NewSession` — the hub `[+ new session]` row; mint a fresh uuid + attach.
///   - `RefreshHub` — the ResumePalette overlay opened (and may re-emit while open):
///     ask the host to re-run cross-daemon discovery and push a FRESH `Hub` envelope,
///     so the live-session list is current even while ATTACHED (it was previously only
///     built once, cold, in the swapper). This is the live-session-listing fix.
///
/// Deserialised from the SAME JSON map as the outer [`ClientMsg`] (serde internal
/// tagging strips `t`, then this reads `r`), so `{ "t":"req", "r":"Submit",
/// "text":"…" }` round-trips into `ClientMsg::Req(GuiReq::Submit { text })`.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "r")]
pub(super) enum GuiReq {
    Ready,
    Submit { text: String },
    SelectSession { id: String },
    /// The hub `[+ new session]` row, or the attached chat view's "new session". `kill`
    /// (default `false`) additionally reaps the CURRENTLY-ATTACHED session's daemon as part
    /// of the switch (the chat view's "close this + start fresh"); the plain start-screen add
    /// omits it. Forwarded — after the native folder picker confirms — as [`HostCtl::New`].
    NewSession {
        #[serde(default)]
        kill: bool,
    },
    /// A hub session row's KILL button (a live COOKING row, or the attached session itself).
    /// Forwarded as [`HostCtl::KillSession`]; the host escalates the kill off its control
    /// loop and refreshes the hub once the daemon is confirmed dead.
    KillSession { id: String },
    /// A hub HISTORY row's DELETE button: physically remove that session (disk + registry).
    /// Forwarded as [`HostCtl::DeleteSession`]; the host resolves the path from the uuid and
    /// refuses to delete a live/locked session (defense in depth).
    DeleteSession { id: String },
    RefreshHub,
    /// Cancel an in-progress session switch (the full-screen loader's Cancel button):
    /// best-effort bail back to the hub. Forwarded as [`HostCtl::ToSwapper`]. The swap
    /// itself can't be interrupted (the host-thread blocks in the attach), so this is
    /// acted on once the target lands — the host then drops to the swapper and pushes a
    /// fresh `Hub`, which clears the loader React-side.
    CancelSwitch,
    /// Attach RAW file bytes from the page (a clipboard-image paste, a drag-drop, or a
    /// file-picker pick). The host base64-decodes `bytes_b64`, writes them to a
    /// host-writable scratch path (preserving `name`'s extension so the daemon's
    /// image-path sniff still fires), then forwards a [`ClientRequest::Paste`] of that
    /// path — reusing the daemon's EXISTING attachment ingest (image paths land in
    /// `pending_attachments`; other files fall through to the daemon's paste handling).
    /// `mime` is carried for the contract but the daemon sniffs by extension/bytes.
    AttachFile {
        name: String,
        // Carried for the bridge contract; the daemon sniffs by extension/bytes so the
        // host never needs to read it (the scratch write preserves `name`'s extension).
        #[serde(default)]
        #[allow(dead_code)]
        mime: Option<String>,
        #[serde(rename = "bytesB64")]
        bytes_b64: String,
    },
    /// Attach an EXISTING on-disk file by path (an omnisearch pick — the file already
    /// lives in the workspace, so no bytes are shipped). Forwarded verbatim as a
    /// [`ClientRequest::Paste`]: an image path is ingested into `pending_attachments`;
    /// a non-image path is handled by the daemon's paste path as before.
    AttachPath { path: String },
    /// Drop a staged attachment chip by its `[Image #N]` marker number (`markerN`).
    /// Forwarded as [`ClientRequest::RemoveAttachment`], which unstages it daemon-side;
    /// the resulting `pending_attachments` change re-emits the Snapshot (chips update).
    RemoveAttachment {
        #[serde(rename = "markerN")]
        marker_n: usize,
    },
    /// Omnisearch: fuzzy-search the workspace file index. Forwarded as
    /// [`ClientRequest::FileSearch`]; the daemon's one-shot reply is re-pushed to JS as a
    /// `SearchResults` envelope by the host `push_loop`. Select a result → `AttachPath`.
    FileSearch {
        query: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Rename the foreground session (the RenameOverlay submit). Forwarded verbatim as
    /// [`ClientRequest::RenameSession`], which sets the session's name + persists it
    /// (registry + settings) daemon-side; the resulting title change re-emits the
    /// Snapshot so `Snapshot.title` — which the overlay prefills from — updates.
    Rename { name: String },

    // ─── GUI config setters (Connector + MCP panels) ─────────────────────────
    // Forwarded to the attached daemon (which owns `AppConfig`) as the matching
    // gui-gated [`ClientRequest`]; the daemon mutates + persists config and re-emits a
    // fresh `Config` push. Field shapes mirror the panel form models exactly.
    /// Upsert an MCP server (McpPanel add/edit). `uuid` is absent for a new server.
    SetMcpServer {
        #[serde(default)]
        uuid: Option<String>,
        name: String,
        enabled: bool,
        transport: String,
        command: String,
        args: String,
        env: String,
        url: String,
    },
    /// Remove an MCP server by uuid (McpPanel arm-delete).
    DeleteMcpServer { uuid: String },
    /// Toggle an MCP server's enabled flag by uuid (McpPanel list switch).
    EnableMcpServer { uuid: String, enabled: bool },
    /// Upsert a provider (Connector ProviderForm). `uuid` is absent for a new provider.
    SetProvider {
        #[serde(default)]
        uuid: Option<String>,
        name: String,
        endpoint: String,
        #[serde(rename = "apiKey")]
        api_key: String,
    },
    /// Remove a provider by uuid (Connector arm-delete).
    DeleteProvider { uuid: String },
    /// Upsert a model (Connector ModelForm). `uuid` is absent for a new model; `roles`
    /// are lowercase tokens; `scope` is `"global"`/`"local"`.
    SetModel {
        #[serde(default)]
        uuid: Option<String>,
        name: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(rename = "providerUuid")]
        provider_uuid: String,
        #[serde(default)]
        route: Option<String>,
        roles: Vec<String>,
        scope: String,
    },
    /// Remove a model by uuid from the addressed `scope` (Connector arm-delete).
    DeleteModel { uuid: String, scope: String },
    /// Fetch the live model-id catalogue for a provider (Connector model picker). The
    /// daemon replies out-of-band; the host re-pushes it as a `ModelList` envelope.
    ListModels { provider: String },
    /// Fetch the live PROVIDER-ROUTE list for one model (Connector ModelForm route picker),
    /// keyed by `provider` uuid + `modelId` (`author/slug`). The daemon fetches the model's
    /// OpenRouter endpoints (non-OpenRouter → empty) and replies out-of-band; the host
    /// re-pushes it as a `RouteList` envelope. Refetched by React whenever the form's
    /// provider or model-id changes.
    ListRoutes {
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    /// Explore "FILE CHANGED" panel: fetch a host-computed diff (original @ `git show
    /// HEAD:<path>` vs the current on-disk contents) for `path` — a `fileChanges`
    /// record's path — to open a Monaco diff tab. Serviced ENTIRELY host-side (the host
    /// process has direct filesystem + git access, so no daemon round-trip is needed or
    /// wanted): routed UNCONDITIONALLY to the host-relay thread via `HostCtl::FileDiff`,
    /// unlike `ListModels`/`ListRoutes`'s attached-daemon-preferring dual routing, so it
    /// works identically whether a session is attached or not.
    FileDiff { path: String },
    /// Explore "GIT" panel opened / refreshed: fetch a host-computed git status
    /// (branch, ahead/behind, staged + unstaged file lists) for the foreground
    /// session's repo. Same reasoning as `FileDiff` — the host process has direct git
    /// access, so no daemon round-trip is needed or wanted: routed UNCONDITIONALLY to
    /// the host-relay thread via `HostCtl::GitStatus`, regardless of attach state.
    GitStatus,
    /// The GIT panel's file row clicked: fetch a host-computed git diff for `path` —
    /// `staged` selects index-vs-HEAD (`true`, the STAGED changes) or worktree-vs-index
    /// (`false`, the UNSTAGED changes) — to open a Monaco diff tab. Same reasoning and
    /// routing as `GitStatus`/`FileDiff`: routed UNCONDITIONALLY to the host-relay
    /// thread via `HostCtl::GitDiff`.
    GitDiff {
        path: String,
        staged: bool,
    },
    /// The GIT panel's "Stage All" header action / a row's hover "+" button: `git add
    /// --` the given repo-root-relative `paths` (a `Vec` so "stage all" is ONE
    /// round-trip covering every unstaged path at once). This also stages the removal
    /// of a tracked file deleted on disk (`git add`'s own behaviour — correct, not a
    /// bug). Routed UNCONDITIONALLY to the host-relay thread via `HostCtl::GitStage`,
    /// same reasoning as `GitStatus`/`GitDiff` — host-local, never the daemon. The
    /// reply is a one-shot `GitOp` envelope, immediately followed by a fresh
    /// `GitStatus` push so the panel's lists refresh from authoritative state.
    GitStage { paths: Vec<String> },
    /// The GIT panel's "Unstage All" header action / a staged row's hover "−" button:
    /// `git restore --staged --` the given `paths`. Same routing + reply pattern as
    /// `GitStage`.
    GitUnstage { paths: Vec<String> },
    /// The GIT panel's "Discard All Changes" header action / an unstaged row's discard
    /// button — destructive, so the React side gates it behind a confirm BEFORE
    /// sending this (this request itself is not reconfirmed host-side). PER PATH: an
    /// untracked file is deleted from disk; a tracked file's unstaged edits are reset
    /// from the index (`git restore --`) — staged content is never touched. Same
    /// routing + reply pattern as `GitStage`; see `git_discard`.
    GitDiscard { paths: Vec<String> },
    /// The GIT panel's commit box submit: `git commit -m <message>` whatever is
    /// CURRENTLY staged. An empty/whitespace-only `message` is rejected host-side (no
    /// git invocation at all) — the reply's `GitOp.error` surfaces that (or git's own
    /// stderr, e.g. "nothing to commit"). Same routing as `GitStage`; the React commit
    /// box clears its draft on a successful (`ok:true`) reply.
    GitCommit { message: String },
    /// The GitKraken-style commit-graph panel opened / scrolled (load-more): fetch
    /// `limit` commits starting `skip` back from the tip, across every ref. Same
    /// reasoning + routing as `GitStatus` — routed UNCONDITIONALLY to the host-relay
    /// thread via `HostCtl::GitGraph`, host-local, never the daemon.
    GitGraph { limit: u32, skip: u32 },
    /// A commit-graph row clicked: fetch that commit's full metadata (incl. body) +
    /// changed-file list to open the commit-detail view. Same routing as `GitGraph`.
    GitCommitDetail { sha: String },
    /// A commit-detail file row clicked: fetch a host-computed diff for `path` at
    /// commit `sha` (vs its first parent) to open a Monaco diff tab. Same routing as
    /// `GitGraph`.
    GitCommitDiff { sha: String, path: String },
    /// The GIT panel's key-picker changed: assign the foreground session's repo to
    /// use SSH key `name` (a vault key from the Settings "SSH Keys" section) for
    /// remote ops, or clear the assignment (`name: null` — "Default (system ssh)").
    /// Routed UNCONDITIONALLY to the host-relay thread via `HostCtl::SetGitKey`,
    /// same reasoning as `GitStage` — host-local, never the daemon. No dedicated
    /// reply; the reply is a fresh `GitStatus` push reflecting the new `keyName`.
    SetGitKey { name: Option<String> },
    /// The GIT panel's Fetch button: `git fetch --prune` for the foreground
    /// session's repo, using its assigned key's SSH override if one is set. Same
    /// routing as `GitStage` — host-local, never the daemon. The reply is a
    /// one-shot `GitOp` envelope (`op: "fetch"`), immediately followed by a fresh
    /// `GitStatus` push so ahead/behind refresh.
    GitFetch,
    /// The GIT panel's Pull button: `git pull --ff-only` (fails loudly on any
    /// divergence rather than merging or leaving a half-merged tree). Same
    /// routing + reply pattern as `GitFetch`.
    GitPull,
    /// The GIT panel's Push button. Same routing + reply pattern as `GitFetch`.
    GitPush,
    /// Branch-switcher popover (footer/GitPanel) or graph context menu opened
    /// (G4): fetch every local + remote-tracking branch. Same routing as
    /// `GitStatus` — host-local, never the daemon. Reply lands as a
    /// `BranchList` push.
    GitBranchList,
    /// Source Control multi-repo picker opened: discover every git repo across the
    /// session's workdirs. Same routing as `GitStatus` — host-local, never the
    /// daemon. Reply lands as a `RepoList` push.
    GitRepos,
    /// Source Control repo picker changed: set the session's ACTIVE repo to `root`
    /// (an absolute toplevel path from a prior `RepoList`). Routed to the host-relay
    /// thread via `HostCtl::SetActiveRepo`, same reasoning as `SetGitKey` — the reply
    /// is a fresh `GitStatus` push for the newly-active repo.
    SetActiveRepo { root: String },
    /// Branch-switcher pick / graph "Checkout"/"Checkout commit" (G4 — SAFE
    /// only, never `--force`): switch (or detach onto) `ref` — a branch name or
    /// a sha. `ref` is a Rust keyword, so the field is `ref_name`
    /// (`#[serde(rename = "ref")]` keeps the wire key `ref`). Same routing +
    /// reply pattern as `GitStage` (`GitOp` then a fresh `GitStatus`).
    GitCheckout {
        #[serde(rename = "ref")]
        ref_name: String,
    },
    /// Branch-switcher "+ Create new branch" / graph "Create branch here…"
    /// (G4 — SAFE only): create branch `name`. `start` is the commit-ish to
    /// branch from (`null`/omitted = current HEAD); `checkout` switches to it
    /// immediately (`git checkout -b`) vs only creating it (`git branch`).
    /// Same routing + reply pattern as `GitCheckout`.
    GitCreateBranch {
        name: String,
        start: Option<String>,
        checkout: bool,
    },
    /// Commit-graph row context menu "Cherry-pick" (G5b — may conflict; the
    /// follow-up `GitStatus` push's `inProgress`/`conflicted` fields carry that
    /// state, not this request's reply alone). Forwarded as
    /// [`HostCtl::GitCherryPick`].
    GitCherryPick { sha: String },
    /// Commit-graph row context menu "Revert" (G5b). Same conflict reasoning as
    /// `GitCherryPick`. Forwarded as [`HostCtl::GitRevert`].
    GitRevert { sha: String },
    /// Commit-graph row context menu "Reset branch to here" (G5b). `mode` is
    /// `"soft"`/`"mixed"`/`"hard"` — `hard` DISCARDS uncommitted changes; the
    /// React side gates this behind a confirm BEFORE sending it (this request
    /// itself is not reconfirmed host-side). Forwarded as [`HostCtl::GitReset`].
    GitReset { sha: String, mode: String },
    /// Branch-switcher / graph context menu "Merge into current branch" (G5b —
    /// may conflict, same reasoning as `GitCherryPick`). `ref` is a Rust
    /// keyword, so the field is `ref_name` (`#[serde(rename = "ref")]` keeps the
    /// wire key `ref`, same trick as `GitCheckout`). Forwarded as
    /// [`HostCtl::GitMerge`].
    GitMerge {
        #[serde(rename = "ref")]
        ref_name: String,
    },
    /// Rebase onto `upstream` (G5b/G6). `branch` is `Some(name)` for the
    /// GitKraken-style drag-to-rebase (drag a branch chip onto a commit/ref —
    /// `branch` is checked out and rebased onto `upstream`; the current branch is
    /// left alone), or `None` for the plain "rebase current branch" op. May
    /// conflict. Forwarded as [`HostCtl::GitRebase`].
    GitRebase {
        upstream: String,
        #[serde(default)]
        branch: Option<String>,
    },
    /// The conflict banner's Abort button (G5b). `kind` is `"merge"`/
    /// `"rebase"`/`"cherry-pick"`/`"revert"` (echoing `GitStatus.inProgress`).
    /// Forwarded as [`HostCtl::GitOpAbort`].
    GitOpAbort { kind: String },
    /// The conflict banner's Continue button (G5b). Same `kind` values as
    /// `GitOpAbort`. Forwarded as [`HostCtl::GitOpContinue`] — the host runs it
    /// with `GIT_EDITOR=true` so it never hangs on an editor prompt.
    GitOpContinue { kind: String },
    /// Activity-bar "Usage" panel: fetch a host-computed LAST-7-DAYS usage preview
    /// (totals, a 7-entry daily cost series, top 3 models) straight off the global
    /// `~/.koma/usage.sqlite` ledger. Same reasoning as `FileDiff`: the ledger is a
    /// process-local file the host can read directly, so this is routed UNCONDITIONALLY
    /// to the host-relay thread via `HostCtl::UsagePreview` regardless of attach state —
    /// no daemon round-trip. Re-sent by React every time the panel becomes active, and
    /// again whenever the header's all/session scope toggle flips.
    ///
    /// `scope` is `"all"` (default, global last-7-days) or `"session"` (same window,
    /// filtered to `sessionId`'s ledger rows only); both default (via `#[serde(default)]`)
    /// so the pre-scope no-field wire form still deserializes. `sessionId` is the
    /// foreground session's uuid — required for a `"session"` scope to mean anything. A
    /// `"session"` scope with no `sessionId` (the welcome/start-screen state) is FORCED to
    /// `"all"` by the ipc handler before it queries — the echoed reply's `scope` always
    /// describes what was actually queried, never a mismatched label.
    UsagePreview {
        #[serde(default)]
        scope: Option<String>,
        #[serde(default, rename = "sessionId")]
        session_id: Option<String>,
    },
    /// Set the active theme (onboarding theme step + the future Settings gear). `name` is
    /// a `view::theme::PALETTES` key. Forwarded as [`ClientRequest::SetTheme`] when
    /// ATTACHED (the daemon persists + re-pushes the Config palette), or applied directly
    /// to `~/.koma/config.json` on the swapper thread when PRE-SESSION (onboarding runs
    /// before any session exists) via [`HostCtl::ConfigMutate`].
    SetTheme { name: String },
    /// The GUI onboarding "koma free" choice: mint/reuse the keyless Koma Free provider +
    /// a Main-role model in the GLOBAL config. Routed EXACTLY like the config setters via
    /// `forward_config_req` (works ATTACHED through the daemon and UN-ATTACHED through the
    /// swapper's `ConfigMutate` path), reusing the shared
    /// [`crate::service::koma_free::ensure_koma_free_config`] mutation — the resulting
    /// `Config` re-push clears `firstRun`, which is what dismisses the GUI onboarding overlay.
    SetupKomaFree,

    // ─── GUI turn/session controls (stop button + kill buttons + model picker) ────
    // Same forward-to-attached-daemon pattern as `Submit`: lock `ipc_req` + send the
    // matching gui-gated [`ClientRequest`]. All no-ops when no session is attached.
    /// The composer STOP button (shown while the turn is working): interrupt the running
    /// turn. Forwarded as [`ClientRequest::Interrupt`] (koma's Esc-interrupt equivalent).
    Interrupt,
    /// The composer `!<cmd>` shell shortcut: run `cmd` in the foreground session's
    /// cwd, no model round-trip. Forwarded verbatim as [`ClientRequest::Shell`] —
    /// the daemon's `Action::Shell` handler appends the `$ cmd` + output entry to
    /// the transcript. Attached-only (like `Interrupt`); the React composer only
    /// sends this while idle (mirrors the TUI's busy guard) — while working it
    /// falls through to a normal `Submit` instead (queues as a steer), a
    /// deliberate deviation from the TUI (which no-ops a `!` line while busy).
    Shell { cmd: String },
    /// Ctrl+R composer parity: resend the last user turn (pop trailing assistant
    /// messages + re-stream). Forwarded as [`ClientRequest::Resend`], attached-only
    /// (like `Interrupt`) — the daemon's `handle_resend` reports a no-op (busy /
    /// no session / nothing to resend) via the status line.
    Resend,
    /// The composer's queued-steer-list clear button: cancel every pending
    /// mid-turn steer at once. Forwarded as [`ClientRequest::CancelSteers`],
    /// attached-only, like `Interrupt`.
    CancelSteers,
    /// The chat hover-edit PENCIL on a user bubble: rewind the conversation TO that
    /// message by its `index` into `SessionSnapshot.messages` (Conversation::messages()).
    /// Forwarded as [`ClientRequest::RewindTo`], which runs koma's `RewindToMessage`
    /// action (abort in-flight turn + truncate live/sqlite + refill the composer); a
    /// non-user / out-of-range index is a clean daemon-side no-op.
    RewindTo { index: usize },
    /// The Explore sidepanel agent-row KILL button: kill sub-agent `id`. Forwarded as
    /// [`ClientRequest::KillSubagent`].
    KillSubagent { id: usize },
    /// The Explore sidepanel agent-row BACKGROUND button: background sub-agent `id`
    /// (flip it to detached without killing it — the agent keeps running, its report
    /// lands via a later push instead of parking the turn). Forwarded as
    /// [`ClientRequest::BackgroundSubagent`].
    BackgroundSubagent { id: usize },
    /// The global Ctrl+B shortcut: background EVERY eligible running sub-agent at once
    /// (mirrors the TUI composer's Ctrl+B). Forwarded as
    /// [`ClientRequest::BackgroundAllSubagents`].
    BackgroundAll,
    /// The Explore sidepanel bash-row KILL button: kill bg-bash job `id` (the numeric part
    /// of the row's `bash-<id>`). Forwarded as [`ClientRequest::BashKill`].
    KillBash { id: usize },
    /// The Explore STREAM TAB view changed: `subagent`/`bash` name which sub-agent / bash
    /// job the webview is live-streaming (the active stream tab), or both absent = no
    /// stream tab. The host remembers this LOCALLY (shared `live_view`, read by the fold to
    /// decide whose transcript / output tail to push) AND forwards it as
    /// [`ClientRequest::SetStreamView`] so the daemon un-suppresses the viewed detached
    /// sub-agent's live churn + projects the viewed bash job's output tail. The local update
    /// happens regardless of attach state; the daemon forward is attached-only (like
    /// `Interrupt`) — a fresh attach's daemon starts with no view anyway.
    ///
    /// `session` is the UUID of the session the ids belong to (the webview sends its current
    /// `session.id`). Forwarded verbatim so the daemon can PIN the view — sub-agent + bash
    /// ids are per-session counters, so the daemon gates both consumers on it. The host-local
    /// `live_view` does NOT need it: the fold reads the foreground session's own snapshot
    /// (already session-scoped) and `live_view` is reset on every session switch.
    SetStreamView {
        #[serde(default)]
        subagent: Option<usize>,
        #[serde(default)]
        bash: Option<usize>,
        #[serde(default)]
        session: Option<String>,
    },
    /// The model quick-picker: set the session-local Main override to the GLOBAL model
    /// `modelUuid`, or clear it (inherit) when `modelUuid` is absent/null. Forwarded as
    /// [`ClientRequest::SetSessionMain`].
    SetSessionMain {
        #[serde(rename = "modelUuid", default)]
        model_uuid: Option<String>,
    },
    /// The composer mode selector: set the GLOBAL agent mode to `mode`
    /// (`"auto"`/`"normal"`/`"plan"`/`"yolo"`). Forwarded as [`ClientRequest::SetMode`],
    /// which routes through the daemon's `set_agent_mode` choke-point; the resulting
    /// snapshot re-projection reflects the new mode back to every attached client.
    SetMode { mode: String },

    // ─── GUI approval gate (wave-7 approval overlay) ─────────────────────────────
    // The GUI renders the paused-call overlay off `Snapshot.awaitingApproval` +
    // `pendingCall`; these answer it, reusing the daemon's EXISTING approval/plan resume
    // logic (no reimplementation). Same forward-to-attached-daemon pattern as `Submit`.
    /// The tool-approval card's Approve/Deny buttons (paused risky/classifier call).
    /// Forwarded as [`ClientRequest::ApproveTool`] — `approve:true` runs the call,
    /// `false` bounces it back to the model (koma's y/n equivalent).
    ApproveTool { approve: bool },
    /// The status-footer Compact action: summarise + trim the foreground session's
    /// history. Forwarded as [`ClientRequest::Compact`] (koma's `/compact` equivalent).
    /// Compacting without an attached session is meaningless, so the un-attached case
    /// is a silent no-op (same pattern as `Interrupt`/`RewindTo`).
    Compact,
    /// The plan-approval card's controls (paused `plan_ready` digest). `decision` is one
    /// of `"approve"`, `"compact"` (approve + compact history to the plan), or `"deny"`
    /// (keep discussing). Forwarded verbatim as [`ClientRequest::PlanDecision`], koma's
    /// y/a/n plan-resume equivalent.
    PlanDecision { decision: String },

    // ─── GUI Settings tab (vscode-style page) ────────────────────────────────────
    /// The Settings tab opened / re-activated: fetch the foreground session's GUI-editable
    /// prefs + the active palette. Dual-routed like `ListModels` via [`forward_or_host`] —
    /// the attached daemon answers with `SettingsValues`, or (un-attached, StartScreen /
    /// swapper) the host answers from the global config — so the tab populates in BOTH host
    /// states and its loading state never hangs.
    GetSettings,
    /// The Settings tab's Session section committed a partial update. Only the present
    /// (`Some`) fields are sent; forwarded to the attached daemon as
    /// [`ClientRequest::SetSessionPrefs`] (attached-only, like `Interrupt` — a settings edit
    /// with no live session is a silent no-op). The daemon applies each field through the
    /// SAME per-field logic the TUI settings save uses, persists, and re-pushes
    /// `SettingsValues`. `internetMode` is `"simple"`/`"full"`. (Name changes reuse `Rename`;
    /// palette changes reuse `SetTheme` — no new plumbing.)
    SetPrefs {
        #[serde(default, rename = "shortSend")]
        short_send: Option<bool>,
        #[serde(default, rename = "slidingCache")]
        sliding_cache: Option<bool>,
        #[serde(default, rename = "bashSaving")]
        bash_saving: Option<bool>,
        #[serde(default, rename = "internetMode")]
        internet_mode: Option<String>,
        #[serde(default)]
        workdir: Option<Vec<String>>,
    },

    // ─── GUI composer EFFORT picker (TUI `/effort` parity) ───────────────────────
    /// The composer's EFFORT pill opened: fetch the derived `/effort` menu for the
    /// foreground session's current model. Forwarded as
    /// [`ClientRequest::GetEffortOptions`], attached-only (like `Interrupt` — the
    /// menu is per-session/per-model, so there's nothing to derive un-attached; a
    /// no-op there just means the request is never sent, and the picker shows its
    /// loading state until an attach lands). The daemon ALWAYS replies with an
    /// `EffortOptions` frame the host re-pushes as an `EffortOptions` envelope.
    GetEffortOptions,
    /// The EFFORT picker's row pick: persist the chosen effort level. Forwarded as
    /// [`ClientRequest::SetEffort`], attached-only like `SetPrefs`. The daemon
    /// re-pushes a fresh `SettingsValues` as the reply, updating the picker's
    /// trigger-pill label.
    SetEffort { effort: String },

    // ─── GUI /agents dashboard (sub-agent definitions) ───────────────────────────
    /// The /agents dashboard opened / refreshed: fetch the merged sub-agent registry +
    /// model / provider catalogue. Dual-routed like `GetSettings` via [`forward_or_host`] —
    /// the attached daemon answers with `AgentsValues`, or (un-attached, StartScreen /
    /// swapper) the host answers from `load_registry(None)` (built-in + global) + the global
    /// config catalogue — so the dashboard populates in BOTH host states and never hangs.
    GetAgents,
    /// The /agents editor's create / save (an upsert). `originalName` is the pre-edit name
    /// (`Some` + differs from `name` = rename); `scope` is `"global"` / `"session"`.
    /// Forwarded like the config setters via [`forward_config_req`] (attached → daemon;
    /// pre-session → the swapper `ConfigMutate` path, a no-op for agents there); the daemon
    /// persists, rebuilds the session roster, and re-pushes `AgentsValues`. Fields are
    /// camelCase to match the JS contract.
    SetAgent {
        #[serde(default, rename = "originalName")]
        original_name: Option<String>,
        scope: String,
        name: String,
        description: String,
        #[serde(default)]
        conditions: String,
        #[serde(default, rename = "modelUuid")]
        model_uuid: Option<String>,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        prompt: String,
    },
    /// The /agents dashboard's delete (a file-backed agent; a built-in is a daemon-side
    /// error). `scope` is `"global"` / `"session"`. Forwarded like the config setters.
    DeleteAgent { scope: String, name: String },

    // ─── GUI OAuth surface (Codex / Kilo Code / xAI login) ───────────────────────
    /// The OAuth screen opened / refreshed: fetch the current state (connections +
    /// available providers). Dual-routed like `GetSettings`/`GetAgents` via
    /// [`forward_or_host`] — the attached daemon answers with `OAuthState`, or (un-attached)
    /// the host answers from `~/.koma/config.json` + the provider registry — so the screen
    /// populates in BOTH host states and never hangs.
    GetOAuthState,
    /// Start a login flow. `provider` is `"codex"` / `"kilocode"` / `"xai"` / `"codex_paste"`.
    /// Forwarded as [`ClientRequest::StartOAuth`], attached-only (like `Interrupt` — the flow
    /// runs on the daemon's runtime; un-attached login is not supported this wave). The daemon
    /// streams progress back as `OAuthState` pushes the host re-pushes as `OAuthState`
    /// envelopes.
    StartOAuth { provider: String },
    /// Complete the Codex paste-token flow with a raw access `token`. Forwarded as
    /// [`ClientRequest::SubmitOAuthPaste`], attached-only.
    SubmitOAuthPaste { token: String },
    /// Cancel an in-flight login flow. Forwarded as [`ClientRequest::CancelOAuth`],
    /// attached-only.
    CancelOAuth,
    /// Delete a persisted OAuth connection by `uuid`. Dual-routed like `GetOAuthState` via
    /// [`forward_or_host`] — the attached daemon deletes + evicts, or (un-attached) the host
    /// removes it from the on-disk config + evicts the cache — then re-pushes a fresh
    /// `OAuthState`, so a connection is removable pre-session too.
    DeleteOAuthConn { uuid: String },

    // ─── GUI SSH key vault (Settings "SSH Keys" submenu, wave 4a) ────────────────
    // A GUI-only, MANUAL, user-owned key vault (`<~/.koma>/keys/`) — completely
    // separate from the model's own git credential machinery (`git_cred.rs`/
    // `git_operator.rs`). Every request here is routed UNCONDITIONALLY to the
    // host-relay thread, same reasoning as `GitStatus`/`FileDiff`: the host process
    // already has direct filesystem + `ssh-keygen` access, so no daemon round-trip
    // is needed or wanted, and it works identically whether a session is attached or
    // not. Remote push/pull (wave 4b) is NOT covered by this wave.
    /// The Settings "SSH Keys" section opened / refreshed: fetch the vault's current
    /// key list. Forwarded as [`HostCtl::KeyList`]; the reply is a one-shot `KeyList`
    /// envelope that never hangs (an empty vault is a valid "no keys yet" state).
    KeyList,
    /// Generate a fresh passphrase-less ed25519 keypair named `name` (`comment`
    /// defaults to `"koma"` when blank). Forwarded as [`HostCtl::KeyGenerate`]; the
    /// reply is a one-shot `KeyOp` envelope, immediately followed by a fresh
    /// `KeyList` push so the section's list refreshes.
    KeyGenerate { name: String, comment: String },
    /// Import an EXISTING private key (`name` + pasted `privateKey` text) into the
    /// vault; the host derives + writes the matching public half. Forwarded as
    /// [`HostCtl::KeyImport`]; same reply pattern as `KeyGenerate`.
    KeyImport {
        name: String,
        #[serde(rename = "privateKey")]
        private_key: String,
    },
    /// Reveal key `name`'s contents — `private: false` for "Copy public key" (no
    /// confirmation needed React-side), `private: true` for "Reveal private key"
    /// (gated behind a deliberate click + warning React-side; never surfaced
    /// passively — the user owns these keys outright). Forwarded as
    /// [`HostCtl::KeyReveal`]; the reply is a one-shot `KeyReveal` envelope.
    KeyReveal { name: String, private: bool },
    /// Delete keypair `name` (both halves, best-effort). Forwarded as
    /// [`HostCtl::KeyDelete`]; same reply pattern as `KeyGenerate`.
    KeyDelete { name: String },

    // ─── GitKraken-style stash ops (GK4a) ─────────────────────────────────────
    // Same reasoning as `GitStatus`/`GitStage` above — the host process already has
    // direct git access, so every request here is routed UNCONDITIONALLY to the
    // host-relay thread, regardless of attach state.
    /// The Source Control toolbar's Stash button: `git stash push` (tracked +
    /// staged changes). Forwarded as [`HostCtl::GitStash`]; the reply is a
    /// one-shot `GitOp` envelope, immediately followed by a fresh `GitStatus`
    /// push (stashing changes the working tree).
    GitStash,
    /// The Source Control toolbar's Pop button: `git stash pop`. May conflict —
    /// the follow-up `GitStatus` push's `conflicted` field carries that, not
    /// this reply alone. Forwarded as [`HostCtl::GitStashPop`].
    GitStashPop,
    /// The Source Control toolbar's stash count/indicator: fetch every stash
    /// entry. Forwarded as [`HostCtl::GitStashList`]; the reply is a one-shot
    /// `StashList` envelope.
    GitStashList,

    // ─── Bubble/activity chart (GK5a) ────────────────────────────────────────
    /// The activity chart opened / refreshed: fetch per-commit author/date/
    /// lines-changed for `limit` commits on the ACTIVE branch (`HEAD`), optionally
    /// scoped to one `path`. Same reasoning as `GitStatus`/`GitGraph` — the host
    /// process already has direct git access, so this is routed UNCONDITIONALLY to
    /// the host-relay thread via `HostCtl::GitActivity`, regardless of attach
    /// state. Reply lands as an `Activity` push.
    GitActivity {
        #[serde(default)]
        path: Option<String>,
        limit: u32,
    },
}

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
    /// A ready-to-inject JSON envelope from the host-relay client-thread. The GUI
    /// event loop frame-batches these as quoted strings through one
    /// `window.__komaClient.pushBatch(...)` evaluate_script call; React paces only
    /// heavy attach envelopes (Snapshot/Loading/Config/…) one frame at a time —
    /// stream/chat traffic is never queued behind them.
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

/// One message in a GUI Tutorial chat turn (`GuiReq::TutorialChat`).
#[derive(Debug, serde::Deserialize)]
pub(super) struct TutorialChatMsg {
    pub role: String,
    pub content: String,
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
    Submit {
        text: String,
    },
    SelectSession {
        id: String,
    },
    /// The hub `[+ new session]` row, or the attached chat view's "new session". `kill`
    /// (default `false`) additionally reaps the CURRENTLY-ATTACHED session's daemon as part
    /// of the switch (the chat view's "close this + start fresh"); the plain start-screen add
    /// omits it. Forwarded — after the native folder picker confirms — as [`HostCtl::New`].
    NewSession {
        #[serde(default)]
        kill: bool,
        /// When true, open a native folder picker before creating the session.
        #[serde(default)]
        folder: bool,
    },
    /// A hub session row's KILL button (a live COOKING row, or the attached session itself).
    /// Forwarded as [`HostCtl::KillSession`]; the host escalates the kill off its control
    /// loop and refreshes the hub once the daemon is confirmed dead.
    KillSession {
        id: String,
    },
    /// A hub HISTORY row's DELETE button: physically remove that session (disk + registry).
    /// Forwarded as [`HostCtl::DeleteSession`]; the host resolves the path from the uuid and
    /// refuses to delete a live/locked session (defense in depth).
    DeleteSession {
        id: String,
    },
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
    AttachPath {
        path: String,
    },
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
    Rename {
        name: String,
    },

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
    DeleteMcpServer {
        uuid: String,
    },
    /// Toggle an MCP server's enabled flag by uuid (McpPanel list switch).
    EnableMcpServer {
        uuid: String,
        enabled: bool,
    },
    /// Request live MCP server connection status for the sidebar panel. Answered with
    /// a one-shot `McpStatus` push envelope (routed through the attached daemon).
    /// `requestId` is echoed back so the frontend can discard stale replies.
    GetMcpStatus {
        #[serde(rename = "requestId")]
        request_id: String,
    },
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
    DeleteProvider {
        uuid: String,
    },
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
    DeleteModel {
        uuid: String,
        scope: String,
    },
    /// Fetch the live model-id catalogue for a provider (Connector model picker). The
    /// daemon replies out-of-band; the host re-pushes it as a `ModelList` envelope.
    ListModels {
        provider: String,
    },
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
    FileDiff {
        path: String,
    },
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
    GitStage {
        paths: Vec<String>,
    },
    /// The GIT panel's "Unstage All" header action / a staged row's hover "−" button:
    /// `git restore --staged --` the given `paths`. Same routing + reply pattern as
    /// `GitStage`.
    GitUnstage {
        paths: Vec<String>,
    },
    /// The GIT panel's "Discard All Changes" header action / an unstaged row's discard
    /// button — destructive, so the React side gates it behind a confirm BEFORE
    /// sending this (this request itself is not reconfirmed host-side). PER PATH: an
    /// untracked file is deleted from disk; a tracked file's unstaged edits are reset
    /// from the index (`git restore --`) — staged content is never touched. Same
    /// routing + reply pattern as `GitStage`; see `git_discard`.
    GitDiscard {
        paths: Vec<String>,
    },
    /// The GIT panel's commit box submit: `git commit -m <message>` whatever is
    /// CURRENTLY staged. An empty/whitespace-only `message` is rejected host-side (no
    /// git invocation at all) — the reply's `GitOp.error` surfaces that (or git's own
    /// stderr, e.g. "nothing to commit"). Same routing as `GitStage`; the React commit
    /// box clears its draft on a successful (`ok:true`) reply.
    GitCommit {
        message: String,
    },
    /// The GitKraken-style commit-graph panel opened / scrolled (load-more): fetch
    /// `limit` commits starting `skip` back from the tip, across every ref. Same
    /// reasoning + routing as `GitStatus` — routed UNCONDITIONALLY to the host-relay
    /// thread via `HostCtl::GitGraph`, host-local, never the daemon.
    GitGraph {
        limit: u32,
        skip: u32,
    },
    /// A commit-graph row clicked: fetch that commit's full metadata (incl. body) +
    /// changed-file list to open the commit-detail view. Same routing as `GitGraph`.
    GitCommitDetail {
        sha: String,
    },
    /// A commit-detail file row clicked: fetch a host-computed diff for `path` at
    /// commit `sha` (vs its first parent) to open a Monaco diff tab. Same routing as
    /// `GitGraph`.
    GitCommitDiff {
        sha: String,
        path: String,
    },
    /// The GIT panel's key-picker changed: assign the foreground session's repo to
    /// use SSH key `name` (a vault key from the Settings "SSH Keys" section) for
    /// remote ops, or clear the assignment (`name: null` — "Default (system ssh)").
    /// Routed UNCONDITIONALLY to the host-relay thread via `HostCtl::SetGitKey`,
    /// same reasoning as `GitStage` — host-local, never the daemon. No dedicated
    /// reply; the reply is a fresh `GitStatus` push reflecting the new `keyName`.
    SetGitKey {
        name: Option<String>,
    },
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
    GitPush {
        #[serde(default)]
        mode: Option<crate::app::runtime::client::git_remote::GitPushMode>,
        #[serde(default)]
        root: Option<String>,
    },
    /// Branch-switcher popover (footer/GitPanel) or graph context menu opened
    /// (G4): fetch every local + remote-tracking branch. Same routing as
    /// `GitStatus` — host-local, never the daemon. Reply lands as a
    /// `BranchList` push.
    GitBranchList {
        #[serde(default, rename = "requestId")]
        request_id: Option<u64>,
    },
    /// Source Control multi-repo picker opened: discover every git repo across the
    /// session's workdirs. Same routing as `GitStatus` — host-local, never the
    /// daemon. Reply lands as a `RepoList` push.
    GitRepos,
    /// Source Control repo picker changed: set the session's ACTIVE repo to `root`
    /// (an absolute toplevel path from a prior `RepoList`). Routed to the host-relay
    /// thread via `HostCtl::SetActiveRepo`, same reasoning as `SetGitKey` — the reply
    /// is a fresh `GitStatus` push for the newly-active repo.
    SetActiveRepo {
        root: String,
    },
    /// Branch-switcher pick / graph "Checkout"/"Checkout commit" (G4 — SAFE
    /// only, never `--force`): switch (or detach onto) `ref` — a branch name or
    /// a sha. `ref` is a Rust keyword, so the field is `ref_name`
    /// (`#[serde(rename = "ref")]` keeps the wire key `ref`). Same routing +
    /// reply pattern as `GitStage` (`GitOp` then a fresh `GitStatus`).
    GitCheckout {
        #[serde(rename = "ref")]
        ref_name: String,
        #[serde(default)]
        root: Option<String>,
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
        #[serde(default)]
        root: Option<String>,
    },
    /// Commit-graph row context menu "Cherry-pick" (G5b — may conflict; the
    /// follow-up `GitStatus` push's `inProgress`/`conflicted` fields carry that
    /// state, not this request's reply alone). Forwarded as
    /// [`HostCtl::GitCherryPick`].
    GitCherryPick {
        sha: String,
    },
    /// Commit-graph row context menu "Revert" (G5b). Same conflict reasoning as
    /// `GitCherryPick`. Forwarded as [`HostCtl::GitRevert`].
    GitRevert {
        sha: String,
    },
    /// Commit-graph row context menu "Reset branch to here" (G5b). `mode` is
    /// `"soft"`/`"mixed"`/`"hard"` — `hard` DISCARDS uncommitted changes; the
    /// React side gates this behind a confirm BEFORE sending it (this request
    /// itself is not reconfirmed host-side). Forwarded as [`HostCtl::GitReset`].
    GitReset {
        sha: String,
        mode: String,
    },
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
    GitOpAbort {
        kind: String,
    },
    /// The conflict banner's Continue button (G5b). Same `kind` values as
    /// `GitOpAbort`. Forwarded as [`HostCtl::GitOpContinue`] — the host runs it
    /// with `GIT_EDITOR=true` so it never hangs on an editor prompt.
    GitOpContinue {
        kind: String,
    },
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
    /// Analytics tab: fetch a host-computed usage dashboard (KPI totals, time
    /// series, per-model table, main-vs-sub role split) straight off the global
    /// `~/.koma/usage.sqlite` ledger. Same reasoning as `UsagePreview`: the ledger
    /// is a process-local file the host can read directly, so this is routed
    /// UNCONDITIONALLY to the host-relay thread via `HostCtl::Analytics` regardless
    /// of attach state — no daemon round-trip.
    ///
    /// Host-owned correlation: `reqSeq` is a client-minted monotonic request id
    /// (stale-reply protection); `scope` is `"all"`/`"session"`; `sessionId` is
    /// the foreground session uuid (required for a `"session"` scope); `range`
    /// is `"today"`/`"7d"`/`"30d"`/`"year"`; `metric` is `"cost"`/`"tokens"`. A
    /// `"session"` scope with no `sessionId` is FORCED to `"all"` by the ipc
    /// handler before it queries — the echoed reply's `scope` always describes
    /// what was actually queried.
    Analytics {
        #[serde(default, rename = "reqSeq")]
        req_seq: u64,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default, rename = "sessionId")]
        session_id: Option<String>,
        #[serde(default)]
        range: Option<String>,
        #[serde(default)]
        metric: Option<String>,
    },
    /// Set the active theme (onboarding theme step + the future Settings gear). `name` is
    /// a `view::theme::PALETTES` key. Forwarded as [`ClientRequest::SetTheme`] when
    /// ATTACHED (the daemon persists + re-pushes the Config palette), or applied directly
    /// to `~/.koma/config.json` on the swapper thread when PRE-SESSION (onboarding runs
    /// before any session exists) via [`HostCtl::ConfigMutate`].
    SetTheme {
        name: String,
    },
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
    Shell {
        cmd: String,
    },
    /// Ctrl+R composer parity: resend the last user turn (pop trailing assistant
    /// messages + re-stream). Forwarded as [`ClientRequest::Resend`], attached-only
    /// (like `Interrupt`) — the daemon's `handle_resend` reports a no-op (busy /
    /// no session / nothing to resend) via the status line.
    Resend,
    /// The composer's queued-steer-list clear button: cancel every pending
    /// mid-turn steer at once. Forwarded as [`ClientRequest::CancelSteers`],
    /// attached-only, like `Interrupt`.
    CancelSteers,
    /// Remove one queued follow-up by list index. Forwarded as
    /// [`ClientRequest::RemoveSteer`].
    RemoveSteer {
        index: usize,
    },
    /// Load one queued follow-up into the composer for edit (removes it from
    /// the queue). Forwarded as [`ClientRequest::EditSteer`]. The GUI should
    /// also `refillComposer` locally with the known full text, since the web
    /// composer does not reconcile `InputChanged`.
    EditSteer {
        index: usize,
    },
    /// The chat hover-edit PENCIL on a user bubble: rewind the conversation TO that
    /// message by its `index` into `SessionSnapshot.messages` (Conversation::messages()).
    /// Forwarded as [`ClientRequest::RewindTo`], which runs koma's `RewindToMessage`
    /// action (abort in-flight turn + truncate live/sqlite + refill the composer); a
    /// non-user / out-of-range index is a clean daemon-side no-op.
    RewindTo {
        index: usize,
    },
    /// Pull older transcript held after a windowed first Snapshot.
    /// `before` = current oldest display idx on the FE (exclusive). Host replies
    /// with `HistoryPage` push. Host-local (uses PushState stash), not the daemon.
    HistoryPage {
        #[serde(default)]
        before: Option<usize>,
    },
    /// The Explore sidepanel agent-row KILL button: kill sub-agent `id`. Forwarded as
    /// [`ClientRequest::KillSubagent`].
    KillSubagent {
        id: usize,
    },
    /// The Explore sidepanel agent-row BACKGROUND button: background sub-agent `id`
    /// (flip it to detached without killing it — the agent keeps running, its report
    /// lands via a later push instead of parking the turn). Forwarded as
    /// [`ClientRequest::BackgroundSubagent`].
    BackgroundSubagent {
        id: usize,
    },
    /// The global Ctrl+B shortcut: background EVERY eligible running sub-agent at once
    /// (mirrors the TUI composer's Ctrl+B). Forwarded as
    /// [`ClientRequest::BackgroundAllSubagents`].
    BackgroundAll,
    /// The Explore sidepanel bash-row KILL button: kill bg-bash job `id` (the numeric part
    /// of the row's `bash-<id>`). Forwarded as [`ClientRequest::BashKill`].
    KillBash {
        id: usize,
    },
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
    SetMode {
        mode: String,
    },

    // ─── GUI approval gate (wave-7 approval overlay) ─────────────────────────────
    // The GUI renders the paused-call overlay off `Snapshot.awaitingApproval` +
    // `pendingCall`; these answer it, reusing the daemon's EXISTING approval/plan resume
    // logic (no reimplementation). Same forward-to-attached-daemon pattern as `Submit`.
    /// The tool-approval card's Approve/Deny buttons (paused risky/classifier call).
    /// Forwarded as [`ClientRequest::ApproveTool`] — `approve:true` runs the call,
    /// `false` bounces it back to the model (koma's y/n equivalent).
    ApproveTool {
        approve: bool,
    },
    /// The status-footer Compact action: summarise + trim the foreground session's
    /// history. Forwarded as [`ClientRequest::Compact`] (koma's `/compact` equivalent).
    /// Compacting without an attached session is meaningless, so the un-attached case
    /// is a silent no-op (same pattern as `Interrupt`/`RewindTo`).
    Compact,
    /// The plan-approval card's controls (paused `plan_ready` digest). `decision` is one
    /// of `"approve"`, `"compact"` (approve + compact history to the plan), or `"deny"`
    /// (keep discussing). Forwarded verbatim as [`ClientRequest::PlanDecision`], koma's
    /// y/a/n plan-resume equivalent.
    PlanDecision {
        decision: String,
    },

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
        #[serde(default, rename = "codingAutosave")]
        coding_autosave: Option<bool>,
        #[serde(default, rename = "internetMode")]
        internet_mode: Option<String>,
        #[serde(default)]
        workdir: Option<Vec<String>>,
        #[serde(default, rename = "subagentMaxTurns")]
        subagent_max_turns: Option<u32>,
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
    SetEffort {
        effort: String,
    },

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
        /// Client request sequence for stale-reply protection. The host fills this
        /// before forwarding (mirrors `agentSaving.seq` from the JS store).
        #[serde(default, rename = "reqSeq")]
        req_seq: u64,
    },
    /// The /agents dashboard's delete (a file-backed agent; a built-in is a daemon-side
    /// error). `scope` is `"global"` / `"session"`. Forwarded like the config setters.
    DeleteAgent {
        scope: String,
        name: String,
        /// Client request sequence for stale-reply protection.
        #[serde(default)]
        req_seq: u64,
    },

    // ─── GUI OAuth surface (Codex / Kilo Code / xAI login) ───────────────────────
    /// The OAuth screen opened / refreshed: fetch the current state (connections +
    /// available providers). Dual-routed like `GetSettings`/`GetAgents` via
    /// [`forward_or_host`] — the attached daemon answers with `OAuthState`, or (un-attached)
    /// the host answers from `~/.koma/config.json` + the provider registry — so the screen
    /// populates in BOTH host states and never hangs.
    GetOAuthState,
    /// Start a login flow. `provider` is `"codex"` / `"kilocode"` / `"xai"` / `"claudeai"` /
    /// `"komarun"` / `"codex_paste"`. Dual-routed like `GetOAuthState` via [`forward_or_host`]
    /// — the attached daemon runs the flow on ITS runtime (unchanged); un-attached (the
    /// WELCOME/home screen) the host now runs the SAME flow on ITS OWN runtime
    /// (`HostCtl::StartOAuth`), so koma.run/provider sign-in works with no session attached.
    /// Either side streams progress back as `OAuthState` pushes (`waiting_url`/`waiting_code`
    /// → `success`/`failed`).
    StartOAuth {
        provider: String,
    },
    /// Complete the Codex paste-token flow with a raw access `token`. Forwarded as
    /// [`ClientRequest::SubmitOAuthPaste`], attached-only — the paste screen only ever
    /// follows an in-session `StartOAuth("codex_paste")`.
    SubmitOAuthPaste {
        token: String,
    },
    /// Cancel an in-flight login flow. Dual-routed like `StartOAuth` via [`forward_or_host`]
    /// — un-attached, aborts whatever host-local flow is in flight (a no-op if none) so the
    /// Cancel button in the Account section never dangles pre-session either.
    CancelOAuth,
    /// Delete a persisted OAuth connection by `uuid`. Dual-routed like `GetOAuthState` via
    /// [`forward_or_host`] — the attached daemon deletes + evicts, or (un-attached) the host
    /// removes it from the on-disk config + evicts the cache — then re-pushes a fresh
    /// `OAuthState`, so a connection is removable pre-session too.
    DeleteOAuthConn {
        uuid: String,
    },
    /// Open `url` in the SYSTEM browser (never inside the webview) — e.g. the Settings
    /// "Account" section's "Manage account on koma.run" link. HOST-LOCAL, unconditional
    /// (no session/attach needed): just spawns the OS opener via
    /// `service::oauth::browser::open_in_browser` and returns — fire-and-forget, no reply,
    /// no push.
    OpenExternal {
        url: String,
    },

    /// GUI Tutorial tab: one chat turn against the keyless koma-free endpoint.
    /// HOST-LOCAL (no daemon, works pre-session). `id` is a client-generated turn
    /// id echoed on `TutorialChatDone`. `messages` is the rolling transcript
    /// (user/assistant only — system prompt is host-owned).
    #[serde(rename_all = "camelCase")]
    TutorialChat {
        id: String,
        messages: Vec<TutorialChatMsg>,
    },

    // ─── GUI extension STORE surface (browse / install / uninstall) ──────────────
    // Browse/detail/installed-list are HOST-LOCAL — routed UNCONDITIONALLY to the
    // host-relay thread (`HostCtl::StoreBrowse` and friends), never the daemon,
    // regardless of attach state, same reasoning as `GitStatus`/`FileDiff`: koma.run
    // browse/detail is a PUBLIC (no-auth) network fetch and the installed list is a
    // local config read, both of which work identically pre-session (the Store tab
    // mounting on the home screen) as attached. Install/uninstall MUTATE live daemon
    // runtime state (the live MCP/ext managers + config), so those two stay forwarded
    // to the attached daemon; with no daemon attached the dispatcher pushes a graceful
    // `ExtensionOpResult{ok:false}` (`HostCtl::ExtNoSession`) instead of a silent no-op.
    /// Browse the store catalogue (optional `q` / `category` filters). Routed as
    /// `HostCtl::StoreBrowse`.
    StoreBrowse {
        query: Option<String>,
        category: Option<String>,
    },
    /// Fetch one extension's detail. Routed as `HostCtl::StoreDetail`.
    StoreDetail {
        id: String,
    },
    /// Install `id` (optional `version`). Forwarded as [`ClientRequest::InstallExtension`]
    /// to the attached daemon; with none attached, pushes a graceful
    /// `ExtensionOpResult{ok:false}` instead.
    InstallExtension {
        id: String,
        version: Option<String>,
    },
    /// Uninstall `id`. Same routing as [`InstallExtension`](Self::InstallExtension).
    UninstallExtension {
        id: String,
    },
    /// Fetch the locally-installed registry. Routed as `HostCtl::ListInstalledExtensions`.
    ListInstalledExtensions,
    /// Fetch full detail of one locally-installed extension. Routed as
    /// `HostCtl::GetInstalledExtensionDetail`.
    GetInstalledExtensionDetail {
        id: String,
    },

    // ─── GUI extension PANEL bridge (W8) ─────────────────────────────────────────
    /// A GUI extension PANEL's request to its backing extension daemon. The panel iframe
    /// (`koma://extension/<extId>/…`) posts this; the host forwards it to the attached daemon as
    /// [`ClientRequest::ExtPanelMsg`], which auto-starts the extension + invokes its `panel.msg`
    /// and answers OUT-OF-BAND with an `ExtPanelReply` push the host re-pushes. `reqId` correlates
    /// the reply; `payload` is the extension-defined request body. Attached-only, like `Interrupt`
    /// — with NO attached daemon it is dropped silently (the GUI-side guard replies locally in
    /// W9). camelCase fields to match the JS contract; `payload` defaults to `null` so a body-less
    /// request still decodes.
    ExtPanelMsg {
        #[serde(rename = "extId")]
        ext_id: String,
        #[serde(rename = "panelId")]
        panel_id: String,
        #[serde(rename = "reqId", default)]
        req_id: Option<String>,
        #[serde(default)]
        payload: serde_json::Value,
    },

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
    KeyGenerate {
        name: String,
        comment: String,
    },
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
    KeyReveal {
        name: String,
        private: bool,
    },
    /// Delete keypair `name` (both halves, best-effort). Forwarded as
    /// [`HostCtl::KeyDelete`]; same reply pattern as `KeyGenerate`.
    KeyDelete {
        name: String,
    },

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

    // ─── Coding panel: workspace file operations ─────────────────────────────
    // Serviced ENTIRELY host-side (the host process has direct filesystem access),
    // so every request here is routed UNCONDITIONALLY to the host-relay thread via
    // the matching `HostCtl::File*` variant, regardless of attach state.
    /// Coding panel: list a directory's immediate children.
    FileTree {
        root: String,
        path: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: read a text file's content.
    FileRead {
        root: String,
        path: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: save a text file. `expected_fingerprint` must match the disk
    /// state from the most recent FileRead; mismatch = conflict.
    FileSave {
        root: String,
        path: String,
        content: String,
        #[serde(rename = "expectedFingerprint")]
        expected_fingerprint: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: create a new file or directory.
    FileCreate {
        root: String,
        path: String,
        kind: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: rename/move within the same workspace root (v1).
    FileRename {
        root: String,
        #[serde(rename = "oldPath")]
        old_path: String,
        #[serde(rename = "newPath")]
        new_path: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: delete a file or directory (recursive for dirs).
    FileDelete {
        root: String,
        path: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: write raw bytes (drag-upload). `bytes_b64` is standard
    /// base64. When `overwrite` is false, existing paths are rejected.
    FileWriteBytes {
        root: String,
        path: String,
        #[serde(rename = "bytesB64")]
        bytes_b64: String,
        #[serde(default)]
        overwrite: bool,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: read raw bytes for download / save-as.
    /// `saveAs: true` asks the host to open a native save dialog and write the
    /// file (required in wry — blob `<a download>` is a no-op).
    FileDownloadBytes {
        root: String,
        path: String,
        #[serde(rename = "requestId")]
        request_id: String,
        /// Default false keeps preview loads (CodingFileViewer) byte-returning.
        #[serde(default, rename = "saveAs")]
        save_as: bool,
    },
    /// Coding panel: VS Code-style content search across a workspace root/subdir.
    FileContentSearch {
        root: String,
        /// Subdir relative to root (empty = whole root).
        #[serde(default)]
        path: String,
        query: String,
        #[serde(default, rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(default, rename = "wholeWord")]
        whole_word: bool,
        #[serde(default, rename = "isRegex")]
        is_regex: bool,
        #[serde(default, rename = "includeGlob")]
        include_glob: Option<String>,
        #[serde(default, rename = "excludeGlob")]
        exclude_glob: Option<String>,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Coding panel: replace-all with the same flags as FileContentSearch.
    FileContentReplace {
        root: String,
        #[serde(default)]
        path: String,
        query: String,
        replacement: String,
        #[serde(default, rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(default, rename = "wholeWord")]
        whole_word: bool,
        #[serde(default, rename = "isRegex")]
        is_regex: bool,
        #[serde(default, rename = "includeGlob")]
        include_glob: Option<String>,
        #[serde(default, rename = "excludeGlob")]
        exclude_glob: Option<String>,
        #[serde(rename = "requestId")]
        request_id: String,
    },

    // ─── Language servers (Settings + editor banner) ──────────────────────────
    // Host-local only (never the daemon): resolve / install / uninstall under
    // `~/.koma/lsp/`. Same reasoning as KeyList / FileTree.
    /// Settings "Language servers" opened / refreshed: full catalogue status.
    /// Reply: `LspStatus` envelope.
    LspStatus,
    /// Install one catalogue id, or every managed server when `all`.
    /// Streams `LspInstall` progress, then a fresh `LspStatus`.
    LspInstall {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        all: bool,
        #[serde(default)]
        force: bool,
    },
    /// Remove a koma-managed install. Never touches PATH copies.
    /// Reply: `LspInstall` ack + fresh `LspStatus`.
    LspUninstall {
        id: String,
    },
    // ─── Language client (Monaco providers + doc sync) ───────────────────────
    // Host-local only: LspManager owns stdio JSON-RPC to ~/.koma/lsp binaries.
    /// Coding tab open / content load: `textDocument/didOpen`.
    LspDidOpen {
        root: String,
        path: String,
        #[serde(rename = "languageId")]
        language_id: String,
        text: String,
    },
    /// Editor dirty change: full-document `textDocument/didChange`.
    LspDidChange {
        root: String,
        path: String,
        text: String,
    },
    /// After successful save: `textDocument/didSave`.
    LspDidSave {
        root: String,
        path: String,
        #[serde(default)]
        text: Option<String>,
    },
    /// Coding tab closed: `textDocument/didClose`.
    LspDidClose {
        root: String,
        path: String,
    },
    /// Monaco completion provider. Reply: `LspCompletion`.
    LspCompletion {
        root: String,
        path: String,
        line: u32,
        character: u32,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Monaco completionItem/resolve (auto-import additionalTextEdits).
    /// Reply: `LspCompletionResolve`.
    LspCompletionResolve {
        root: String,
        path: String,
        /// Boxed — CompletionItem carries opaque `data` JSON and edit lists.
        item: Box<crate::lsp::LspCompletionItem>,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Monaco hover provider. Reply: `LspHover`.
    LspHover {
        root: String,
        path: String,
        line: u32,
        character: u32,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Monaco definition provider. Reply: `LspDefinition`.
    LspDefinition {
        root: String,
        path: String,
        line: u32,
        character: u32,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Monaco references / CodeLens. Reply: `LspReferences`.
    LspReferences {
        root: String,
        path: String,
        line: u32,
        character: u32,
        #[serde(default, rename = "includeDeclaration")]
        include_declaration: bool,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Monaco CodeLens symbol anchors. Reply: `LspDocumentSymbol`.
    LspDocumentSymbol {
        root: String,
        path: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },

    // ─── Frontend error logging ───────────────────────────────────────────────
    /// Write an error message to the global error log (`~/.koma/error.log`).
    /// Used by the web frontend to log runtime errors that only occur in the
    /// built/running app (not in dev mode with full error messages). Host-local,
    /// unconditional — no reply, no attach needed.
    WriteErrorLog {
        message: String,
    },

    // ─── Remote host management ──────────────────────────────────────────────
    /// Fetch the saved remote hosts list. Always a reply.
    GetRemoteHosts,
    /// Add a new remote host. Reply: fresh RemoteHosts push.
    AddRemoteHost {
        name: String,
        user: String,
        host: String,
        port: u16,
        #[serde(default, rename = "keyPath")]
        key_path: Option<String>,
    },
    /// Edit an existing remote host by id. Reply: fresh RemoteHosts push.
    EditRemoteHost {
        id: String,
        name: String,
        user: String,
        host: String,
        port: u16,
        #[serde(default, rename = "keyPath")]
        key_path: Option<String>,
    },
    /// Delete a remote host by id. Reply: fresh RemoteHosts push.
    DeleteRemoteHost {
        id: String,
    },
    /// Connect to a remote host (placeholder — starts SSH session).
    ConnectRemoteHost {
        #[serde(rename = "hostId")]
        host_id: String,
    },
    /// Disconnect from a remote host (placeholder).
    DisconnectRemoteHost {
        #[serde(rename = "hostId")]
        host_id: String,
    },
    /// Submit a password for remote host authentication.
    SubmitRemotePassword {
        password: String,
    },
    /// Cancel an in-progress remote connection.
    CancelRemoteConnect,

    /// Open the GUI remote working-directory picker using retained SSH context.
    RequestRemotePath,
    /// List remote directories over SSH; never uses local filesystem APIs.
    ListRemotePath {
        path: String,
    },
    /// Confirm a remote working directory.
    ConfirmRemotePath {
        path: String,
    },
    /// Cancel the remote working-directory picker.
    CancelRemotePath,

    /// Open a second GUI process on the same remote session (multi-window attach).
    /// Spawns `koma gui remote user@host --session <id>` detached.
    OpenSecondWindow {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(default, rename = "hostId")]
        host_id: Option<String>,
    },

    /// Import-graph visualization request. `path` is the focal file (None = overview);
    /// `depth` is traversal depth (1–3, clamped); `direction` is
    /// `"dependencies"`/`"dependents"`/`"both"`.
    #[cfg(feature = "linker")]
    ImportGraph {
        path: Option<String>,
        #[serde(default)]
        depth: Option<u32>,
        #[serde(default)]
        direction: Option<String>,
        #[serde(default, rename = "filterRoots")]
        filter_roots: Option<Vec<String>>,
        #[serde(default, rename = "filterLanguages")]
        filter_languages: Option<Vec<String>>,
        #[serde(default, rename = "requestId")]
        request_id: Option<String>,
    },
    /// Impact analysis: transitive reverse deps for a file (depth-capped).
    #[cfg(feature = "linker")]
    ImportGraphImpact {
        path: String,
        #[serde(default)]
        depth: Option<u32>,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    /// Reindex configured workspaces: reconcile/register, rescan, poll
    /// until the scan completes, then refresh the scoped visualization.
    /// The host handles this entirely off-thread so the event loop is never
    /// blocked.  `request_id` is echoed back in the `ImportGraphResult`
    /// so the GUI can correlate and reject stale replies.
    #[cfg(feature = "linker")]
    ImportGraphReindex {
        #[serde(default, rename = "requestId")]
        request_id: Option<String>,
    },

    // ─── GUI terminal view ──────────────────────────────────────────────
    /// Create a new interactive terminal session (PTY) at the given working
    /// directory. The host manages the PTY lifecycle and streams output back
    /// as TerminalOutput/TerminalExit push envelopes. `id` is a client-minted
    /// stable identifier (the tab's terminal ID); `cwd` is the starting
    /// directory (defaults to the session's workdir when absent).
    TerminalCreate {
        id: String,
        #[serde(default, rename = "cwd")]
        cwd: Option<String>,
    },
    /// Forward keystroke data from xterm.js to the PTY's stdin. `id` is the
    /// terminal session id from TerminalCreate.
    TerminalInput {
        id: String,
        data: String,
    },
    /// Resize the PTY's viewport. Sent by xterm.js's fit addon when the
    /// terminal container changes size. `id` is the terminal session id.
    TerminalResize {
        id: String,
        cols: u16,
        rows: u16,
    },
    /// Kill a terminal session's PTY and clean up. `id` is the terminal
    /// session id.
    TerminalKill {
        id: String,
    },
}

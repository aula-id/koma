//! [`PushEnvelope`] + the one-shot `push_*` emit helpers for the GUI
//! push-envelope bridge — the Rust half of the `window.__komaClient.push` JSON
//! contract (`#[serde(tag = "k")]` names each envelope, matching the JS `push`
//! dispatcher's `k` switch EXACTLY). Split out of `render.rs` originally (pure
//! code motion, no behaviour change), then split AGAIN into `push_rows.rs`
//! (same reason): every `Push*` ROW/DTO struct `PushEnvelope`'s variants carry
//! now lives there, re-exported below so importers keep using
//! `super::push_proto::PushX` unchanged. Split a THIRD time (same reason) into
//! `push_proto_git.rs`: the GIT-domain (+ SSH-key-vault) `push_*` helpers live
//! there instead — `PushEnvelope` can't split (one enum), so it + the
//! remaining non-git helpers stay here.
//!
//! `PushEnvelope` stays `pub(super)` (struct-crossing-a-sibling-module reach,
//! same as every row struct). `emit` stays in `render.rs` (its callers span
//! both this file and `project.rs`), referenced here as `super::render::emit`.

pub(super) use super::push_rows::{
    PushAnalyticsModel, PushAnalyticsSeriesPoint, PushAttachment, PushBashJob, PushCooking,
    PushFileChange, PushFileTreeEntry, PushHistory, PushMcpServer, PushMcpStatusServer, PushModel,
    PushMsg, PushPalette, PushPaletteInfo, PushPendingCall, PushPlanTodo, PushProvider, PushRoute,
    PushSubAgent, PushToolCall, PushUsageDay, PushUsageModel,
};

/// Impact analysis result: paths that transitively depend on a file.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphImpactResult {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub path: String,
    pub depth: u32,
    pub paths: Vec<String>,
    pub total: usize,
    pub error: Option<String>,
}

/// The daemon->JS envelope, tagged on `k`. One variant per bridge message; every
/// field name matches the contract verbatim (camelCase where the contract uses it).
#[derive(serde::Serialize)]
#[serde(tag = "k")]
pub(super) enum PushEnvelope {
    /// Structural / commit tick (the catch-all): the full committed transcript +
    /// title + palette for `session`. `state` is always `"attached"`.
    Snapshot {
        session: String,
        state: &'static str,
        messages: Vec<PushMsg>,
        title: String,
        palette: PushPalette,
        /// Foreground session's sub-agents (list + status). Authoritative full array —
        /// React REPLACES on each Snapshot, never accumulates.
        subagents: Vec<PushSubAgent>,
        /// Foreground session's background-bash jobs (list + status). Authoritative
        /// full array — React REPLACES on each Snapshot, never accumulates.
        bash: Vec<PushBashJob>,
        /// Foreground session's cumulative file-change log (#24). Authoritative full
        /// array — React REPLACES on each Snapshot. Empty when nothing was touched.
        #[serde(rename = "fileChanges")]
        file_changes: Vec<PushFileChange>,
        /// Foreground session's Plan-mode todo checklist (Explore "PLAN" section).
        /// Authoritative full array — React REPLACES on each Snapshot; empty when
        /// not in Plan mode or no plan is in progress (the section hides/dims).
        #[serde(rename = "planTodos")]
        plan_todos: Vec<PushPlanTodo>,
        /// Foreground session's STAGED composer attachments (chips). Authoritative full
        /// array — React REPLACES on each Snapshot; empty once the message is sent.
        attachments: Vec<PushAttachment>,
        /// The current GLOBAL agent mode label (`"auto"`/`"normal"`/`"plan"`/`"yolo"`),
        /// decoded from the snapshot into the shadow. Rides the Snapshot (folded into its
        /// fingerprint) so the composer mode selector shows + reflects the live mode; a
        /// `SetMode` round-trip flips this on the next snapshot.
        mode: String,
        /// Foreground session's QUEUED mid-turn steer previews (koma's `pending_steer`).
        /// Authoritative full array — React REPLACES on each Snapshot; the composer
        /// renders it as the "Queued N/5" preview list while a turn is in flight.
        /// Folded into the fingerprint so queuing/consuming a steer re-emits the Snapshot.
        #[serde(rename = "pendingSteer")]
        pending_steer: Vec<String>,
        /// Foreground session's tool-approval GATE (wave-7): `true` when a risky/classifier
        /// call OR a `plan_ready` plan digest has PARKED and the daemon is blocked waiting on
        /// a decision the GUI must surface. React raises the approval overlay when set; a
        /// `GuiReq::ApproveTool` (tool card) or `GuiReq::PlanDecision` (plan card) answers it.
        /// Folded into the fingerprint so a park/resume re-emits the Snapshot on its own.
        #[serde(rename = "awaitingApproval")]
        awaiting_approval: bool,
        /// The classifier's "why" for a paused risky call — `None` for a `plan_ready` pause
        /// or a non-classifier gate. Shown as the reason line on the approval card.
        #[serde(rename = "approvalReason")]
        approval_reason: Option<String>,
        /// The actual paused call (name + args); `Some` only while `awaitingApproval`. React
        /// branches on `pendingCall.name == "plan_ready"` (plan controls) vs any other name
        /// (tool approve/deny card).
        #[serde(rename = "pendingCall")]
        pending_call: Option<PushPendingCall>,
        // ─── SDLC projection (mode=sdlc only; None/omitted otherwise) ───
        /// SDLC phase when mode is sdlc. Cleared on mode switch.
        #[serde(rename = "sdlcPhase", skip_serializing_if = "Option::is_none")]
        sdlc_phase: Option<String>,
        /// Approved mission goal. Cleared on mode switch.
        #[serde(rename = "sdlcGoal", skip_serializing_if = "Option::is_none")]
        sdlc_goal: Option<String>,
        /// Mission branch. Cleared on mode switch.
        #[serde(rename = "sdlcBranch", skip_serializing_if = "Option::is_none")]
        sdlc_branch: Option<String>,
        /// Open graph node count. Cleared on mode switch.
        #[serde(rename = "sdlcOpen", skip_serializing_if = "Option::is_none")]
        sdlc_open: Option<usize>,
        /// Sealed graph node count. Cleared on mode switch.
        #[serde(rename = "sdlcSealed", skip_serializing_if = "Option::is_none")]
        sdlc_sealed: Option<usize>,
    },
    /// Swap-START signal: the host is about to tear down the current attach and connect
    /// a different (or freshly minted) session. `to` is the target session id/uuid — the
    /// authoritative identifier React maps to a friendly hub label (an unknown id, e.g. a
    /// brand-new session, has no row yet, so the overlay falls back to a generic label).
    /// Pushed the instant a `Select`/`New` is acted on, BEFORE teardown, so React can
    /// raise a deterministic full-screen loader across the (uninterruptible, possibly
    /// multi-second build-skew) attach gap during which NOTHING else is pushed. Cleared
    /// naturally by the next `Snapshot { state:"attached" }` — whose `session` equals `to`
    /// on a resolved swap.
    Switching { to: String },
    /// The FULL live streaming buffer (React REPLACES the live bubble). Emitted every
    /// frame the buffer changes; an empty `text` clears the bubble on commit.
    StreamMsg { session: String, text: String },
    /// The FULL live reasoning buffer (React REPLACES). Empty `text` clears it.
    Reasoning { session: String, text: String },
    /// Working flag + optional toast. React animates the spinner locally; the host
    /// only says whether the session is working and what toast (if any) to show.
    /// `toastKind` carries the toast SEVERITY (`"error"` / `"info"`) so React can colour
    /// a safeguard/harness block red vs an informational notice neutrally — the daemon's
    /// `ToastKind` (the third field of `fg.toast`) is otherwise dropped on the wire.
    Status {
        session: String,
        working: bool,
        toast: Option<String>,
        #[serde(rename = "toastKind")]
        toast_kind: Option<&'static str>,
        /// Foreground session's cumulative token/cost counters (mirrors the daemon's
        /// `SessionRuntime` totals — see `client_shadow/session.rs`'s rehydration),
        /// for the GUI status footer. `tokensIn` is the CURRENT context size (not a
        /// running sum — mirrors `SessionRuntime::tokens_in`'s own semantics);
        /// `tokensOut`/`tokensCached`/`cost` accumulate across the session.
        #[serde(rename = "tokensIn")]
        tokens_in: u64,
        #[serde(rename = "tokensCached")]
        tokens_cached: u64,
        #[serde(rename = "tokensOut")]
        tokens_out: u64,
        cost: f64,
        /// The EFFECTIVE agent mode label (`"auto"`/`"normal"`/`"plan"`/`"yolo"`) —
        /// identical source to the Snapshot's `mode` (`shadow.rest.agent_mode()`), so
        /// e.g. a model-driven `plan_enter` while the selector reads "auto" still
        /// reports `"plan"` here the instant the round-trip lands (Status re-emits
        /// independently of the Snapshot fingerprint). Rides the status footer so it
        /// updates even on ticks where the transcript itself doesn't change.
        mode: String,
    },
    /// The detached session swapper: the `[+ new session]` row + live cooking rows +
    /// on-disk history. `state` is always `"swapper"`.
    Hub {
        state: &'static str,
        cooking: Vec<PushCooking>,
        history: Vec<PushHistory>,
    },
    /// One-shot omnisearch results for `query` — the GUI overlay REPLACES its list with
    /// `items` (each `{ path, label }`; an empty `path` marks a non-attachable dir row).
    /// Pushed out-of-band (not fingerprinted) whenever the daemon answers a `FileSearch`;
    /// `query` is echoed so the overlay can drop a stale/out-of-order reply.
    SearchResults {
        query: String,
        items: Vec<crate::ipc::proto::FileSearchItem>,
    },
    /// The authoritative GLOBAL config catalogue for the Connector + MCP panels. React
    /// REPLACES its config slices on each push. Emitted whenever the projected config
    /// changes (a full snapshot carries it) and re-emitted on `Ready` (page reload) so a
    /// fresh webview always has the current catalogue. `models` folds the global scope
    /// and the foreground session's local-override scope into one list, each row tagged
    /// with its `scope`.
    Config {
        providers: Vec<PushProvider>,
        models: Vec<PushModel>,
        mcp: Vec<PushMcpServer>,
        /// The active palette (theme), so the EMPTY/swapper state — which never receives a
        /// `Snapshot` (the only other palette carrier) — can repaint to `config.json`'s
        /// theme instead of the hardcoded dark default. Rides Config because Config is
        /// pushed in BOTH host states (swapper via `push_swapper_config`, attached via
        /// `push_config`), so the theme is always available.
        palette: PushPalette,
        /// The uuid of the foreground session's LOCAL Main-role model override, or `null`
        /// when there is none (the session inherits the global Main). The model
        /// quick-picker uses this as its current selection: `null` = the `(inherit)` row,
        /// else the matching local-override option. `None` is serialised as JSON `null`.
        #[serde(rename = "sessionMainUuid")]
        session_main_uuid: Option<String>,
        /// The available theme registry names (`view::theme::PALETTES` keys), for the GUI
        /// onboarding theme step + the future Settings gear picker. Static + identical in
        /// both host states, but rides `Config` so the picker always has the list without a
        /// dedicated envelope. React renders one option per name; a pick round-trips as
        /// `SetTheme { name }`.
        themes: Vec<&'static str>,
        /// The ACTIVE palette (theme) registry key (`config.palette`), so the GUI can
        /// highlight the current card in the Settings Appearance grid + the onboarding
        /// theme picker. Re-pushed on every theme change (Config rides the palette diff), so
        /// the highlight tracks live. Serialised as `theme` to match the React store's
        /// `ConfigSlice.theme`.
        theme: String,
        /// The FULL palette catalogue with resolved colours (one [`PushPaletteInfo`] per
        /// `themes` entry, same order), for the GUI Settings tab's Appearance grid — each a
        /// name + its 11 role colours as `#rrggbb`. Kept ALONGSIDE `themes` (names only) so
        /// the onboarding theme step stays backward-compatible; the Settings grid consumes
        /// this richer list. Static (theme registry is compile-time), but rides `Config` so
        /// the picker always has it. A pick round-trips as `SetTheme { name }`.
        palettes: Vec<PushPaletteInfo>,
        /// FIRST-RUN flag (wave-3+4 A/B): `true` when no usable Main route is configured
        /// (no providers, or no global/local Main-role model bound to a known provider) —
        /// i.e. an empty/first-run `~/.koma/config.json`. React shows the full-screen
        /// onboarding flow (theme → connection) instead of the normal start screen when
        /// this is set; it clears the instant a provider + Main model land.
        ///
        /// Serialised as `firstRun` to match the React store's authoritative Config
        /// envelope contract (`ConfigSlice.firstRun`) — same boolean semantics
        /// (`true` = show onboarding).
        #[serde(rename = "firstRun")]
        needs_onboarding: bool,
    },
    /// One-shot live model-id catalogue for `provider` (uuid), answering a
    /// `ListModels` — the Connector ModelForm REPLACES its model-id picker options with
    /// `models`. Pushed out-of-band (not fingerprinted) whenever the daemon answers a
    /// `ListModels`; `provider` is echoed so the form can drop a stale/out-of-order reply.
    ModelList {
        provider: String,
        models: Vec<String>,
    },
    /// One-shot live provider-route list for `modelId` under `provider` (uuid), answering a
    /// `ListRoutes` — the Connector ModelForm REPLACES its ROUTE picker options with the
    /// model's REAL routes (`routes`, each a `PushRoute`) instead of the hardcoded demo
    /// list. Pushed out-of-band (not fingerprinted) whenever the daemon answers a
    /// `ListRoutes`; `provider`/`modelId` are echoed so the form can drop a stale reply (a
    /// provider/model-id change refetches). An EMPTY `routes` means a non-OpenRouter
    /// provider or a failed fetch — the form shows only its synthetic "Auto" default.
    #[serde(rename_all = "camelCase")]
    RouteList {
        provider: String,
        model_id: String,
        routes: Vec<PushRoute>,
    },
    /// One-shot host-computed FILE DIFF answering a `FileDiff` request from the Explore
    /// "FILE CHANGED" panel (opening a Monaco diff tab): `original` is `git show
    /// HEAD:<path>` (empty for a new/untracked file — a valid all-added diff);
    /// `modified` is the current on-disk contents (empty when the file was deleted).
    /// `error` is set (both strings then empty) when the diff couldn't be computed at
    /// all (no git repository, or either side over the size cap); `binary` is set
    /// (both strings then empty, no `error`) when either side isn't valid UTF-8 text.
    /// Computed ENTIRELY host-side (see `compute_file_diff` — never forwarded to the
    /// daemon), so this is pushed the SAME way regardless of attach state, and — like
    /// `ModelList`/`RouteList` — is ALWAYS a reply so the diff tab never hangs.
    /// `origin` says where the ORIGINAL side came from: `"git"` (`git show HEAD:`) or
    /// `"baseline"` (the session's "virtual git" first-touch pre-image — non-git
    /// directories); the GUI shows a dim "session baseline" badge for the latter.
    FileDiff {
        path: String,
        original: String,
        modified: String,
        error: Option<String>,
        binary: bool,
        origin: String,
    },
    /// One-shot host-computed GIT STATUS answering a `GitStatus` request from the
    /// Explore "GIT" panel: branch (+ ahead/behind, detached flag) and the staged /
    /// unstaged file lists. Computed ENTIRELY host-side (see `compute_git_status` —
    /// never forwarded to the daemon), so this is pushed the SAME way regardless of
    /// attach state, and — like `FileDiff` — is ALWAYS a reply so the panel never
    /// hangs loading. Carries [`super::git::GitStatusResult`] verbatim (it is already
    /// `Serialize`, camelCase).
    GitStatus(super::git::GitStatusResult),
    /// One-shot host-computed GIT DIFF answering a `GitDiff` request from the GIT
    /// panel's file-row click (opening a Monaco diff tab): `staged` selects
    /// index-vs-HEAD or worktree-vs-index. Computed ENTIRELY host-side (see
    /// `compute_git_diff` — never forwarded to the daemon), so this is pushed the SAME
    /// way regardless of attach state, and — like `FileDiff` — is ALWAYS a reply so
    /// the diff tab never hangs. Carries [`super::git::GitDiffResult`] verbatim.
    GitDiff(super::git::GitDiffResult),
    /// One-shot host-computed GIT OP result answering a `GitStage`/`GitUnstage`/
    /// `GitDiscard`/`GitCommit` mutation from the Source Control "GIT" panel. `op`
    /// (`"stage"`/`"unstage"`/`"discard"`/`"commit"`) lets React branch per-kind (e.g.
    /// clear the commit box only on a successful commit); `error` (set only when `ok`
    /// is `false`) surfaces the failure as a toast. Carries NO list data itself — it is
    /// ALWAYS immediately followed by a fresh `GitStatus` push (the mutation worker
    /// computes + pushes that right after), which is what actually refreshes the
    /// panel's staged/unstaged lists. Carries [`super::git::GitOpResult`] verbatim (it
    /// is already `Serialize`, camelCase).
    GitOp(super::git::GitOpResult),
    /// One-shot host-computed COMMIT GRAPH answering a `GitGraph` request from the
    /// GitKraken-style commit-graph panel: `commits` (parents + refs per node), the
    /// current `head` sha, and `hasMore` (a full page likely means more history exists
    /// — a scroll-load-more hint). Computed ENTIRELY host-side (see
    /// `git_graph::compute_git_graph` — never forwarded to the daemon), so this is
    /// pushed the SAME way regardless of attach state, and — like `GitStatus` — is
    /// ALWAYS a reply so the panel never hangs loading. Carries
    /// [`super::git_graph::GitGraphResult`] verbatim (already camelCase).
    GitGraph(super::git_graph::GitGraphResult),
    /// One-shot host-computed COMMIT DETAIL answering a `GitCommitDetail` request (a
    /// commit-graph row click): full metadata (incl. body) + the changed-file list
    /// (first-parent view for a merge commit). Computed ENTIRELY host-side (see
    /// `git_graph::compute_commit_detail`), pushed the SAME way regardless of attach
    /// state, ALWAYS a reply. Carries [`super::git_graph::CommitDetailResult`] verbatim.
    CommitDetail(super::git_graph::CommitDetailResult),
    /// One-shot host-computed COMMIT DIFF answering a `GitCommitDiff` request (a
    /// commit-detail file-row click): `path` at commit `sha` vs its first parent,
    /// opening a Monaco diff tab. SEPARATE envelope from `GitDiff` (working-tree/index
    /// diff) so the GUI can route a commit-history diff to its own tab id without
    /// collision. Computed ENTIRELY host-side (see `git_graph::compute_commit_diff`),
    /// pushed the SAME way regardless of attach state, ALWAYS a reply. Carries
    /// [`super::git_graph::CommitDiffResult`] verbatim.
    CommitDiff(super::git_graph::CommitDiffResult),
    /// One-shot host-computed per-commit ACTIVITY answering a `GitActivity` request
    /// from the bubble/activity chart (GK5a): author/date/lines-changed for each
    /// commit on `HEAD`. Computed ENTIRELY host-side (see
    /// `git_activity::compute_git_activity`), pushed the SAME way regardless of
    /// attach state, ALWAYS a reply. Carries [`super::git_activity::ActivityResult`]
    /// verbatim (already camelCase).
    Activity(super::git_activity::ActivityResult),
    /// One-shot host-computed LAST-7-DAYS usage preview answering a `UsagePreview`
    /// request from the activity-bar Usage panel: aggregate totals, a 7-entry daily cost
    /// series (oldest first, today last — zero-filled for days with no ledger rows), and
    /// the top 3 models by spend in the window. Computed ENTIRELY host-side straight off
    /// the global `~/.koma/usage.sqlite` ledger (see `compute_usage_preview` — never
    /// forwarded to the daemon), so this is pushed the SAME way regardless of attach
    /// state, and — like `FileDiff` — is ALWAYS a reply so the panel never hangs loading.
    /// `scope` echoes the request's `"all"`/`"session"` token, and `session_id`
    /// (serialised `sessionId`) echoes the session uuid ACTUALLY queried (`None` for an
    /// "all" scope) — together they let React drop a reply that no longer matches
    /// what's currently selected/attached: a rapid all/session toggle racing an
    /// in-flight request (scope mismatch), OR the foreground session switching mid-flight
    /// while "session" scope stayed selected (session id mismatch) — the latter is what
    /// would otherwise render session A's numbers under session B's attach.
    #[serde(rename_all = "camelCase")]
    UsagePreview {
        cost: f64,
        tokens_in: u64,
        tokens_cached: u64,
        tokens_out: u64,
        calls: u64,
        days: Vec<PushUsageDay>,
        top_models: Vec<PushUsageModel>,
        scope: String,
        session_id: Option<String>,
    },
    /// One-shot host-computed Analytics dashboard answering a `Analytics`
    /// request from the main Analytics tab: KPI totals (cost/calls/token
    /// breakdown + cache rate), a zero-filled time series, per-model table, and
    /// main-vs-sub-agent role split. Computed ENTIRELY host-side straight off the
    /// global `~/.koma/usage.sqlite` ledger (see `compute_analytics` — never
    /// forwarded to the daemon), so this is pushed the SAME way regardless of
    /// attach state, and — like `UsagePreview` — is ALWAYS a reply so the tab
    /// never hangs loading.
    ///
    /// Correlation fields (`req_seq`, `scope`, `session_id`, `range`, `metric`)
    /// echo the request so React can drop a stale reply across rapid filter /
    /// session changes. `status` is `"ok"` / `"empty"` / `"error"` — a successful
    /// zero-call window is `"empty"` (not an error); `"error"` carries a
    /// human-readable `error` string.
    #[serde(rename_all = "camelCase")]
    Analytics {
        req_seq: u64,
        scope: String,
        session_id: Option<String>,
        range: String,
        metric: String,
        status: String,
        error: Option<String>,
        cost: f64,
        tokens_in: u64,
        tokens_cached: u64,
        tokens_out: u64,
        calls: u64,
        cache_rate: f64,
        series: Vec<PushAnalyticsSeriesPoint>,
        models: Vec<PushAnalyticsModel>,
        main_cost: f64,
        main_calls: u64,
        sub_cost: f64,
        sub_calls: u64,
    },
    /// One-shot reply to a `GetSettings` (and the re-push after a `SetSessionPrefs`),
    /// carrying the foreground session's GUI-editable prefs + the active palette for the
    /// GUI Settings tab's Session section. Pushed out-of-band (not fingerprinted) whenever
    /// the daemon answers a `GetSettings` — or, un-attached, straight from the swapper's
    /// global-config fallback. `internetMode` is `"simple"`/`"full"`. ALWAYS a reply so the
    /// tab's loading state can never hang.
    #[serde(rename_all = "camelCase")]
    SettingsValues {
        name: String,
        workdir: Vec<String>,
        short_send: bool,
        sliding_cache: bool,
        bash_saving: bool,
        coding_autosave: bool,
        internet_mode: String,
        palette: String,
        /// The foreground session's stored `/effort` value (`""` = model
        /// default), for the composer's effort-picker trigger-pill label.
        effort: String,
        /// Max agentic turns per sub-agent (user-editable, ≥ 1).
        subagent_max_turns: u32,
    },
    /// One-shot reply to a `GetAgents` (and the re-push after a `SetAgent` / `DeleteAgent`):
    /// the merged sub-agent registry + model / provider catalogue for the GUI /agents
    /// dashboard. `agents` is the full roster (built-in + global + session), each entry a
    /// [`crate::ipc::proto::AgentEntry`] serialised with ITS OWN snake_case fields
    /// (`name`/`description`/`conditions`/`source`/`model_uuid`/`model`/`tools`/`prompt`);
    /// `catalogueModels` / `catalogueProviders` are the editor's keyless catalogue
    /// ([`crate::ipc::proto::CatalogueModelSnapshot`] — `uuid`/`name`/`model_id`/
    /// `provider_uuid`; [`crate::ipc::proto::CatalogueProviderSnapshot`] — `uuid`/`name`/
    /// `endpoint`). `availableTools` is the editor tool-picker's selectable tool-name list
    /// (a plain string array, registry order), the SAME set the TUI picker shows. Pushed
    /// out-of-band (not fingerprinted) whenever the daemon answers a `ListAgents` — or,
    /// un-attached, straight from the host's `load_registry(None)` + global-config fallback.
    /// ALWAYS a reply so the dashboard's loading state can never hang.
    #[serde(rename_all = "camelCase")]
    AgentsValues {
        /// Request-sequence echoed from the agent mutation for stale-reply protection.
        /// 0 = no correlation (read-only fetch or host-built fallback).
        req_seq: u64,
        agents: Vec<crate::ipc::proto::AgentEntry>,
        catalogue_models: Vec<crate::ipc::proto::CatalogueModelSnapshot>,
        catalogue_providers: Vec<crate::ipc::proto::CatalogueProviderSnapshot>,
        available_tools: Vec<String>,
    },
    /// The streaming GUI OAuth surface's authoritative state — the one-shot reply to a
    /// `GetOAuthState` / `DeleteOAuthConn` / `CancelOAuth` / `SubmitOAuthPaste`, AND the
    /// streamed transitions of an in-flight `StartOAuth` (`starting` → `waiting_url` /
    /// `waiting_code` → `success` / `failed`). Pushed out-of-band (not fingerprinted)
    /// whenever the daemon emits a `DaemonEvent::OAuthState` — or, un-attached, straight from
    /// the host's config + provider-registry fallback. `phase` is a data value token:
    /// `"idle"` / `"starting"` / `"waiting_url"` / `"waiting_code"` / `"paste"` / `"success"`
    /// / `"failed"`. `url` (Codex auth URL) / `userCode` + `verificationUrl` (Kilo device) /
    /// `error` (failure reason) are set per phase (the envelope KEYS are camelCase; the phase
    /// VALUE stays as-is). `conns` is the TOKENLESS connection list (`OAuthConnWire` — never an
    /// access/refresh/id token) and `providers` the available-provider catalogue
    /// (`OAuthProviderWire`); React REPLACES its OAuth slice on each push. ALWAYS a reply so
    /// the OAuth screen's loading state can never hang.
    #[serde(rename_all = "camelCase")]
    OAuthState {
        phase: String,
        url: Option<String>,
        user_code: Option<String>,
        verification_url: Option<String>,
        error: Option<String>,
        conns: Vec<crate::ipc::proto::OAuthConnWire>,
        providers: Vec<crate::ipc::proto::OAuthProviderWire>,
    },
    /// The GUI extension-STORE catalogue — the one-shot reply to a `StoreBrowse`. `error` is
    /// set (and `items` empty) on a store network/parse failure so the grid renders an error
    /// state rather than hanging. Re-pushed from the intercepted `DaemonEvent::StoreCatalogue`;
    /// the nested `StoreItemWire`s carry their own camelCase keys.
    #[serde(rename_all = "camelCase")]
    StoreCatalogue {
        items: Vec<crate::ipc::proto::StoreItemWire>,
        error: Option<String>,
    },
    /// One extension's full detail — the reply to a `StoreDetail`. `detail` is `null` (and
    /// `error` set) when the fetch failed / the id was unknown.
    #[serde(rename_all = "camelCase")]
    StoreItemDetail {
        detail: Option<crate::ipc::proto::StoreDetailWire>,
        error: Option<String>,
    },
    /// The locally-installed extension registry — the reply to `ListInstalledExtensions` and
    /// the re-push after a successful install/uninstall.
    #[serde(rename_all = "camelCase")]
    InstalledExtensions {
        items: Vec<crate::ipc::proto::InstalledExtWire>,
    },
    /// The ok/error result of an install/uninstall op (echoing `id` so the GUI clears that
    /// card's pending spinner). On success the authoritative registry reply is the following
    /// `InstalledExtensions` push; this carries the status + any failure message.
    #[serde(rename_all = "camelCase")]
    ExtensionOpResult {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    /// Full detail of one locally-installed extension — the reply to
    /// `GetInstalledExtensionDetail`. `detail` is `null` when the extension is not
    /// in the registry. `id` echoes the requested extension id for stale-reply protection.
    #[serde(rename_all = "camelCase")]
    InstalledExtensionDetail {
        id: String,
        detail: Option<crate::ipc::proto::InstalledExtensionDetailWire>,
        error: Option<String>,
    },
    /// Out-of-band reply to a `ExtPanelMsg` (W8 panel bridge) — the extension's `panel.msg`
    /// invoke outcome, re-pushed from the intercepted `DaemonEvent::ExtPanelReply` so the panel
    /// iframe can correlate it by `reqId` and resolve its pending request. `ok`/`payload`/`error`
    /// carry the result (an unavailable / disabled / oneshot extension, a failed auto-start, or a
    /// timed-out/failed invoke is `ok:false` + `error`). camelCase keys per the JS contract.
    #[serde(rename_all = "camelCase")]
    ExtPanelReply {
        ext_id: String,
        panel_id: String,
        req_id: Option<String>,
        ok: bool,
        payload: Option<serde_json::Value>,
        error: Option<String>,
    },
    /// Unsolicited daemon→panel push (W8 panel bridge) — re-pushed from the intercepted
    /// `DaemonEvent::ExtPanelPush` so a panel iframe's live UI updates without a request.
    /// camelCase keys per the JS contract.
    #[serde(rename_all = "camelCase")]
    ExtPanelPush {
        ext_id: String,
        panel_id: String,
        payload: serde_json::Value,
    },
    /// One-shot reply to a `GetEffortOptions`: the derived `/effort` menu for the
    /// foreground session's current model. `state` is `"loading"` (a catalogue
    /// fetch was just armed or is already in flight — `options` empty),
    /// `"unsupported"` (the model has no reasoning control, or there's no active
    /// session — `options` empty), or `"ready"` (`options`/`selected` populated).
    /// `note` carries the human-readable reason/hint in every state. Pushed
    /// out-of-band (not fingerprinted) whenever the daemon answers a
    /// `GetEffortOptions` — ALWAYS a reply so the picker never hangs.
    #[serde(rename_all = "camelCase")]
    EffortOptions {
        options: Vec<String>,
        selected: usize,
        note: String,
        state: String,
    },
    /// The TUI's animated startup splash ([`crate::app::mode::Mode::Loading`]),
    /// projected so the GUI can render its own loading overlay while a returning
    /// session warms asynchronously (workspace reindex + project-docs awareness
    /// summary). `workspace`/`awareness` are one of `"pending"`/`"running"`/
    /// `"done"`/`"skipped"`/`"failed"` (mirrors [`crate::app::mode::WarmStatus`],
    /// dropping `Done`'s carried detail string — the webview shows a generic
    /// "ready" glyph, not the TUI's dim detail text). `active` is `true` while the
    /// foreground session's mode is `Loading`; pushed exactly ONCE more with
    /// `active: false` (`workspace`/`awareness` both `"done"`) the frame the mode
    /// leaves `Loading`, then never again until the next warm cycle — see
    /// `serialize_and_push`'s dedup comment for why the webview can rely on that
    /// single terminal `false` frame to dismiss its overlay.
    Loading {
        active: bool,
        workspace: String,
        awareness: String,
    },
    /// One-shot host-computed SSH KEY LIST answering a `KeyList` request from the
    /// Settings "SSH Keys" section: every keypair currently in the vault
    /// (`<~/.koma>/keys/`). Computed ENTIRELY host-side (see `keys::list_keys` —
    /// never forwarded to the daemon; this is a GUI-only, manual, user-owned key
    /// vault, separate from the model's own git credential machinery), so this is
    /// pushed the SAME way regardless of attach state, and — like `GitStatus` — is
    /// ALWAYS a reply so the section never hangs loading (an empty vault is itself
    /// a valid "no keys yet" state). Also pushed as the follow-up refresh after
    /// any `KeyOp` mutation. A named `keys` field (not a bare newtype) since an
    /// internally-tagged enum (`tag = "k"`) can't carry a top-level array.
    KeyList { keys: Vec<super::keys::KeyInfo> },
    /// One-shot host-computed SSH KEY REVEAL answering a `KeyReveal` request (the
    /// "Copy public key" / "Reveal private key" actions). Carries
    /// [`super::keys::KeyRevealResult`] verbatim (already camelCase) — `private`
    /// echoes the request so React applies the reply to the right affordance;
    /// `error` set means `content` is empty.
    KeyReveal(super::keys::KeyRevealResult),
    /// One-shot host-computed SSH KEY OP result answering a `KeyGenerate`/
    /// `KeyImport`/`KeyDelete` mutation from the Settings "SSH Keys" section. `op`
    /// (`"generate"`/`"import"`/`"delete"`) lets React branch per-kind; `error`
    /// (set only when `ok` is `false`) surfaces the failure as a toast. Carries NO
    /// list data itself — ALWAYS immediately followed by a fresh `KeyList` push
    /// (the mutation worker computes + pushes that right after), mirroring
    /// `GitOp`/`GitStatus`. Carries [`super::keys::KeyOpResult`] verbatim (already
    /// camelCase).
    KeyOp(super::keys::KeyOpResult),
    /// One-shot daemon agent create/edit/delete result, re-pushed to the GUI so
    /// failed agent mutations surface as error toasts (the attached daemon path
    /// sends `DaemonEvent::AgentOp` for failures, which is otherwise silently
    /// consumed by the shadow). On success the AUTHORITATIVE reply is always a
    /// fresh `AgentsValues` push (re-pushed by the existing intercept below), so
    /// this envelope only carries `ok: false` with a human-readable `error` — the
    /// GUI uses it to clear its saving state and show the failure toast. `req_seq`
    /// echoes the client request sequence for stale-reply protection (0 = no
    /// correlation, used by the generic `DaemonEvent::Error` fallback).
    #[serde(rename_all = "camelCase")]
    AgentOp {
        ok: bool,
        error: Option<String>,
        req_seq: u64,
    },
    /// One-shot reply to a `GetMcpStatus` request — per-server live connection state
    /// (tool counts + errors) plus an optional top-level availability error. Echoes
    /// `requestId` so the frontend store can discard a stale reply. This is a
    /// dedicated runtime-status envelope — it does NOT ride `Config` and does not
    /// resend providers/models/palette. The frontend merges its fields into
    /// `config.mcp` by server id.
    #[serde(rename_all = "camelCase")]
    McpStatus {
        request_id: String,
        servers: Vec<PushMcpStatusServer>,
        #[serde(skip_serializing_if = "Option::is_none")]
        global_error: Option<String>,
    },
    /// One-shot host-computed BRANCH LIST answering a `GitBranchList` request
    /// from the branch-switcher popover or the graph context menu (G4): every
    /// local + remote-tracking branch, current one flagged. Computed ENTIRELY
    /// host-side (never forwarded to the daemon) — like `GitStatus`, ALWAYS a
    /// reply so the picker never hangs. Carries
    /// [`super::git_branch::BranchListResult`] verbatim (already camelCase).
    BranchList(super::git_branch::BranchListResult),
    /// One-shot host-computed STASH LIST answering a `GitStashList` request (GK4a)
    /// for the Source Control toolbar's stash count/indicator: every stash entry
    /// (`index` + `message`), git's own `stash list` order. Computed ENTIRELY
    /// host-side (never forwarded to the daemon) — like `GitStatus`, ALWAYS a
    /// reply so the indicator never hangs. Carries
    /// [`super::git_stash::StashListResult`] verbatim (already camelCase).
    StashList(super::git_stash::StashListResult),
    /// One-shot host-computed REPO LIST answering a `GitRepos` request from the
    /// Source Control multi-repo picker: every git repo discovered across the
    /// session's workdirs, plus which one is currently active. Computed ENTIRELY
    /// host-side (never forwarded to the daemon) — like `GitStatus`, ALWAYS a
    /// reply so the picker never hangs (an empty list is a valid "no repos" state).
    /// Carries [`super::git_repos::RepoListResult`] verbatim (already camelCase);
    /// mirrors `BranchList`'s newtype/flatten shape (`{k:"RepoList", repos, active}`).
    RepoList(super::git_repos::RepoListResult),

    // ─── Coding panel (workspace file ops) ────────────────────────────────────
    /// Coding panel: directory listing reply.
    #[serde(rename_all = "camelCase")]
    FileTree {
        root: String,
        path: String,
        request_id: String,
        entries: Vec<super::push_rows::PushFileTreeEntry>,
        error: Option<String>,
    },
    /// Coding panel: file read reply.
    #[serde(rename_all = "camelCase")]
    FileRead {
        root: String,
        path: String,
        request_id: String,
        content: Option<String>,
        fingerprint: String,
        binary: bool,
        too_large: bool,
        error: Option<String>,
    },
    /// Coding panel: file save reply.
    #[serde(rename_all = "camelCase")]
    FileSave {
        root: String,
        path: String,
        request_id: String,
        fingerprint: String,
        error: Option<String>,
    },
    /// Coding panel: file create reply.
    #[serde(rename_all = "camelCase")]
    FileCreate {
        root: String,
        path: String,
        request_id: String,
        error: Option<String>,
    },
    /// Coding panel: file rename reply.
    #[serde(rename_all = "camelCase")]
    FileRename {
        root: String,
        old_path: String,
        new_path: String,
        request_id: String,
        error: Option<String>,
    },
    /// Coding panel: file delete reply.
    #[serde(rename_all = "camelCase")]
    FileDelete {
        root: String,
        path: String,
        request_id: String,
        error: Option<String>,
    },
    /// One-shot import-graph visualization result answering a `GuiReq::ImportGraph`
    /// request from the GUI. Computed by the linker daemon (off-thread), pushed the
    /// same way regardless of attach state, ALWAYS a reply so the panel never hangs.
    #[cfg(feature = "linker")]
    ImportGraph(super::import_graph::ImportGraphResult),
    /// One-shot impact analysis result answering a `GuiReq::ImportGraphImpact`.
    #[cfg(feature = "linker")]
    ImportGraphImpact(ImportGraphImpactResult),

    // ─── Remote host connect/disconnect ──────────────────────────────────────
    /// Remote connection state pushed to React. `state` is one of:
    /// `"disconnected"`, `"resolving"`, `"auth_required"`, `"bootstrapping"`,
    /// `"connecting"`, `"connected"`, `"error"`. Carries host identity + optional
    /// `sessionId` (set once connected) + optional `error` (set on `"error"`).
    #[serde(rename_all = "camelCase")]
    RemoteState {
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        host_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Push a swap-START [`PushEnvelope::Switching`] for target session `to`. Called at every
/// swap seam (a hub `Select`/`New` in either host state, a daemon `NewSession` hand-off)
/// the instant BEFORE teardown, so React raises a full-screen loader across the attach
/// gap; the next attached `Snapshot` clears it. Fire-and-forget: a serialise failure is
/// swallowed (the loader just never rises — no worse than before this signal existed).
pub(super) fn push_switching(push: &dyn Fn(String), to: &str) {
    let env = PushEnvelope::Switching { to: to.to_string() };
    if let Ok(json) = serde_json::to_string(&env) {
        push(json);
    }
}

/// Emit a one-shot `ModelList` envelope (out-of-band, un-fingerprinted) for the GUI
/// Connector model-id picker. Shared by the UN-ATTACHED swapper fallback
/// ([`super::host_swapper`]) so a detached `ListModels` lands the SAME envelope the attached
/// `push_loop` re-push produces — byte-compatible (echoed `provider` + `models`).
pub(super) fn push_model_list(push: &dyn Fn(String), provider: String, models: Vec<String>) {
    super::render::emit(push, &PushEnvelope::ModelList { provider, models });
}

/// Emit a one-shot `RouteList` envelope for the GUI Connector route picker, flattening each
/// daemon `ModelEndpointWire` to the camelCase `PushRoute` JS contract — the SAME mapping the
/// attached `push_loop` does. Shared by the UN-ATTACHED swapper fallback so a detached
/// `ListRoutes` lands the SAME envelope (echoed `provider` + `modelId`; an EMPTY `routes`
/// leaves the form showing only its synthetic "Auto").
pub(super) fn push_route_list(
    push: &dyn Fn(String),
    provider: String,
    model_id: String,
    routes: Vec<crate::ipc::proto::ModelEndpointWire>,
) {
    let env = PushEnvelope::RouteList {
        provider,
        model_id,
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
    super::render::emit(push, &env);
}

/// Emit a one-shot `FileDiff` envelope for the GUI Explore panel's Monaco diff tab,
/// carrying a host-computed [`super::diff::FileDiffResult`] verbatim. Shared by the
/// UN-ATTACHED swapper fallback and the attached `push_loop`'s off-thread worker,
/// since a `FileDiff` is serviced entirely host-side regardless of attach state.
pub(super) fn push_file_diff(push: &dyn Fn(String), result: super::diff::FileDiffResult) {
    let env = PushEnvelope::FileDiff {
        path: result.path,
        original: result.original,
        modified: result.modified,
        error: result.error,
        binary: result.binary,
        origin: result.origin.to_string(),
    };
    super::render::emit(push, &env);
}

/// Emit a one-shot `UsagePreview` envelope for the GUI activity-bar Usage panel, carrying
/// a host-computed [`super::diff::UsagePreviewResult`] plus the `scope` ("all"/"session") AND
/// `session_id` (the session uuid actually queried, `None` for "all") the request was
/// made under, both echoed back verbatim. Shared by the UN-ATTACHED swapper fallback and
/// the attached `push_loop`'s off-thread worker, since a `UsagePreview` is serviced
/// entirely host-side (the global ledger) regardless of attach state.
pub(super) fn push_usage_preview(
    push: &dyn Fn(String),
    result: super::diff::UsagePreviewResult,
    scope: String,
    session_id: Option<String>,
) {
    let env = PushEnvelope::UsagePreview {
        cost: result.cost,
        tokens_in: result.tokens_in.max(0) as u64,
        tokens_cached: result.tokens_cached.max(0) as u64,
        tokens_out: result.tokens_out.max(0) as u64,
        calls: result.calls.max(0) as u64,
        days: result
            .days
            .into_iter()
            .map(|(epoch, cost)| PushUsageDay { epoch, cost })
            .collect(),
        top_models: result
            .top_models
            .into_iter()
            .map(|m| PushUsageModel {
                model_id: m.model_id,
                cost: m.total_cost,
                calls: m.call_count.max(0) as u64,
            })
            .collect(),
        scope,
        session_id,
    };
    super::render::emit(push, &env);
}

/// Emit a one-shot `Analytics` envelope for the GUI Analytics tab, carrying a
/// host-computed [`super::diff::AnalyticsResult`] (correlation fields + KPI +
/// series + models + role split). Shared by the UN-ATTACHED swapper fallback
/// and the attached `push_loop`'s off-thread worker, since Analytics is
/// serviced entirely host-side (the global ledger) regardless of attach state.
pub(super) fn push_analytics(push: &dyn Fn(String), result: super::diff::AnalyticsResult) {
    let env = PushEnvelope::Analytics {
        req_seq: result.req_seq,
        scope: result.scope,
        session_id: result.session_id,
        range: result.range,
        metric: result.metric,
        status: result.status,
        error: result.error,
        cost: result.cost,
        tokens_in: result.tokens_in.max(0) as u64,
        tokens_cached: result.tokens_cached.max(0) as u64,
        tokens_out: result.tokens_out.max(0) as u64,
        calls: result.calls.max(0) as u64,
        cache_rate: result.cache_rate,
        series: result
            .series
            .into_iter()
            .map(|p| PushAnalyticsSeriesPoint {
                epoch: p.epoch,
                cost: p.cost,
                tokens: p.tokens,
            })
            .collect(),
        models: result
            .models
            .into_iter()
            .map(|m| PushAnalyticsModel {
                model_id: m.model_id,
                cost: m.cost,
                tokens_in: m.tokens_in.max(0) as u64,
                tokens_cached: m.tokens_cached.max(0) as u64,
                tokens_out: m.tokens_out.max(0) as u64,
                calls: m.calls.max(0) as u64,
            })
            .collect(),
        main_cost: result.main_cost,
        main_calls: result.main_calls.max(0) as u64,
        sub_cost: result.sub_cost,
        sub_calls: result.sub_calls.max(0) as u64,
    };
    super::render::emit(push, &env);
}

/// Emit a one-shot `SettingsValues` envelope for the GUI Settings tab. Shared by the
/// attached `push_loop` intercept (which unpacks the daemon's `DaemonEvent::SettingsValues`
/// reply) and the UN-ATTACHED swapper fallback ([`super::host_swapper`]), so a detached
/// `GetSettings` lands the SAME envelope the attached path produces.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_settings_values(
    push: &dyn Fn(String),
    name: String,
    workdir: Vec<String>,
    short_send: bool,
    sliding_cache: bool,
    bash_saving: bool,
    coding_autosave: bool,
    internet_mode: String,
    palette: String,
    effort: String,
    subagent_max_turns: u32,
) {
    super::render::emit(
        push,
        &PushEnvelope::SettingsValues {
            name,
            workdir,
            short_send,
            sliding_cache,
            bash_saving,
            coding_autosave,
            internet_mode,
            palette,
            effort,
            subagent_max_turns,
        },
    );
}

/// Emit a one-shot `AgentsValues` envelope for the GUI /agents dashboard. Shared by the
/// attached `push_loop` intercept (which unpacks the daemon's `DaemonEvent::AgentsValues`
/// reply) and the UN-ATTACHED host fallback ([`super::host`]), so a detached `GetAgents`
/// lands the SAME envelope the attached path produces.
pub(super) fn push_agents_values(
    push: &dyn Fn(String),
    req_seq: u64,
    agents: Vec<crate::ipc::proto::AgentEntry>,
    catalogue_models: Vec<crate::ipc::proto::CatalogueModelSnapshot>,
    catalogue_providers: Vec<crate::ipc::proto::CatalogueProviderSnapshot>,
    available_tools: Vec<String>,
) {
    super::render::emit(
        push,
        &PushEnvelope::AgentsValues {
            req_seq,
            agents,
            catalogue_models,
            catalogue_providers,
            available_tools,
        },
    );
}

/// Emit a one-shot `StoreCatalogue` envelope for the GUI Store tab, carrying host-computed
/// koma.run catalogue rows (or an `error` on a network/parse failure). Shared by the
/// UN-ATTACHED `host_swapper` fallback and the attached `push_loop`'s off-thread worker —
/// Store browse is serviced ENTIRELY host-side (see `store_host::fetch_catalogue`)
/// regardless of attach state, mirroring `push_file_diff`/`push_git_status`.
pub(super) fn push_store_catalogue(
    push: &dyn Fn(String),
    items: Vec<crate::ipc::proto::StoreItemWire>,
    error: Option<String>,
) {
    super::render::emit(push, &PushEnvelope::StoreCatalogue { items, error });
}

/// Emit a one-shot `StoreItemDetail` envelope for the GUI Store detail pane. Shared the
/// same way as [`push_store_catalogue`] — see `store_host::fetch_detail`.
pub(super) fn push_store_detail(
    push: &dyn Fn(String),
    detail: Option<crate::ipc::proto::StoreDetailWire>,
    error: Option<String>,
) {
    super::render::emit(push, &PushEnvelope::StoreItemDetail { detail, error });
}

/// Emit a one-shot `InstalledExtensions` envelope for the GUI Store "Installed" section,
/// carrying a host-read `~/.koma/config.json` registry projection. Shared the same way as
/// [`push_store_catalogue`] — see `store_host::installed_extensions`. Also reused verbatim
/// by the daemon-forwarded install/uninstall path's re-push (unchanged — that still rides
/// `DaemonEvent::InstalledExtensions` through `push_intercept`).
pub(super) fn push_installed_extensions(
    push: &dyn Fn(String),
    items: Vec<crate::ipc::proto::InstalledExtWire>,
) {
    super::render::emit(push, &PushEnvelope::InstalledExtensions { items });
}

/// Emit a one-shot `InstalledExtensionDetail` envelope for the GUI installed-extension
/// detail tab. Shared the same way as [`push_store_catalogue`] — see
/// `store_host::get_installed_detail`.
pub(super) fn push_installed_ext_detail(
    push: &dyn Fn(String),
    id: String,
    detail: Option<crate::ipc::proto::InstalledExtensionDetailWire>,
    error: Option<String>,
) {
    super::render::emit(
        push,
        &PushEnvelope::InstalledExtensionDetail { id, detail, error },
    );
}

/// Emit a one-shot `ExtensionOpResult` envelope for the DETACHED (home screen / swapper)
/// install/uninstall path — the reply to a `GuiReq::InstallExtension`/`UninstallExtension`
/// run host-locally, echoing `id` so the GUI clears that card's pending spinner. The
/// ATTACHED path's reply is a separate inline re-push of the daemon's own
/// `DaemonEvent::ExtensionOpResult` (see `push_intercept`) — this is the detached twin, used
/// by `store_host::spawn_install`/`spawn_uninstall`.
pub(super) fn push_ext_op_result(
    push: &dyn Fn(String),
    id: String,
    ok: bool,
    error: Option<String>,
) {
    super::render::emit(push, &PushEnvelope::ExtensionOpResult { id, ok, error });
}

/// Emit a one-shot `OAuthState` envelope for the streaming GUI OAuth surface. Shared by the
/// attached `push_loop` intercept (which unpacks the daemon's `DaemonEvent::OAuthState`) and
/// the UN-ATTACHED host fallback ([`super::host`]), so a detached `GetOAuthState` /
/// `DeleteOAuthConn` lands the SAME envelope the attached path produces. `conns` is the
/// TOKENLESS connection list; no secret ever crosses here.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_oauth_state(
    push: &dyn Fn(String),
    phase: String,
    url: Option<String>,
    user_code: Option<String>,
    verification_url: Option<String>,
    error: Option<String>,
    conns: Vec<crate::ipc::proto::OAuthConnWire>,
    providers: Vec<crate::ipc::proto::OAuthProviderWire>,
) {
    super::render::emit(
        push,
        &PushEnvelope::OAuthState {
            phase,
            url,
            user_code,
            verification_url,
            error,
            conns,
            providers,
        },
    );
}

/// Emit a one-shot `RemoteState` envelope for the GUI remote-host panel.
/// `state` is one of: `"disconnected"`, `"resolving"`, `"auth_required"`,
/// `"bootstrapping"`, `"connecting"`, `"connected"`, `"error"`.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_remote_state(
    push: &dyn Fn(String),
    state: &str,
    host_id: Option<&str>,
    user: Option<&str>,
    host: Option<&str>,
    session_id: Option<&str>,
    error: Option<&str>,
) {
    super::render::emit(
        push,
        &PushEnvelope::RemoteState {
            state: state.to_string(),
            host_id: host_id.map(str::to_string),
            user: user.map(str::to_string),
            host: host.map(str::to_string),
            session_id: session_id.map(str::to_string),
            error: error.map(str::to_string),
        },
    );
}

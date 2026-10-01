import type { DiagramDoc } from '../../lib/diagram'
import type { EditorGroupId, EditorLayoutNode, SplitDir } from '../editorGroups'
import type { AnalyticsData, AnalyticsMetric, AnalyticsRange, AnalyticsScope } from './analytics'
import type { ChatMessage, PaletteInfo } from './chat'
import type { ActivityCommit, CommitDetail, GitCommitNode } from './git'
import type { InstalledExt, InstalledExtDetail, StoreDetail, StoreItem } from './marketplace'
import type { OAuthConn, OAuthPhase, OAuthProviderEntry } from './oauth'
import type { AttachmentEntry, BashJobEntry, BootstrapState, FileChangeEntry, HubCookingEntry, HubHistoryEntry, LoadPhase, ModelRoute, PendingCall, PlanTodoEntry, SearchResultEntry, SubAgentEntry, ToastEntry } from './session'
import type { Tab } from './tabs'
import type { McpServer, Model, Provider } from '../../types/config'

// ---- Store shape --------------------------------------------------------

export type SessionSlice = {
  id: string | null
  state: string | null
  messages: ChatMessage[]
  // Host-reported full projected length. When > messages.length (or hasMoreOlder),
  // ChatView can pull HistoryPage for the rest.
  messageCount: number
  // True while host still holds older history beyond the FE store.
  hasMoreOlder: boolean
  title: string
  working: boolean
  stream: string
  reasoning: string
  subagents: SubAgentEntry[]
  bash: BashJobEntry[]
  fileChanges: FileChangeEntry[]
  // Plan-mode todo checklist (Explore "PLAN" section). REPLACED wholesale on
  // each Snapshot; empty outside Plan mode or before a plan exists.
  planTodos: PlanTodoEntry[]
  attachments: AttachmentEntry[]
  searchResults: SearchResultEntry[]
  // Global agent mode token ("auto"/"normal"/"plan"/"yolo"), projected from the
  // host's process-global agent_mode via the Snapshot envelope. Drives the
  // composer mode selector. Defaults to "auto".
  mode: string
  // Queued mid-turn follow-ups (host `SessionSnapshot.pending_steer`) —
  // submits made while the turn is cooking are queued daemon-side (cap 5) rather
  // than starting a new turn. Full text for edit/remove. REPLACED wholesale on
  // each Snapshot.
  pendingSteer: string[]
  // Approval gate (host `awaiting_approval`): true while the turn is parked on a
  // y/a/n decision. Drives the ApprovalOverlay modal (risky/classifier pause) +
  // the inline plan controls (plan_ready pause). REPLACED on each Snapshot.
  awaitingApproval: boolean
  // The classifier's reason for a risky pause (null for a plan_ready / non-
  // classifier park). Shown as the "why" in the approval card.
  modelRoutes: ModelRoute[]
  approvalReason: string | null
  // The tool call the session is parked on (name/args of
  // pending_tool_calls[tool_idx]); null when not awaiting. Distinguishes a plan
  // decision (`name === 'plan_ready'`) from a tool approval.
  pendingCall: PendingCall | null
  // SDLC projection fields (mode=sdlc only; cleared on mode switch / session change).
  sdlcPhase: string | null
  sdlcGoal: string | null
  sdlcBranch: string | null
  sdlcOpen: number | null
  sdlcSealed: number | null
  // Usage counters + running cost projected on every Status push (host
  // token-accounting). Drive the UsageFooter statusline. Default to 0 when the
  // host hasn't projected them yet.
  tokensIn: number
  tokensCached: number
  tokensOut: number
  cost: number
}

export type HubSlice = {
  state: string | null
  cooking: HubCookingEntry[]
  history: HubHistoryEntry[]
}

// Global config (not per-session) — authoritative from the daemon's
// AppConfig projection. Always REPLACED wholesale by a Config push, never
// accumulated.
export type ConfigSlice = {
  mcp: McpServer[]
  providers: Provider[]
  models: Model[]
  // True once the first authoritative Config push has landed. The pre-session
  // gate (start screen vs onboarding) waits on this so it never flashes
  // onboarding against the empty initial slice before the host reports config.
  loaded: boolean
  // Host's first-run flag (see Config envelope). Undefined until pushed.
  firstRun?: boolean
  // Active theme name + advertised theme registry (see Config envelope).
  theme: string
  themes: string[]
  // Full palette catalogue with resolved colours (Settings Appearance grid).
  // Empty until the first Config push that carries it.
  palettes: PaletteInfo[]
}

// The OAuth login screen's full state (host `OAuthState` push) — global, not
// per-session (mirrors ConfigSlice). REPLACED wholesale on each push, never
// accumulated. `url`/`userCode`/`verificationUrl`/`error` are only meaningful
// for the phase that produces them (see `OAuthPhase`); the others sit at
// `null` outside their owning phase.
export type OAuthSlice = {
  phase: OAuthPhase
  url: string | null
  userCode: string | null
  verificationUrl: string | null
  error: string | null
  conns: OAuthConn[]
  providers: OAuthProviderEntry[]
}

// The extension-STORE tab's full state — global (not per-session). `catalogue`
// is the last browse result; `detail` is the currently-open detail (null on the
// grid view or while loading); `installed` is the local registry (kept fresh via
// the InstalledExtensions push after every install/uninstall). `busy` covers a
// browse/detail fetch OR an in-flight install/uninstall (the grid/detail show a
// spinner); `error` is the last store/op error string (null when clear).
// GUI Tutorial tab local transcript + in-flight turn. Not a real session —
// host-proxied koma-free only. Transcript stays in memory for the window life.
export type TutorialMsg = {
  id: string
  role: 'user' | 'assistant'
  content: string
  tour?: string | null
}
export type TutorialSlice = {
  messages: TutorialMsg[]
  busy: boolean
  error: string | null
  // Client turn id awaiting TutorialChatDone (stale-drop).
  pendingId: string | null
  // Latest offered tour id from the coach (also mirrored on the assistant msg).
  pendingTour: string | null
}

export type StoreSlice = {
  catalogue: StoreItem[]
  detail: StoreDetail | null
  installed: InstalledExt[]
  // Full detail for the currently-open installed-extension detail tab (Tab-B).
  // `null` when no installed-ext tab is open or while loading; REPLACED on each
  // InstalledExtensionDetail push.
  installedDetail: InstalledExtDetail | null
  // Independent loading/error state for the currently-open installed-extension
  // detail tab. These must not share the marketplace's busy/error state.
  installedDetailLoading: boolean
  installedDetailError: string | null
  // Requested extension id for the currently in-flight
  // GetInstalledExtensionDetail request. Stale replies whose id doesn't match
  // are silently dropped.
  installedDetailRequestId: string | null
  busy: boolean
  error: string | null
  // Per-extension in-flight install/uninstall id (so ONLY that card shows its
  // button spinner, not the whole grid). Cleared on its ExtensionOpResult.
  pendingOp: string | null
  // Which op `pendingOp` is (so the ExtensionOpResult handler can word the
  // `opResult` confirmation as "Installed x" vs "Uninstalled x"). Cleared
  // alongside `pendingOp`.
  pendingOpKind: 'install' | 'uninstall' | null
  // Last install/uninstall outcome — read by the notice banner in BOTH the
  // grid and the detail view (unlike `error`, which is browse/detail-fetch-only).
  // Cleared on navigation (browseStore/openStoreDetail/closeStoreDetail), on the
  // next op start, or an explicit dismiss (clearStoreNotice).
  opResult: { ok: boolean; message: string } | null
}

// Local-only UI state (never pushed by the host, never sent upstream) — the
// omnisearch overlay's open/closed flag. Kept in the store (rather than
// component state) so the Composer, nested under a different route subtree
// than RootLayout's overlay mount point, can open it without prop drilling.
export type UiSlice = {
  omnisearchOpen: boolean
  // One-shot signal: a workspace path picked from OmniSearchPalette, queued
  // for the Composer to append into its local draft text. The daemon's
  // attachment ingest is image-only, so omnisearch picks are inserted as a
  // plain path reference (for the model to read via its own tools) rather
  // than routed through AttachPath. Composer consumes this via useEffect and
  // clears it with consumeComposerInsert so it doesn't re-fire on rerender.
  composerInsert: string | null
  // Diagrams queued for the composer. Each one renders as a drawing chip.
  // Submit sends the Mermaid, which is what the model reads.
  diagramChatQueue: { title: string; mermaid: string; doc: DiagramDoc }[]
  /** Pending composer marker-insert rows for AttachFile/AttachPaste from outside the composer. */
  pendingComposerAttachmentInserts: {
    id: string
    kind: 'image' | 'pasted_text'
    name?: string
    text?: string
    path?: string
  }[]
  // Design slices queued as chips. Submit sends the kdsgn fence, which is
  // what the model reads. The fence is not written into the draft text.
  designChatQueue: { title: string; text: string }[]
  // One-shot body for a staged `[Pasted Text #N]` chip. The composer copies it
  // into the editable chip and clears it.
  pasteBody: { markerN: number; text: string } | null
  // One-shot signal: text to REPLACE the Composer's draft with, queued by a
  // rewind (the hover-edit pencil on a user bubble). Distinct from
  // `composerInsert` (which APPENDS an omnisearch path) — rewind refills the
  // whole draft with the rewound message's text for editing + resend. Composer
  // consumes it via useEffect and clears it with consumeComposerRefill.
  composerRefill: string | null
  // Staged rewind (edit pencil): the DISPLAY index of the user message being
  // edited, remembered so the NEXT send fires `RewindTo(index)` before `Submit`
  // (rewind-on-send). `null` when no rewind is staged. Set by `stageRewind` (edit
  // click), cleared by `clearRewind` (send commits it, or the composer is emptied
  // to cancel). Clicking edit does NOT truncate — the chat stays visible until send.
  pendingRewindIndex: number | null
  // Monotonic tick bumped on every send. ChatView watches it to FORCE a
  // jump-to-bottom (re-engaging the scroll-stick regardless of scroll position)
  // when the user submits while scrolled up. Not a boolean so repeat sends at
  // the same scroll position still fire the effect.
  scrollTick: number
  // Full-screen session-swap overlay: set optimistically the moment
  // SelectSession/NewSession is emitted from ResumePalette, holding the
  // target session's display name. There is no host-pushed "swap started"
  // signal on this build (the attach can block synchronously for several
  // seconds — build-skew daemon restarts, cold session spawn), so the next
  // authoritative Snapshot is the only reliable clear point. `null` = no
  // swap in flight.
  switchingTo: string | null
  // Active transient toast (host safeguard/harness/generic notice), or null when
  // none is showing. Set from the Status envelope's `toast`/`kind`; cleared by
  // ToastContainer's auto-dismiss (or a newer toast replacing it). Deduped by
  // text so a host that re-pushes the same live toast on every Status tick
  // doesn't keep resetting the timer.
  toast: ToastEntry | null
  // Monotonic counter minting `ToastEntry.id` — guarantees each distinct toast
  // gets a fresh id (and thus a fresh dismiss timer) even after a null gap.
  toastSeq: number
  // VSCode-style editor tabs over the main content column. tabs[0] is always
  // the permanent chat tab; diff tabs append as File-changed rows are opened.
  // Local-only UI state — never pushed by the host. Reset to just the chat tab
  // on a genuine session switch (a diff tab is file-change context for the OLD
  // session).
  tabs: Tab[]
  // The focused group's shown tab. Other groups have their own entry in
  // `groupActive` and remain visible while this points elsewhere.
  activeTabId: string
  // VSCode-style editor groups. Membership is stored separately from Tab so
  // existing open-tab actions remain additive; normalizeGroups stamps newly
  // opened tabs into the focused group and repairs stale mappings.
  groups: EditorGroupId[]
  tabGroup: Record<string, EditorGroupId>
  groupActive: Record<EditorGroupId, string>
  activeGroupId: EditorGroupId
  splitDir: SplitDir
  groupSizes: Record<EditorGroupId, number>
  splitTree: EditorLayoutNode
  groupSplitDir: Record<EditorGroupId, SplitDir>
  // Monotonic tick bumped by `focusPlanSection` (the UsageFooter PLAN badge
  // click): a cross-tree signal, mirrors `scrollTick`. RootLayout watches it
  // to open the Explore sidebar/panel; ExplorePanel watches it to expand its
  // PLAN section — both live outside the Composer/footer's subtree, so a
  // store tick (not a prop) is how the click reaches them.
  focusPlanTick: number
  // Usage panel scope toggle ("all" = global last-7-days [default], "session" =
  // the same window filtered to the CURRENT session's ledger rows only). The
  // Sidebar header's all/session control is HIDDEN whenever there's no current
  // session (the welcome/start screen) and this is forced back to "all" the
  // instant a session goes away while "session" was selected (see UsagePanel's
  // session-loss effect + `setUsageScope`).
  usageScope: 'all' | 'session'
  // TUI-parity startup splash state (host `Loading` push envelope) — drives
  // SwitchingOverlay's cold-session warm-up splash (centered "koma" wordmark +
  // the two phase rows). `null` = no warm-up in flight. Set by the `Loading`
  // envelope case; defensively cleared on a genuine session switch (Snapshot's
  // `switched` branch) and in `detachSession()` so a stale splash from the OLD
  // session/warm-up can never leak into a new attach.
  loading: { active: boolean; workspace: LoadPhase; awareness: LoadPhase } | null
  // GUI attach hydration shown above `loading`: session Snapshot, Config, then
  // the SettingsValues/RepoList replies requested by Snapshot. Null once every
  // phase is terminal. This is deliberately separate from daemon Loading.
  bootstrap: BootstrapState | null
  // Skip-latch: set true when the user dismisses the cold-start splash via
  // the Skip button. Suppresses splash VISIBILITY only — `loading` itself
  // keeps updating with fresh host `Loading` pushes underneath. Reset to
  // false on a genuine session switch/detach so a NEW cold-start splash can
  // still show later.
  loadingDismissed: boolean
}

// A single node from the linker daemon's import graph.
export type ImportGraphNode = {
  path: string
  language: string
  outDegree: number
  inDegree: number
  role: 'Focus' | 'Dependency' | 'Dependent' | 'Overview'
  depthFromFocus: number | null
  workspaceRoot: string | null
}

// A single edge from the linker daemon's import graph.
export type ImportGraphEdge = {
  from: string
  to: string
}

// Per-root workspace metadata for filter pickers.
export type ImportGraphRootInfo = {
  /** Canonical identity path used for requests, security, and matching. */
  root: string
  /** The configured (user-input) path — may be a symlink or relative spelling.
   *  Identical to `root` when the configured path already canonicalises to `root`.
   *  Omitted (undefined) from JSON when it equals `root`. */
  configuredPath?: string
  /** Human-friendly display label for the UI (typically basename). Kept
   *  distinct from `root` so the frontend can show a short name while keeping
   *  the canonical path in the title/sublabel. */
  displayPath?: string
  fileCount: number
  languages: { name: string; count: number }[]
  indexedState: 'indexed' | 'scanning' | 'not-indexed' | 'unavailable'
}

// The commit-graph tab's slice (G2) — the loaded commit page(s) + selection +
// fetched detail. `loadMode` records how the LAST GitGraph req was issued so the
// reducer knows whether to concat (append) or replace the incoming page. Global
// (mirrors the `git` slice; the host resolves the graph off the foreground
// session's repo). Reset naturally: a session switch closes the tab (tabs reset
// to chat), and reopening remounts GraphTab → a fresh refreshGraph.
export type GraphSlice = {
  commits: GitCommitNode[]
  head: string | null
  hasMore: boolean
  loading: boolean
  loadMode: 'replace' | 'append'
  // A refreshGraph() call that landed while a GitGraph request was already in
  // flight (append or replace) — client-side serialization keeps at most one
  // GitGraph request in flight at a time so `loadMode` is never ambiguous
  // between a racing append and replace reply. Set true instead of firing;
  // replayed by the 'GitGraph' reducer once the in-flight reply lands and
  // `loading` clears.
  pendingRefresh: boolean
  selectedSha: string | null
  detail: CommitDetail | null
  // Rail-line (default, the existing virtualized commit list) vs Bubble
  // (GK5's activity view — a placeholder here) mode switch (GK2). Local UI
  // preference, never persisted, never touches the wire.
  graphMode: 'rail' | 'bubble'
}

// The bubble/activity chart's slice (GK5b) — the loaded per-commit activity
// series for whatever path is currently narrowed to (`null` = whole active
// branch). Global (mirrors `graph`; the host resolves it off the foreground
// session's repo). `error` set means the workdir isn't a git repo — `commits`
// is then empty rather than the chart guessing at a stale page.
export type ActivitySlice = {
  commits: ActivityCommit[]
  loading: boolean
  error: string | null
  path: string | null
}

// The import-graph tab's slice — the loaded graph from the linker daemon.
// Global (mirrors the graph slice; the host resolves it off the foreground
// session's workspace). Reset naturally: a session switch closes the tab
// (tabs reset to chat), and reopening remounts ImportGraphTab → a fresh
// refreshImportGraph.
export type ImportGraphSlice = {
  status: 'idle' | 'ok' | 'scanning' | 'not-indexed' | 'unavailable'
  nodes: ImportGraphNode[]
  edges: ImportGraphEdge[]
  focus: string | null
  generation: number
  fileCount: number
  edgeCount: number
  languages: string[]
  nodesTruncated: boolean
  edgesTruncated: boolean
  totalNodesAvailable: number
  totalEdgesAvailable: number
  loading: boolean
  error: string | null
  // Controls
  selectedPath: string | null
  depth: number
  direction: 'dependencies' | 'dependents' | 'both'
  // Chain of visited focus paths for chained exploration.
  breadcrumb: string[]
  // Available workspace roots (from daemon graph data, for filter pickers).
  availableRoots: ImportGraphRootInfo[]
  // Active filters: empty array = "all" (no filtering).
  filterRoots: string[]
  filterLanguages: string[]
  // Request coalescing: when a refresh is requested while loading, set true.
  // The ImportGraph reducer replays exactly one request when the in-flight reply lands.
  queuedRefresh: boolean
  // Persistent browser tree: all files seen across overview + focused replies.
  // The sidebar panel and Cmd+K search read from this, so focusing one
  // neighborhood never removes other files from the tree.
  treeNodes: ImportGraphNode[]
  // Impact analysis state.
  impactRequestId: string | null
  impactPath: string | null
  impactDepth: number
  impactStatus: 'idle' | 'loading' | 'loaded' | 'error'
  impactPaths: string[]
  impactTotal: number
  impactError: string | null
  // Reindex lifecycle: set true when an ImportGraphReindex req fires; cleared
  // by the next ImportGraph push (success or error). Drives the refresh
  // button spinner + 'Indexing workspace' banner.
  reindexBusy: boolean
  reindexError: string | null
  // Request correlation: the latest request id and session id for stale-reply
  // rejection.  `activeRequestId` is set by refreshImportGraph /
  // reindexImportGraph and compared against the push envelope's `requestId`.
  // `activeSessionId` is set the same way and compared against `sessionId` so
  // a reindex started before a session switch is silently dropped.
  activeRequestId: string | null
  activeSessionId: string | null
}

// The Analytics dashboard tab's slice — host-authoritative projection + local
// filter/lifecycle state. `loading`/`error`/`data` are mutually exclusive
// presentation states; correlation fields (`reqSeq`/`scope`/`sessionId`/
// `range`/`metric`) reject stale replies. Global (ledger is process-local),
// but session-scope results are still keyed by `sessionId`.
export type AnalyticsSlice = {
  scope: AnalyticsScope
  range: AnalyticsRange
  metric: AnalyticsMetric
  // Client-minted monotonic request id; 0 = none in flight yet.
  reqSeq: number
  // Session uuid actually requested for the in-flight/last accepted reply
  // (null for "all" scope). Used with scope/range/metric/reqSeq to reject
  // stale replies.
  sessionId: string | null
  loading: boolean
  error: string | null
  data: AnalyticsData | null
  // True once a matching reply has been accepted for the CURRENT filters
  // (even when status was "empty"). Distinguishes "never loaded" from
  // "loaded empty".
  hasData: boolean
}

// Persisted layout for the VSCode-style ActivityBar's managed (built-in)
// icons — drag-reorder + per-icon visibility toggle (Settings "Sidebar"
// section), keyed by each item's stable `view` id (ActivityBar.ACTIVITY_BAR_ITEMS)
// so it round-trips through localStorage across reloads. `order` need not list
// every known id (see `resolveActivityBarOrder`) — a fresh install has an empty
// order/hidden pair and every item defaults to its ACTIVITY_BAR_ITEMS position,
// visible. Local-only, never touches the daemon wire.
export type ActivityBarLayout = {
  order: string[]
  hidden: string[]
}

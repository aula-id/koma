import type { DesignDoc } from '../lib/design'
import type { DiagramDoc } from '../lib/diagram'
import type { LspDiagnostic } from '../lib/lsp-bridge'
import type { CodingSlice } from './coding'
import type { DesignSlice } from './design'
import type { DiagramSlice } from './diagram'
import type { EditorGroupId, SplitDir } from './editorGroups'
import type { AnalyticsMetric, AnalyticsRange, AnalyticsScope } from './types/analytics'
import type { EffortOptions, PaletteColors, SettingsValues, UsagePreview } from './types/chat'
import type { PushEnvelope } from './types/envelope'
import type { BranchInfo, GitPushMode, GitStatus, KeyInfo, KeyReveal, RepoEntry, StashEntry } from './types/git'
import type { AgentEntry, CatalogueModelEntry, CatalogueProviderEntry, DyingMark, LspInstallProgress, LspRuntimeServer, LspServerStatus } from './types/session'
import type { ActivityBarLayout, ActivitySlice, AnalyticsSlice, ConfigSlice, GraphSlice, HubSlice, ImportGraphSlice, OAuthSlice, SessionSlice, StoreSlice, TutorialSlice, UiSlice } from './types/slices'
import type { ModelListEntry, RouteEntry } from '../types/config'

export type KomaState = {
  computer: import('../types/computer').ComputerStatus | null
  computerError: string | null
  session: SessionSlice
  hub: HubSlice
  palette: PaletteColors
  ui: UiSlice
  config: ConfigSlice
  oauth: OAuthSlice
  store: StoreSlice
  tutorial: TutorialSlice
  // Persisted ActivityBar order/visibility layout (see `ActivityBarLayout`).
  activityBar: ActivityBarLayout
  // Live per-provider model-id catalogue, keyed by the most recent
  // ListModels reply's provider (see ModelForm's provider-select trigger).
  modelList: ModelListEntry[]
  // Live per-model route (OpenRouter endpoint) list from the most recent
  // ListRoutes reply — carries the provider+modelId it was fetched for so the
  // consumer can ignore a stale reply. `null` until the first reply lands.
  routeList: { provider: string; modelId: string; routes: RouteEntry[] } | null
  // The Settings tab's Session-section values from the latest GetSettings /
  // SetPrefs re-push. `null` until the first reply lands (the tab shows a
  // loading row); REPLACED wholesale on each reply.
  settingsValues: SettingsValues | null
  // The composer EffortPicker's latest GetEffortOptions reply. `null` until the
  // first reply lands (the picker shows a loading row); REPLACED wholesale on
  // each reply — the picker clears this to `null` itself right before firing a
  // fresh GetEffortOptions (the open-time re-request), so a stale menu never
  // lingers under a different state.
  effortOptions: EffortOptions | null
  // The activity-bar Usage panel's latest LAST-7-DAYS preview. `null` until the
  // first reply lands (the panel shows a loading row); REPLACED wholesale on
  // each reply. The panel re-requests it every time it's shown.
  usagePreview: UsagePreview | null
  // True from the moment a UsagePreview req fires (mount, scope/session
  // change, or the Sidebar header's manual refresh button) until the
  // matching reply is applied — drives the refresh button's spinner. A
  // dropped stale/mismatched reply (see the 'UsagePreview' push case)
  // leaves this true, since the request it answers isn't the current one.
  usagePreviewBusy: boolean
  // MCP sidebar status refresh lifecycle: set the moment a GetMcpStatus fires
  // (mount or manual refresh) and cleared when a matching McpStatus reply
  // lands. Drives the refresh button spinner. A stale/discarded reply leaves
  // this true.
  mcpStatusBusy: boolean
  // The requestId of the currently in-flight GetMcpStatus. Used to discard
  // stale replies (when the requestId in the reply doesn't match this).
  mcpStatusRequestId: string | null
  // Agent-tab saving lifecycle tracker — set right before a SetAgent request,
  // cleared on the confirmatory push. `seq` prevents stale-reply races.
  // `null` when no save is in flight.
  agentSaving: {
    tabId: string
    seq: number
    originalName: string | null
    newName: string
  } | null
  // Session ids with a KillSession/DeleteSession req in flight (ResumePalette
  // / StartScreen row kill/delete confirm) — renders that row non-interactive
  // + spinning instead of its trailing action. Kind-scoped (see `DyingMark`)
  // so a kill mark migrating cooking->history on the next Hub push can't
  // leak onto the row it migrated into. Pruned automatically the moment a
  // fresh Hub push confirms the kill/delete landed, so no explicit "done"
  // signal is needed.
  dyingSessions: DyingMark[]
  // The Agents dashboard's full agent list (built-in + global + session,
  // merged daemon-side) from the latest AgentsValues push. REPLACED wholesale
  // on each push — empty until the first GetAgents reply lands.
  agents: AgentEntry[]
  // The Agents dashboard's model/provider catalogues, from the same push —
  // feeds the AgentTab model picker and the panel row's resolved model label.
  catalogueModels: CatalogueModelEntry[]
  catalogueProviders: CatalogueProviderEntry[]
  // The full set of tool names the daemon knows about — feeds the AgentTab
  // tools field's toggle-chip grid (one chip per available tool). REPLACED
  // wholesale on each AgentsValues push; empty until the first reply lands.
  availableTools: string[]
  // The Source Control "GIT" panel's authoritative status (latest GitStatus
  // push) — global, not per-session (mirrors ConfigSlice; the host resolves
  // it off the foreground session's workdir). REPLACED wholesale on each
  // push; starts at the neutral "no repo" default until the first reply.
  git: GitStatus
  // Which remote sync op (Fetch/Pull/Push) is currently in flight, or null when
  // none is — a transient (not host-authoritative) flag so the GIT panel's sync
  // toolbar can disable its buttons + spinner the active one. Set by
  // `gitFetch`/`gitPull`/`gitPush` right before firing the req; cleared by the
  // matching `GitOp` push reply (success OR failure — either way the op is no
  // longer in flight).
  remoteBusy: string | null
  // Controlled draft text for the GIT panel's commit box. Store-level (not
  // component state) so navigating away from Source Control and back doesn't
  // lose an in-progress message; cleared automatically on a successful commit
  // (see the push reducer's 'GitOp' case).
  commitDraft: string
  // The Settings "SSH Keys" section's authoritative vault list (latest KeyList
  // push) — a GUI-only, manual, user-owned key vault, entirely separate from
  // the model's own git credential machinery. REPLACED wholesale on each push;
  // empty until the first reply lands. Global (not per-session), mirroring `git`.
  keys: KeyInfo[]
  // Settings "Language servers" catalogue (latest LspStatus). Empty until first reply.
  lspServers: LspServerStatus[]
  // Per-id install progress (latest LspInstall). Cleared when status lands idle.
  lspProgress: Record<string, LspInstallProgress>
  // Diagnostics keyed by file URI (latest LspDiagnostics per uri).
  lspDiagnostics: Record<string, LspDiagnostic[]>
  // Precomputed problem badge counts (updated in LspDiagnostics reducer).
  lspDiagCounts: { errors: number; warnings: number }
  // Live language-server processes (starting / indexing / ready). Footer drawer.
  lspRuntime: LspRuntimeServer[]
  // One docked panel shared by Tasks, Language Servers and Problems.
  bottomPanelTab: 'tasks' | 'lsp' | 'problems' | null
  setBottomPanelTab: (tab: 'tasks' | 'lsp' | 'problems' | null) => void
  // The SSH Keys section's transient "Copy public key" / "Reveal private key"
  // result (latest KeyReveal push), or `null` when nothing has been revealed
  // yet / the reveal box was dismissed. Kept separate from `keys` (the list
  // itself never carries key material). Named distinctly from the `keyReveal`
  // ACTION below (same-name field+method would collide in this one object type).
  keyRevealResult: KeyReveal | null
  // The branch-switcher popover (footer/GitPanel) + graph context menu's
  // authoritative branch list (latest BranchList push) — every local +
  // remote-tracking branch, current one flagged. REPLACED wholesale on each
  // push; empty until the first reply lands. Global (not per-session),
  // mirroring `git`/`keys` (G4).
  branches: BranchInfo[]
  // Transient (not host-authoritative) picker-loading flag: `true` from
  // `refreshBranches()` until the matching `BranchList` reply lands, so the
  // popover can show a spinner instead of a stale/empty list.
  branchesLoading: boolean
  // Generation of the latest branch-list request; used to filter out-of-order replies.
  branchesRequestId: number | null
  // Every detected repository root in the workspace (multi-repo support) —
  // latest RepoList push. REPLACED wholesale on each push; empty until the
  // first reply lands. Global (not per-session), mirroring `branches`.
  repos: RepoEntry[]
  // The repo picker's currently-active root (latest RepoList push's `active`
  // field), or `null` when no repo has been detected/selected yet. Drives
  // which repo `git`/`graph`/`activity` describe.
  activeRepoRoot: string | null
  // The toolbar's authoritative stash list (latest StashList push, GK4c) —
  // every `git stash list` entry, newest (index 0) first. REPLACED wholesale
  // on each push; empty until the first reply lands. Global (not
  // per-session), mirroring `branches`. Drives the Pop button's
  // enabled-state + count badge.
  stashes: StashEntry[]
  // Rust -> JS: apply an authoritative push envelope. Always REPLACES the
  // relevant slice fields — never accumulates/appends.
  // The GitKraken-style commit-graph tab's slice (G2). See GraphSlice.
  graph: GraphSlice
  // The bubble/activity chart's slice (GK5b). See ActivitySlice.
  activity: ActivitySlice
  // The Analytics dashboard tab's slice. See AnalyticsSlice.
  analytics: AnalyticsSlice
  // The import-graph tab's slice — linker daemon code-dependency graph.
  importGraph: ImportGraphSlice
  // Coding panel slice — workspace roots, lazy dir listings, open file docs.
  coding: CodingSlice
  // Saved remote hosts (GUI remote panel). REPLACED wholesale on each push.
  remoteHosts: RemoteHost[]
  // Remote connection state (SSH connect/disconnect lifecycle). REPLACED
  // wholesale on each push; drives the remote panel's connect/disconnect UI.
  remoteState: {
    state: string
    hostId: string | null
    user: string | null
    host: string | null
    sessionId: string | null
    error: string | null
    sessions: Array<{
      sessionId: string
      name: string
      pwd?: string
      working: boolean
      isForeground: boolean
    }>
  }
  remotePath: {
    state: 'idle' | 'listing' | 'ready' | 'error' | 'cancelled'
    path: string
    dirs: string[]
    error: string | null
  }
  // Open (or focus) the singleton commit-graph tab (id 'graph'). The GraphTab
  // itself fires refreshGraph on mount, so opening is enough. Mirrors
  // openSettingsTab's dedupe + activate shape.
  openGraphTab: () => void
  // Open (or focus) the singleton Analytics tab (id 'analytics'). The
  // AnalyticsTab fires refreshAnalytics on mount / filter change.
  openAnalyticsTab: () => void
  // (Re)load the Analytics dashboard for the CURRENT filters: bump reqSeq,
  // mark loading, fire Analytics{...}. Safe to call repeatedly; stale replies
  // are rejected by the reducer.
  refreshAnalytics: () => void
  // Local Analytics filter setters.
  setAnalyticsScope: (scope: AnalyticsScope) => void
  setAnalyticsRange: (range: AnalyticsRange) => void
  setAnalyticsMetric: (metric: AnalyticsMetric) => void
  // Open (or focus) the singleton extension-STORE tab (id 'store'). The StoreTab
  // fires browseStore + refreshInstalled on mount. Mirrors openGraphTab's shape.
  openStoreTab: () => void
  // Browse the store catalogue (optional full-text / category filters). Marks the
  // store busy + clears the last error, then StoreBrowse; the StoreCatalogue push
  // fills `catalogue` + clears busy. Also returns to the grid (clears `detail`).
  browseStore: (query?: string, category?: string) => void
  // Open one extension's detail (StoreDetail): marks busy, sets `detail` null
  // (grid→detail transition shows a spinner), fires the request. The
  // StoreItemDetail push fills `detail`.
  openStoreDetail: (id: string) => void
  // Back to the catalogue grid from a detail view (local only — no request).
  closeStoreDetail: () => void
  // Open (or focus) the installed-extension detail tab (Tab-B) for `id`: find-
  // or-create keyed by `extId`, fire GetInstalledExtensionDetail, and activate it.
  openInstalledExtensionTab: (id: string) => void
  // Install one extension by id (optional version). Marks that card's pendingOp,
  // fires InstallExtension; the ExtensionOpResult clears it + surfaces any error,
  // and the following InstalledExtensions push refreshes the registry.
  installExtension: (id: string, version?: string) => void
  // Uninstall one extension by id (same pendingOp lifecycle as install).
  uninstallExtension: (id: string) => void
  // Fetch the local installed registry (ListInstalledExtensions → the
  // InstalledExtensions push). Fired on StoreTab mount.
  refreshInstalled: () => void
  // Dismiss the store notice banner (browse/detail `error` + install/uninstall
  // `opResult`) without navigating. Wired to the banner's close button.
  clearStoreNotice: () => void
  // Open (or focus) one extension-contributed panel tab (id
  // `ext:${extId}:${panelId}`) — the ActivityBar's merged extension items call
  // this on click. No wire fetch: the tab renders a `koma://extension/...`
  // iframe, which loads itself.
  openExtensionTab: (extId: string, panelId: string, title: string) => void
  // (Re)load the FIRST page of the commit graph (replace mode): mark loading +
  // GitGraph{ limit:200, skip:0 }. Fired on GraphTab mount + its refresh button.
  refreshGraph: () => void
  // Append the NEXT page (append mode, skip = current commit count) when scrolled
  // near the bottom and `hasMore`. Guarded against a duplicate in-flight load via
  // the `loading` flag, and a no-op past the last page.
  loadMoreGraph: () => void
  // Select a commit (graph row / a parent-chip click): set selectedSha + fetch
  // its GitCommitDetail. Clears stale `detail` when the sha actually changes so
  // the detail pane shows a loading state, not the previous commit's detail.
  selectCommit: (sha: string) => void
  // Clear the current commit selection (closes the right detail pane): resets
  // `selectedSha`/`detail` back to null without touching the wire.
  clearSelection: () => void
  // Rail-line/Bubble mode switch (GK2) — local UI toggle, no wire request.
  setGraphMode: (mode: 'rail' | 'bubble') => void
  // ─── Import Graph tab actions ────────────────────────────────────────
  // Open (or focus) the singleton import-graph tab (id 'import-graph').
  openImportGraphTab: () => void
  // (Re)load the import graph: mark loading + fire ImportGraph req with
  // current controls (focus/depth/direction/filters). `path` overrides the focus.
  refreshImportGraph: (path?: string | null) => void
  // Set the traversal depth and optionally re-fetch.
  setImportGraphDepth: (depth: number) => void
  // Set the direction and optionally re-fetch.
  setImportGraphDirection: (dir: 'dependencies' | 'dependents' | 'both') => void
  // Click a node: set selected + refetch with new focus.
  selectImportGraphNode: (path: string | null) => void
  // Deselect + close detail pane.
  clearImportGraphSelection: () => void
  // Navigate to a specific breadcrumb entry (chained exploration).
  navigateBreadcrumb: (idx: number) => void
  // Go back one step in the breadcrumb chain.
  popBreadcrumb: () => void
  // Request impact analysis for a file.
  requestImportGraphImpact: (path: string) => void
  // Send ImportGraphReindex to the linker daemon; sets reindexBusy.
  // Coalesces (no-op) while already in flight.
  reindexImportGraph: () => void
  // Clear the breadcrumb trail (fresh start).
  clearBreadcrumb: () => void
  // Set workspace root filters and re-fetch. Empty = all roots.
  setImportGraphRootFilter: (roots: string[]) => void
  // Set language filters and re-fetch. Empty = all languages.
  setImportGraphLanguageFilter: (langs: string[]) => void
  // (Re)load the bubble/activity chart's commit series (GK5b): mark loading +
  // GitActivity{ path, limit:800 }. `path` narrows to one pathspec; omitted or
  // `null` means the whole active branch. Fired on GraphBubble mount + its
  // path-filter submit.
  refreshActivity: (path?: string | null) => void
  // Open (or focus) a Monaco diff tab for `path` at commit `sha` vs its first
  // parent — distinct `commitdiff:${sha}:${path}` id from openDiffTab/
  // openGitDiffTab (never collides). Marks loading + fires the GitCommitDiff req.
  openCommitDiffTab: (sha: string, path: string, oldPath?: string) => void
  push: (env: PushEnvelope) => void
  // JS -> Rust: typed request helper, tags the envelope { t: 'req', ...g }.
  req: (g: GuiReq) => void
  // Open `url` in the SYSTEM browser (never inside the webview) — fires the
  // host-local OpenExternal req. No reply, no store mutation. Used by the
  // Settings "Account" section's "Manage account on koma.run" link.
  openExternal: (url: string) => void
  openOmniSearch: () => void
  closeOmniSearch: () => void
  // Queue a workspace path for the Composer to insert into its draft text.
  insertToComposer: (path: string) => void
  /** Insert a coding path/dir `@` token and focus the chat tab + composer. */
  putCodingPathInChat: (root: string, path: string, opts?: { isDir?: boolean }) => void
  /** Stage a code-range paste pile (compact chip; body goes out on send). */
  askCodingSelectionInChat: (payload: { text: string; label: string; path: string }) => void
  /** Queue a diagram drawing for the composer. The model receives its Mermaid. */
  addDiagramToChat: (item: { title: string; mermaid: string; doc: DiagramDoc }) => void
  consumeDiagramChatQueue: () => void
  /** Register a marker-insert row before AttachFile / AttachPaste from diagram → chat. */
  stageComposerAttachmentInsert: (
    kind: 'image' | 'pasted_text',
    extra?: { name?: string; text?: string; path?: string },
  ) => string
  consumePendingComposerAttachmentInserts: () => void
  /** Queue a design slice as a chat chip. The model receives its html fence. */
  addDesignToChat: (item: { title: string; text: string }) => void
  consumeDesignChatQueue: () => void
  consumePasteBody: () => void
  // Composer-side ack: clears the one-shot signal after consuming it.
  consumeComposerInsert: () => void
  // Queue text to REPLACE the Composer draft (rewind refill). Called right after
  // a RewindTo request so the rewound message drops back into the composer.
  refillComposer: (text: string) => void
  // Composer-side ack: clears the refill one-shot after consuming it.
  consumeComposerRefill: () => void
  // Stage a rewind-on-send: remember the DISPLAY index of the message being edited
  // (the edit pencil). The Composer fires RewindTo(index) then Submit on send.
  stageRewind: (index: number) => void
  // Clear a staged rewind (send committed it, or the user emptied the composer).
  clearRewind: () => void
  // Pull one page of older history held on the host after a windowed Snapshot.
  requestHistoryPage: () => void
  // Bump scrollTick to force ChatView to jump to the bottom (on send).
  requestScrollBottom: () => void
  // Optimistically raise the session-swap overlay with the target's display
  // name. Called right before the SelectSession/NewSession request is sent.
  startSwitching: (name: string) => void
  // Best-effort cancel: dismisses the overlay locally. The in-flight swap on
  // the host side cannot be interrupted, so this only stops showing the
  // loader — the eventual Snapshot for the target session still lands and is
  // applied normally.
  cancelSwitching: () => void
  // Force any still-pending GUI bootstrap phases to `skipped` so a hung
  // settings/repos reply cannot keep attach chrome alive forever.
  skipBootstrapRemaining: () => void
  // Open the remote folder picker. Optimistic `listing` so the overlay + braille
  // spinner appear on the first click (SSH list_dirs can lag). No-ops while the
  // picker is already open so double-clicks don't spawn duplicate SSH listings.
  requestRemotePath: () => void
  dismissLoading: () => void
  // Dismiss the active toast (auto-dismiss timer, or a manual close). No-op if
  // the id no longer matches the current toast (a newer toast already replaced
  // it — its own timer owns the dismissal).
  dismissToast: (id: number) => void
  // Open (or focus) the singleton Settings tab (id 'settings'): find-or-create,
  // activate it, and fire GetSettings so its values refresh. Mirrors openDiffTab's
  // dedupe + activate shape.
  openSettingsTab: () => void
  // Open (or focus) the singleton Help tab (id 'help'): find-or-create, activate
  // it. No wire request — the Help tab is static content, unlike Settings.
  openHelpTab: () => void
  // Open (or focus) the singleton Tutorial tab (id 'tutorial').
  openTutorialTab: () => void
  // Send one Tutorial coach turn (host koma-free). Appends the user message
  // optimistically; assistant lands on TutorialChatDone.
  sendTutorialChat: (text: string) => void
  clearTutorialPendingTour: () => void
  clearTutorialError: () => void
  // ActivityBar drag-reorder: replace the persisted order wholesale (the caller
  // — ActivityBar's drop handler — computes the full reordered id list) and
  // write it through to localStorage.
  setActivityBarOrder: (order: string[]) => void
  // ActivityBar / Settings "Sidebar" section: toggle one item's hidden state
  // (hidden = moved into the "…" overflow menu, never removed) and write the
  // updated layout through to localStorage.
  setActivityBarHidden: (view: string, hidden: boolean) => void
  // Open (or focus) a per-agent editor tab: find-or-create keyed by agentId
  // (the agent's name, or `null` for a create — see the Tab union's 'agent'
  // member), activate it. Unlike Settings/Help this is NOT a singleton — a
  // different agentId opens a DIFFERENT tab (diff-tab-style dedupe).
  openAgentTab: (agentId: string | null) => void
  // Rebind an already-open agent tab's identity after a successful
  // create/rename (fired optimistically right after the SetAgent req, since
  // the wire gives no dedicated ack — just a fresh AgentsValues push). Updates
  // both the tab's `id` (so a later click on that agent's now-renamed row in
  // AgentsPanel still finds THIS tab instead of opening a duplicate) and its
  // `agentId`. No-op if `oldAgentId === newAgentId` (a save with no rename).
  renameAgentTab: (oldAgentId: string | null, newAgentId: string) => void
  // Open (or focus) a Monaco diff tab for a File-changed `path`: find-by-path or
  // create, mark it loading, fire the FileDiff req, and activate it. Re-opening
  // an already-open file refreshes it (same loading + re-request path).
  openDiffTab: (path: string) => void
  // Re-fetch the Source Control "GIT" panel's status (branch/ahead-behind +
  // staged/unstaged lists). Fired on GitPanel mount/(re)activation, and once
  // at boot (routes/index.tsx) so the footer's branch indicator populates
  // without ever opening the panel.
  refreshGitStatus: () => void
  // Open (or focus) a Monaco diff tab for a GIT-panel file row: `staged`
  // picks index-vs-HEAD (true) or worktree-vs-index (false) — distinct tab id
  // scheme (`gitdiff:${staged}:${path}`) from `openDiffTab`'s `diff:${path}`,
  // since a git diff needs BOTH staged and unstaged tabs open for the SAME
  // path without colliding. Marks loading + fires the GitDiff req.
  openGitDiffTab: (path: string, staged: boolean) => void
  // Update the GIT panel's commit-box draft text (controlled textarea).
  setCommitDraft: (text: string) => void
  // GIT panel mutations — stage/unstage/discard a batch of repo-root-relative
  // paths (a single path for a row action, every staged/unstaged path for a
  // "Stage All"/"Unstage All"/"Discard All Changes" header action), or commit
  // whatever is currently staged. Each fires the matching req; the reply lands
  // as a one-shot GitOp push (surfaced as an error toast on failure) followed
  // by a fresh GitStatus push that refreshes the panel's lists — these
  // actions never touch `git`/`commitDraft` optimistically themselves.
  gitStage: (paths: string[]) => void
  gitUnstage: (paths: string[]) => void
  gitDiscard: (paths: string[]) => void
  gitCommit: (message: string) => void
  // GIT panel key-picker: assign the repo to vault key `name`, or clear the
  // assignment (`null` — "Default (system ssh)"). No dedicated reply; a fresh
  // GitStatus push (host-side, always follows) reflects the new `keyName`.
  setGitKey: (name: string | null) => void
  // GIT panel sync toolbar: fetch/pull/push the repo's configured remote, using
  // its assigned key's SSH override if one is set. Each sets `remoteBusy` to its
  // op name BEFORE firing the req (disabling the toolbar + showing a spinner on
  // the active button); the matching GitOp reply clears it and toasts the
  // outcome (an error, or a short success confirmation using `message` if
  // present), followed by a fresh GitStatus push that refreshes ahead/behind.
  gitFetch: () => void
  gitPull: () => void
  gitPush: (mode?: GitPushMode) => void
  // Branch-switcher popover / graph context menu (G4): re-fetch every local +
  // remote-tracking branch. Sets `branchesLoading` before firing the req;
  // cleared by the matching `BranchList` reply.
  refreshBranches: () => void
  // Repo picker (multi-repo support): re-fetch every detected repository root
  // + which one is active.
  refreshRepos: () => void
  // Repo picker pick: switch the active repo to `root`. Optimistically
  // updates `activeRepoRoot` + clears the stale graph/activity slices
  // (preserving the graph view mode) before telling the host — each panel's
  // `activeRepoRoot`-keyed effect then refetches for the newly-active repo.
  setActiveRepo: (root: string) => void
  // Toolbar "Stash" button (GK4c): `git stash push`. Reply lands as a
  // one-shot GitOp push (toasted either way); the GitOp reducer follows up
  // with `refreshStashes()` (the working-tree change itself is already
  // covered by the host's own follow-up GitStatus push, so no explicit
  // refreshGitStatus here). This op never moves HEAD, so no graph refresh.
  gitStash: () => void
  // Toolbar "Pop" button (GK4c): `git stash pop`. May conflict — the
  // existing G5 conflict banner surfaces it via the host's follow-up
  // GitStatus push, same as `gitCherryPick`. Same reply/refresh pattern as
  // `gitStash`.
  gitStashPop: () => void
  // Toolbar mount / stash-op follow-up (GK4c): re-fetch every stash list
  // entry so the Stash/Pop buttons' counts stay correct.
  refreshStashes: () => void
  // Switch (or detach onto) `ref` — a branch name or a sha. SAFE only (never
  // `--force`); the reply lands as a one-shot GitOp push (toasted either way)
  // followed by a fresh GitStatus AND a graph refresh (HEAD moved).
  gitCheckout: (ref: string) => void
  // Create branch `name` from `start` (`null` = current HEAD), optionally
  // switching to it immediately (`checkout`). Same reply pattern as
  // `gitCheckout`.
  gitCreateBranch: (name: string, start: string | null, checkout: boolean) => void
  // Commit-graph row context menu "Cherry-pick commit" (G5c) — may conflict;
  // the follow-up GitStatus push's `inProgress`/`conflicted` carry that state.
  // Reply lands as a one-shot GitOp push (toasted either way), followed by a
  // fresh GitStatus AND graph refresh (see the `GitOp` reducer case).
  gitCherryPick: (sha: string) => void
  // Commit-graph row context menu "Revert commit" (G5c). Same reply pattern
  // as `gitCherryPick`.
  gitRevert: (sha: string) => void
  // Commit-graph row context menu "Reset <branch> to here" (G5c). `mode` is
  // 'soft'/'mixed'/'hard' — 'hard' DISCARDS uncommitted changes; the caller
  // gates this behind a strong inline confirm BEFORE calling this (this
  // action itself fires the request unconditionally). Same reply pattern as
  // `gitCherryPick`.
  gitReset: (sha: string, mode: 'soft' | 'mixed' | 'hard') => void
  // Branch-switcher / graph context menu "Merge into current branch" (G5c) —
  // may conflict, same reasoning as `gitCherryPick`. `ref` is a branch name or
  // a sha.
  gitMerge: (ref: string) => void
  // Rebase onto `upstream` (G5c/G6) — a branch name or a sha. `branch`
  // (G6 GitKraken-style drag-to-rebase — a dragged branch chip dropped onto a
  // commit/ref) checks out + rebases THAT branch instead of the current one;
  // omitted rebases the current branch (unchanged G5c behaviour). May
  // conflict, same reasoning as `gitCherryPick`.
  gitRebase: (upstream: string, branch?: string) => void
  // The conflict banner's Abort button (G5c): `kind` is `git.inProgress`
  // verbatim ('merge'/'rebase'/'cherry-pick'/'revert'). Same reply pattern as
  // `gitCherryPick`.
  gitOpAbort: (kind: string) => void
  // The conflict banner's Continue button (G5c). Same `kind` values and reply
  // pattern as `gitOpAbort` — git refuses (surfacing an error toast) if
  // conflicts remain.
  gitOpContinue: (kind: string) => void
  // Settings "SSH Keys" section: re-fetch the vault's key list. Fired on the
  // section opening/re-activating.
  refreshKeys: () => void
  // Generate a fresh passphrase-less ed25519 keypair. The reply lands as a
  // one-shot KeyOp push (toasted on failure) followed by a fresh KeyList push
  // that refreshes `keys` — this action never mutates `keys` optimistically.
  keyGenerate: (name: string, comment: string) => void
  // Import an existing pasted private key under `name`. Same reply pattern as
  // keyGenerate.
  keyImport: (name: string, privateKey: string) => void
  // Reveal a keypair's public (`private: false`) or private (`private: true`)
  // half. The reply lands as a one-shot KeyReveal push into `keyReveal`.
  keyReveal: (name: string, priv: boolean) => void
  // Dismiss the currently-shown reveal box (local-only — no wire request).
  clearKeyReveal: () => void
  // Delete a keypair (both halves, best-effort). Same reply pattern as
  // keyGenerate.
  keyDelete: (name: string) => void
  // Settings "Language servers": re-fetch catalogue status.
  refreshLsp: () => void
  // Install one server (or all managed when `all`).
  lspInstall: (id: string | null, all?: boolean, force?: boolean) => void
  // Uninstall a koma-managed server (never touches PATH copies).
  lspUninstall: (id: string) => void
  // Toggle the cross-tab Problems drawer above the footer.
  setProblemsOpen: (open: boolean) => void
  toggleProblemsOpen: () => void
  // Toggle the cross-tab Language Servers drawer (mutually exclusive with Problems).
  setLspDrawerOpen: (open: boolean) => void
  toggleLspDrawerOpen: () => void
  // Open coding file at a diagnostic location (uri may be file://).
  openDiagnostic: (uri: string, line: number, character: number) => void
  // Open (or focus) a read-only STREAM tab for a sub-agent (`kind:'subagent'`) or bash
  // job (`kind:'bash'`) by its numeric id: find-or-create (dedup by the stable
  // `sa:`/`bash:` id), activate it, and sync the stream view so the host starts streaming
  // THAT target's transcript / output tail.
  openStreamTab: (kind: 'subagent' | 'bash', targetId: number, title: string) => void
  // Open (or focus) an interactive terminal tab: if a terminal tab with this
  // terminalId already exists, just focus it; otherwise create a new tab and
  // send TerminalCreate to the host. `title` defaults to "Terminal" for the
  // first, "Terminal N" for subsequent ones.
  openTerminalTab: (terminalId: string, title: string) => void
  // Stream-view chokepoint: derive {subagent, bash} from the CURRENTLY-ACTIVE tab (a
  // stream tab → its target; anything else → both null) and send SetStreamView, so
  // exactly ONE stream view is ever active (the active stream tab, else none). Called
  // from openStreamTab / activateTab / closeTab / session-switch (the four paths that
  // can change which tab is active).
  syncStreamView: () => void
  // Close a tab (never 'chat'). If it was the active tab, activate the
  // adjacent (left) tab — tabs[0] is always the chat tab, so a fallback exists.
  // Dirty codingFile tabs are a no-op unless `force: true` (the floating dirty-
  // close popover collects the confirmation first).
  closeTab: (id: string, opts?: { force?: boolean }) => void
  // Activate a tab. Re-focusing a diff tab RE-REQUESTS its FileDiff for
  // freshness (contents may have changed since it was opened) while keeping the
  // stale diff on screen so the editor doesn't flash.
  activateTab: (id: string) => void
  // Focus a pane without changing which tab that pane shows.
  focusEditorGroup: (groupId: EditorGroupId) => void
  // Move/reorder one tab into an existing pane. `beforeId: null` appends it to
  // that pane's strip. The permanent chat tab cannot be moved.
  moveTabToGroup: (tabId: string, groupId: EditorGroupId, beforeId?: string | null) => void
  // Split `targetGroupId` into a nested pair and move `tabId` into the new leaf.
  splitTab: (
    tabId: string,
    targetGroupId: EditorGroupId,
    side: 'before' | 'after',
    dir: SplitDir,
  ) => void
  // Flip the parent split of `groupId` (focused leaf when omitted).
  toggleSplitDir: (groupId?: EditorGroupId) => void
  // Set the parent split axis of `groupId`. No-op when unsplit or already that dir.
  setSplitDir: (dir: SplitDir, groupId?: EditorGroupId) => void
  // Resize the divider of a split node.
  resizeEditorGroups: (splitId: string, deltaPx: number, totalPx: number) => void
  // The UsageFooter PLAN badge click (Plan mode only): bump `focusPlanTick` so
  // RootLayout opens the Explore sidebar/panel and ExplorePanel expands its
  // PLAN section in response.
  focusPlanSection: () => void
  // The Sidebar Usage-panel header's all/session segmented control: switch
  // scope. UsagePanel re-requests on the resulting change.
  setUsageScope: (scope: 'all' | 'session') => void
  // Manual re-fetch trigger for the Sidebar Usage-panel header's refresh
  // button — fires the same UsagePreview req UsagePanel's mount/scope-change
  // effect uses, for the CURRENT usageScope + attached session. Safe to call
  // repeatedly (e.g. spam-clicking the button).
  refreshUsagePreview: () => void
  // Fire a GetMcpStatus request and mark the MCP status as busy. The
  // requestId is minted here and echoed back by the McpStatus reply for
  // stale-reply protection. Safe to call repeatedly (e.g. spam-clicking).
  refreshMcpStatus: () => void
  // Mark a session id "dying" right after firing its KillSession ('kill') or
  // DeleteSession ('delete') req (ResumePalette/StartScreen confirm).
  // Idempotent — marking the same id+kind twice (or a race) never duplicates
  // the entry.
  markDying: (id: string, kind: 'kill' | 'delete') => void
  // Kill-the-ATTACHED-session fast path: KillSession on the foreground session
  // sends the host straight to the swapper WITHOUT ever emitting a Snapshot
  // (only Hub pushes follow), so `session.id` would otherwise stay stale
  // forever and IndexPage would keep rendering the dead chat. Call this right
  // after firing that KillSession req to reset the session slice to
  // `initialSession` locally (hub/dyingSessions untouched — the follow-up Hub
  // push still needs to land to move the row into History) and clear any
  // per-session UI state that would otherwise render stale (tabs back to just
  // chat, active tab back to 'chat', any stuck switching overlay), mirroring
  // the Snapshot handler's `switched` branch. IndexPage's `sessionId === null`
  // gate then falls back to StartScreen immediately instead of waiting on a
  // push that isn't coming.
  detachSession: () => void
  // Set agentSaving to track a pending save (called right before the SetAgent
  // request is sent). `tabId` is the agent tab's client-local id, `originalName`
  // is the pre-edit name (null for create), `newName` is the trimmed final name.
  // Returns the assigned seq so the caller can pass it in the GuiReq.
  setAgentSaving: (tabId: string, originalName: string | null, newName: string) => number
  // Clear agentSaving (called on confirmation — AgentsValues success that
  // includes the saved agent, or AgentOp error toast). Takes the expected
  // `seq` to guard against stale clears; an undefined or mismatched seq is
  // a no-op.
  clearAgentSaving: (seq?: number) => void
  // ─── Coding panel ──────────────────────────────────────────────────────
  setActiveCodingRoot: (root: string | null) => void
  // Open (or focus) a coding file tab. Optional placement drives explorer DnD:
  // drop on a pane center / tab strip → that group; drop on a pane edge → split.
  openCodingFile: (
    root: string,
    path: string,
    opts?: {
      force?: boolean
      preview?: boolean
      groupId?: EditorGroupId
      beforeId?: string | null
      split?: { side: 'before' | 'after'; dir: SplitDir }
    },
  ) => void
  openLocalFileTab: (absPath: string, title: string) => void
  saveCodingFile: (root: string, path: string) => void
  revertCodingFile: (root: string, path: string) => void
  updateCodingContent: (root: string, path: string, content: string) => void
  createCodingItem: (root: string, path: string, kind: 'file' | 'dir') => void
  renameCodingItem: (root: string, oldPath: string, newPath: string) => void
  deleteCodingItem: (root: string, path: string) => void
  /** Drag-upload / remote upload: write raw file bytes under root/path. */
  uploadCodingFile: (root: string, dirPath: string, file: File, overwrite?: boolean) => Promise<void>
  /** Download / save-as: fetch bytes and trigger a browser download. */
  downloadCodingFile: (root: string, path: string) => void
  refreshCodingDir: (root: string, path: string) => void
  clearCodingConflict: (root: string, path: string) => void
  // Content search pane
  setCodingSearchQuery: (query: string) => void
  setCodingSearchReplace: (replace: string) => void
  setCodingSearchFlag: (
    flag: 'caseSensitive' | 'wholeWord' | 'isRegex' | 'replaceOpen' | 'filtersOpen',
    value: boolean,
  ) => void
  setCodingSearchGlobs: (includeGlob: string, excludeGlob: string) => void
  searchCodingContent: (root: string) => void
  replaceCodingContentAll: (root: string) => void
  openCodingSearchHit: (root: string, path: string, line: number, col?: number) => void
  // ─── Diagram designer ──────────────────────────────────────────────────
  diagram: DiagramSlice
  // Open or focus one diagram tab. Each `.diag` file is its own tab.
  openDiagramTab: (root: string, path: string) => void
  saveDiagram: (root: string, path: string) => void
  updateDiagram: (root: string, path: string, doc: DiagramDoc) => void
  // `path` is `.koma/<name>.diag`. Creates the file, then seeds an empty document.
  createDiagramFile: (root: string, path: string) => void
  // ─── UI designer ───────────────────────────────────────────────────────
  design: DesignSlice
  openDesignTab: (root: string, path: string) => void
  saveDesign: (root: string, path: string) => void
  updateDesign: (root: string, path: string, doc: DesignDoc) => void
  // `path` is `.koma/<name>.kdsgn`. Creates the file, then seeds an empty document.
  createDesignFile: (root: string, path: string) => void
  setDesignPanelTab: (id: string | null) => void
  setDesignFileUi: (
    root: string,
    path: string,
    patch: Partial<import('./design').DesignFileUiState>,
  ) => void
}

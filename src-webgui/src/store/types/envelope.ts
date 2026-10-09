import type { SkillCatalogueEntry, SkillDetail, SkillItemOutcome } from './skills'
import type { GitReply } from '../../lib/gitWorkbench'
import type { LspDiagnostic } from '../../lib/lsp-bridge'
import type { ContentSearchFileHit, FileTreeEntry } from '../coding'
import type { AnalyticsModelRow, AnalyticsSeriesPoint, AnalyticsStatus } from './analytics'
import type { ChatMessage, PaletteColors, PaletteInfo, UsageDayEntry, UsageModelEntry } from './chat'
import type { ActivityCommit, BranchInfo, CommitFile, GitCommitNode, GitFileEntry, GitPushMode, KeyInfo, RepoEntry, StashEntry } from './git'
import type { InstalledExt, InstalledExtDetail, StoreDetail, StoreItem } from './marketplace'
import type { AttachmentEntry, BashJobEntry, FileChangeEntry, HubCookingEntry, HubHistoryEntry, LoadPhase, LspRuntimeServer, LspServerStatus, ModelRoute, PendingCall, PlanTodoEntry, SearchResultEntry, SubAgentEntry } from './session'
import type { ImportGraphEdge, ImportGraphNode, ImportGraphRootInfo } from './slices'
import type { McpServer, Model, ModelListEntry, Provider, RouteEntry } from '../../types/config'

export type PushEnvelope = PushEvent & { eventId?: string }
export type PushEvent =
  | { k: 'Notifications'; reply: import('../../lib/notifications').NotificationReply }
  | ({ k: 'WebSearchValues' } & import('../../types/web-search').SearchReply)
  | { k: 'Computer'; status: import('../../types/computer').ComputerStatus }
  | { k: 'ComputerPreview'; frame: import('../../types/computer').ComputerPreviewFrame }
  | { k: 'ComputerError'; message: string }
  | import('../../lib/coding-service').CodingReply
  | { k: 'CodingEvent'; clientId?: string; workspace: import('../../lib/coding-service').WorkspaceRef; event: { k: string; [key: string]: unknown } }
  | ({ k: 'GitWorkbench' } & GitReply)
  | {
      k: 'Snapshot'
      session: string
      state: string
      messages: ChatMessage[]
      // Full projected transcript length (incl. not-yet-pushed older head).
      messageCount?: number
      title: string
      palette: PaletteColors
      subagents: SubAgentEntry[]
      bash: BashJobEntry[]
      // Cumulative file-change log (#24). Optional-tolerant: a host build that
      // doesn't project it yet omits it, and the panel shows "No changes".
      fileChanges?: FileChangeEntry[]
      // Plan-mode todo checklist (Explore "PLAN" section). Optional-tolerant:
      // a host build that doesn't project it yet leaves the panel's PLAN
      // section empty (as if no plan were in progress).
      planTodos?: PlanTodoEntry[]
      attachments: AttachmentEntry[]
      // Global agent mode token ("auto"/"normal"/"plan"/"yolo"), projected from
      // the host's process-global agent_mode. Optional-tolerant: a host build
      // that doesn't project it yet leaves the store's current mode untouched.
      mode?: string
      // Queued mid-turn steer messages (host `SessionSnapshot.pending_steer`):
      // messages submitted while the turn is cooking, capped at 5 daemon-side.
      // Full text (clients ellipsize at render). Optional-tolerant: a host build
      // that doesn't project it yet leaves the store's queue empty.
      pendingSteer?: string[]
      // Approval/plan-decision gate (host `awaiting_approval` — approval.rs).
      // True when the turn is PARKED waiting on a y/a/n decision. The paused
      // call rides along in `pendingCall` (name/args); `approvalReason` is the
      // classifier's `verdict.reason` for a risky pause (null for a plan_ready
      // pause or a non-classifier park). Optional-tolerant: a host build that
      // doesn't project these yet leaves the gate closed.
      awaitingApproval?: boolean
      loadedSkillNames?: string[]
      modelRoutes?: ModelRoute[]
      approvalReason?: string | null
      pendingCall?: PendingCall | null
      // SDLC projection fields (mode=sdlc only; absent/undefined otherwise).
      sdlcPhase?: string | null
      sdlcGoal?: string | null
      sdlcBranch?: string | null
      sdlcOpen?: number | null
      sdlcSealed?: number | null
    }
  // Swap-START signal pushed the instant a Select/New is acted on host-side,
  // BEFORE teardown, so the loader rises deterministically across the
  // uninterruptible attach gap (matches Rust PushEnvelope::Switching { to }).
  // `to` is the target session id/uuid — resolved to a friendly hub label,
  // falling back to any optimistic label already raised, then a generic one.
  | { k: 'Switching'; to: string }
  // Append-only transcript growth (same session); concat onto messages.
  | { k: 'SnapshotTail'; session: string; messages: ChatMessage[] }
  // Prepend older history after a truncated first Snapshot (same session).
  // Host may chunk: `more` means further SnapshotHead frames will follow.
  | {
      k: 'SnapshotHead'
      session: string
      messages: ChatMessage[]
      more?: boolean
      totalOlder?: number | null
    }
  // On-demand older history page (pull). Prepend like SnapshotHead.
  | {
      k: 'HistoryPage'
      session: string
      messages: ChatMessage[]
      hasMore?: boolean
    }
  // In-place last-message update (tool result join, etc.).
  | { k: 'SnapshotSetLast'; session: string; message: ChatMessage }
  | { k: 'StreamMsg'; session: string; text: string }
  // Incremental stream: reset replaces the bubble; else append. Empty append+reset clears.
  | { k: 'StreamDelta'; session: string; reset: boolean; append: string }
  | { k: 'Reasoning'; session: string; text: string }
  | { k: 'ReasoningDelta'; session: string; reset: boolean; append: string }
  // `toast` is the transient message text (safeguard/harness/classifier notices
  // + generic host toasts). `kind` is the severity token ("error"/"info") the
  // host now carries alongside the text so the GUI can colour error vs info —
  // optional-tolerant: a host build that doesn't project it yet defaults to info.
  // The usage fields (tokensIn/tokensCached/tokensOut/cost/contextWindow/mode)
  // drive the chat column's UsageFooter statusline; optional-tolerant for an
  // older host build that doesn't project them yet (default 0 / 'auto' in the reducer).
  | {
      k: 'Status'
      session: string
      working: boolean
      toastSession?: string | null
      toastEventId?: string
      toast: string | null
      toastKind?: string
      tokensIn?: number
      tokensCached?: number
      tokensOut?: number
      cost?: number
      // Effective context window for the usage card. Optional: an older host
      // omits it and the reducer keeps the previous value.
      contextWindow?: number
      mode?: 'auto' | 'normal' | 'plan' | 'yolo' | 'sdlc'
    }
  // Resident-memory sample from the GUI process itself (not the session
  // daemon). Pushed when a 1 MiB bucket changes.
  | {
      k: 'UsageLive'
      memWindow?: number
      memAgent?: number
      memServices?: number
      memSystem?: number
    }
  | {
      k: 'Hub'
      state: string
      cooking: HubCookingEntry[]
      history: HubHistoryEntry[]
    }
  | { k: 'SearchResults'; query: string; items: SearchResultEntry[] }
  | { k: 'PasteBody'; markerN: number; text: string }
  | { k: 'AttachmentLocated'; markerN: number; absPath: string; relPath: string; name: string }
  // Authoritative config projection (mcp/providers/models) — global, not
  // per-session. REPLACES the whole config slice, pushed on config change and
  // on (re)attach. Also carries the active palette (theme) — Config is pushed
  // in BOTH the empty/swapper state and the attached state (render.rs
  // `PushEnvelope::Config.palette`), so it's the one push the empty/swapper
  // state — which never emits a Snapshot — can rely on to repaint to
  // config.json's theme instead of falling back to the dark default.
  | {
      k: 'Config'
      mcp: McpServer[]
      providers: Provider[]
      models: Model[]
      palette?: PaletteColors
      // Onboarding gate: the host's authoritative first-run flag (Rust
      // `Mode::Onboard` — no usable Main route). Optional-tolerant: a host
      // build that doesn't project it yet leaves it undefined, and the UI
      // derives first-run from an empty/unconfigured config instead.
      firstRun?: boolean
      // Active theme (palette) name — the currently-selected key in the host's
      // named-palette registry (theme.rs). Drives the onboarding theme picker's
      // active row. Optional-tolerant.
      theme?: string
      // Available theme (palette) names the host advertises (theme.rs
      // registry). The onboarding picker lists these; falls back to a bundled
      // KNOWN_THEMES list when the host omits them.
      themes?: string[]
      // Full palette catalogue WITH resolved colours (host `PushPaletteInfo`),
      // for the Settings tab's Appearance grid. Optional-tolerant: absent on a
      // host build that doesn't project it yet (the grid then falls back to the
      // names-only `themes` list rendered as label chips).
      palettes?: PaletteInfo[]
    }
  // Reply to GuiReq ListModels — live per-provider model-id catalogue. Field
  // is `models` to match the daemon's PushEnvelope::ModelList { provider, models }.
  | { k: 'ModelList'; provider: string; models: ModelListEntry[] }
  // Reply to GuiReq ListRoutes — live per-model OpenRouter endpoint list. Echoes
  // the provider+modelId it was fetched for so ModelForm can discard a stale
  // reply that no longer matches its current selection. Empty `routes` = a
  // non-OpenRouter provider (UI shows only the synthetic "Auto" row).
  | { k: 'RouteList'; provider: string; modelId: string; routes: RouteEntry[] }
  // Reply to GuiReq FileDiff — the original/modified contents of a File-changed
  // path, for a Monaco diff tab. Echoes the `path` it was fetched for (the tab
  // key). A reply is guaranteed for every request; the reducer ignores a reply
  // whose tab was closed meanwhile.
  | {
      k: 'FileDiff'
      path: string
      original: string
      modified: string
      error: string | null
      binary: boolean
      origin?: 'git' | 'baseline'
    }
  // Reply to GuiReq UsagePreview — a LAST-7-DAYS usage preview computed straight
  // off the global usage ledger (host-only, never touches the daemon). ALWAYS a
  // reply so the Usage panel's loading state can never hang. `scope` echoes the
  // request's "all"/"session" token, and `sessionId` echoes the session uuid
  // ACTUALLY queried (null for an "all" scope) — together they let the reducer drop
  // a reply that no longer matches what's currently selected/attached: a rapid
  // all/session toggle racing an in-flight request (scope mismatch), OR the
  // foreground session switching mid-flight while "session" scope stayed selected
  // (session id mismatch — otherwise session A's numbers would render under B's
  // attach).
  | {
      k: 'UsagePreview'
      cost: number
      tokensIn: number
      tokensCached: number
      tokensOut: number
      calls: number
      days: UsageDayEntry[]
      topModels: UsageModelEntry[]
      scope: string
      sessionId: string | null
    }
  // Reply to GuiReq Analytics — host-computed usage dashboard (KPI totals,
  // series, models, main-vs-sub role split). ALWAYS a reply so the Analytics
  // tab never hangs. Echoes every correlation input (`reqSeq`/`scope`/
  // `sessionId`/`range`/`metric`) so the reducer can drop a stale reply.
  // `status` is "ok" | "empty" | "error" — empty is a successful zero-call
  // window, not a failure.
  | {
      k: 'Analytics'
      reqSeq: number
      scope: string
      sessionId: string | null
      range: string
      metric: string
      status: AnalyticsStatus
      error: string | null
      cost: number
      tokensIn: number
      tokensCached: number
      tokensOut: number
      calls: number
      cacheRate: number
      series: AnalyticsSeriesPoint[]
      models: AnalyticsModelRow[]
      mainCost: number
      mainCalls: number
      subCost: number
      subCalls: number
    }
  // Reply to GuiReq GetSettings (and the re-push after SetPrefs) — the Settings
  // tab's Session-section values + active palette. Guaranteed for every request
  // (even detached: the host answers from global config with defaults).
  | {
      k: 'SettingsValues'
      name: string
      workdir: string[]
      shortSend: boolean
      slidingCache: boolean
      bashSaving: boolean
      codingAutosave: boolean
      internetMode: string
      palette: string
      effort: string
      subagentMaxTurns: number
      shortSendEngageN: number
      shortSendTailN: number
      maxOutputTokens?: number
      contextWindowLimit?: number
      contextModelAlias?: string
      extraSkillRoots?: string[]
    }
  // Reply to GuiReq GetEffortOptions — the composer EffortPicker's derived
  // `/effort` menu for the foreground session's current model. ALWAYS a reply
  // (loading/unsupported/ready) so the picker never hangs.
  | { k: 'EffortOptions'; options: string[]; selected: number; note: string; state: 'loading' | 'unsupported' | 'ready' }
  // TUI-parity startup splash (cold-session warm-up): the host's two
  // background warm-up phases — indexing the workspace and reading project
  // docs (awareness). `active` false means no warm-up in flight (or it just
  // finished) — the reducer clears `ui.loading` to null in that case rather
  // than storing a "false" splash. Pushed independently of Snapshot/Switching
  // so the splash can keep showing (and finish its phase lines) even after
  // the attach itself has landed and `ui.switchingTo` has already cleared.
  | { k: 'Loading'; active: boolean; workspace: LoadPhase; awareness: LoadPhase }
  // Reply to GuiReq GetAgents (and the re-push after every SetAgent/
  // DeleteAgent) — the Agents dashboard's full agent list + model/provider
  // catalogues. ALWAYS a reply, even un-attached (host answers from built-in +
  // global config only). MIXED CASING, verified against the Rust wire: the
  // envelope's OWN fields are camelCase (`agents`/`catalogueModels`/
  // `catalogueProviders`/`availableTools`), but each nested entry struct has
  // NO rename_all of its own and serializes plain snake_case (`model_uuid`,
  // `model_id`, `provider_uuid`) — NOT a typo, the push case normalizes these
  // into the camelCase `AgentEntry`/`CatalogueModelEntry`/
  // `CatalogueProviderEntry` shapes above. `availableTools` is optional-
  // tolerant (defaults to [] in the reducer) for an older host build that
  // doesn't project it yet — the AgentTab tools chip grid then just has
  // nothing to offer beyond whatever an existing agent already carries.
  | {
      k: 'SkillValues'
      requestId: string
      sessionEpoch: number
      skills: SkillCatalogueEntry[]
      loadedSkillNames: string[]
      error: string | null
    }
  | {
      k: 'SkillDetailValues'
      requestId: string
      sessionEpoch: number
      tabId: string
      detail: SkillDetail | null
      filePath: string | null
      fileContent: string | null
      error: string | null
    }
  | {
      k: 'SkillOp'
      requestId: string
      sessionEpoch: number
      tabId: string
      operation: string
      outcomes: SkillItemOutcome[]
      loadedSkillNames: string[]
    }
  | {
      k: 'AgentsValues'
      reqSeq: number // 0 = no correlation (read-only fetch / host-built fallback)
      agents: {
        name: string
        description: string
        conditions: string
        source: string
        model_uuid: string | null
        model: string | null
        tools: string[]
        prompt: string
        ext_id?: string | null
      }[]
      catalogueModels: { uuid: string; name: string; model_id: string; provider_uuid: string }[]
      catalogueProviders: { uuid: string; name: string; endpoint: string }[]
      availableTools?: string[]
    }
  // Reply to GuiReq GetOAuthState (and the re-push after every StartOAuth
  // progress tick / SubmitOAuthPaste / CancelOAuth / DeleteOAuthConn) — the
  // OAuth login screen's full state: which phase the flow is in + its
  // phase-specific fields + the connections/providers lists. ALWAYS a reply to
  // GetOAuthState, even un-attached (host answers from disk + the provider
  // registry). Same MIXED CASING as AgentsValues: the envelope's OWN fields
  // are camelCase (`userCode`/`verificationUrl`), but the nested `conns`/
  // `providers` entry structs have no rename_all of their own and serialize
  // plain snake_case (`account_id`) — normalized to camelCase `OAuthConn` in
  // the push case.
  | {
      k: 'OAuthState'
      phase: string
      url: string | null
      userCode: string | null
      verificationUrl: string | null
      error: string | null
      conns: { uuid: string; name: string; provider: string; email: string; plan: string; account_id: string }[]
      providers: { id: string; label: string; kind: string }[]
    }
  // Extension STORE replies. Unlike the OAuth push, the nested wire structs are
  // ALREADY camelCase on the wire (StoreItemWire/StoreDetailWire/InstalledExtWire
  // carry `#[serde(rename_all="camelCase")]`), so these map straight to the
  // StoreItem/StoreDetail/InstalledExt store types with no snake_case normalize.
  // Reply to StoreBrowse — the catalogue grid. `error` set (items empty) on a
  // store network/parse failure so the grid shows an error, not a hang.
  | { k: 'StoreCatalogue'; items: StoreItem[]; error: string | null }
  // Reply to StoreDetail — one extension's full detail. `detail` null (+ error)
  // when the fetch failed / the id was unknown.
  | { k: 'StoreItemDetail'; detail: StoreDetail | null; error: string | null }
  // Reply to ListInstalledExtensions + the re-push after a successful install/
  // uninstall — the local registry for the "Installed" section.
  | { k: 'InstalledExtensions'; items: InstalledExt[] }
  // Reply to GetInstalledExtensionDetail — full detail of one installed extension.
  // `id` echoes the requested extension id for stale-reply protection.
  | { k: 'InstalledExtensionDetail'; id: string; detail: InstalledExtDetail | null; error: string | null }
  // Result of an install/uninstall op (echoes `id` to clear that card's pending
  // spinner). On success the authoritative registry reply is the following
  // InstalledExtensions push; `ok:false` + `error` surfaces a failure.
  | { k: 'ExtensionOpResult'; id: string; ok: boolean; error: string | null }
  // Reply to TutorialChat — coach text + optional tour id (host strips TOUR: trailer).
  | {
      k: 'TutorialChatDone'
      id: string
      text: string
      tour: string | null
      error: string | null
    }
  // Out-of-band reply to a GuiReq ExtPanelMsg (W9 panel bridge) — the
  // extension's `panel.msg` invoke outcome, re-pushed by the host from the
  // daemon's ExtPanelReply. Routed straight to the matching panel iframe via
  // postToPanel (lib/panelBridge.ts) by `reqId`; never touches store state.
  | {
      k: 'ExtPanelReply'
      extId: string
      panelId: string
      reqId: string | null
      ok: boolean
      payload?: unknown
      error?: string | null
    }
  // Unsolicited daemon->panel push (W9 panel bridge) — re-pushed by the host
  // from the daemon's ExtPanelPush so a panel iframe's live UI can update
  // without a request. Routed straight to the matching panel iframe via
  // postToPanel; never touches store state.
  | { k: 'ExtPanelPush'; extId: string; panelId: string; payload: unknown }
  // Reply to GuiReq GitStatus — host-computed branch/ahead/behind + staged/
  // unstaged file lists for the Source Control "GIT" panel. Carries the
  // Rust `GitStatusResult` verbatim (already camelCase) flattened onto the
  // envelope (a `#[serde(tag = "k")]` newtype variant). ALWAYS a reply so the
  // panel never hangs loading — `error` set means not a git repository.
  | {
      k: 'GitStatus'
      root: string | null
      branch: string | null
      detached: boolean
      ahead: number | null
      behind: number | null
      staged: GitFileEntry[]
      unstaged: GitFileEntry[]
      error: string | null
      keyName: string | null
      // G5c additions — see GitStatus type comments.
      inProgress: string | null
      conflicted: GitFileEntry[]
      pushMode: GitPushMode | null
    }
  // Reply to GuiReq GitDiff — a host-computed git diff for one GIT-panel file
  // row, for a Monaco diff tab. `staged` echoes the request (index-vs-HEAD vs
  // worktree-vs-index) so the reducer applies it to the matching
  // `gitdiff:${staged}:${path}` tab, never the wrong one.
  | {
      k: 'GitDiff'
      path: string
      staged: boolean
      original: string
      modified: string
      error: string | null
      binary: boolean
    }
  // Reply to a GitStage/GitUnstage/GitDiscard/GitCommit/GitFetch/GitPull/GitPush
  // mutation. `op` is "stage"/"unstage"/"discard"/"commit"/"fetch"/"pull"/"push";
  // `error` (only when `ok` is false) is git's own failure message. `message`
  // (wave 4b remote ops only — a short human-readable SUCCESS summary, e.g. a
  // fetch/pull/push's own stdout/stderr) is present only when the host had
  // something worth surfacing; absent (undefined) for every local mutation and
  // for a remote op with nothing to say. Carries no list data — ALWAYS
  // immediately followed by a fresh GitStatus push, which is what actually
  // refreshes the panel.
  | {
      k: 'GitOp'
      ok: boolean
      op: string
      error: string | null
      message?: string
    }
  // Reply to GuiReq GitGraph — a host-computed paginated commit graph across
  // every ref (GitKraken-style tab). Carries `GitGraphResult` verbatim (already
  // camelCase) flattened onto the envelope (a `#[serde(tag = "k")]` newtype
  // variant). `head` is the current HEAD sha (null when unresolved); `hasMore`
  // hints more history exists past this page (scroll-load-more). `error` set
  // means not a git repository (`commits` then empty). ALWAYS a reply.
  | {
      k: 'GitGraph'
      commits: GitCommitNode[]
      head: string | null
      hasMore: boolean
      error: string | null
    }
  // Reply to GuiReq GitCommitDetail — one commit's full metadata (incl. body) +
  // first-parent changed-file list, for the graph's detail pane. Carries
  // `CommitDetailResult` verbatim (already camelCase) flattened onto the
  // envelope. `sha` echoes the request so the reducer can drop a stale reply for
  // a since-changed selection.
  | {
      k: 'CommitDetail'
      sha: string
      author: string
      email: string
      date: string
      subject: string
      body: string
      parents: string[]
      files: CommitFile[]
      error: string | null
    }
  // Reply to GuiReq GitCommitDiff — one file's diff at `sha` vs its first parent,
  // for a Monaco diff tab. SEPARATE envelope + tab-id scheme
  // (`commitdiff:${sha}:${path}`) from GitDiff (working-tree/index) so a
  // commit-history diff never collides with a Source-Control one. `sha`/`path`
  // echo the request so the reducer applies it to the matching tab.
  | {
      k: 'CommitDiff'
      sha: string
      path: string
      original: string
      modified: string
      error: string | null
      binary: boolean
    }
  // Reply to GuiReq KeyList — the Settings "SSH Keys" section's authoritative
  // vault list. ALWAYS a reply so the section never hangs loading (an empty
  // vault is itself a valid "no keys yet" state). Also arrives as the
  // follow-up refresh after any KeyGenerate/KeyImport/KeyDelete mutation.
  | {
      k: 'KeyList'
      keys: KeyInfo[]
    }
  // Reply to GuiReq KeyReveal — a host-computed keypair reveal for the "Copy
  // public key" / "Reveal private key" actions. `private` echoes the request
  // so the reducer never mismatches a public reveal with a private one.
  | {
      k: 'KeyReveal'
      name: string
      private: boolean
      content: string
      error: string | null
    }
  // Reply to a KeyGenerate/KeyImport/KeyDelete mutation. `op` is
  // "generate"/"import"/"delete"; `error` (only when `ok` is false) is the
  // host's own failure message. Carries no list data — ALWAYS immediately
  // followed by a fresh KeyList push, which is what actually refreshes the
  // section's list.
  | {
      k: 'KeyOp'
      ok: boolean
      op: string
      error: string | null
    }
  // Settings "Language servers": full catalogue status (managed / PATH / missing).
  | {
      k: 'LspStatus'
      servers: LspServerStatus[]
    }
  // Install/uninstall progress for one server. pct 0–100; error set = failure.
  | {
      k: 'LspInstall'
      id: string
      pct: number
      error: string | null
    }
  | {
      k: 'LspDiagnostics'
      uri: string
      diagnostics: LspDiagnostic[]
    }
  | {
      k: 'LspCompletion'
      requestId: string
      items: import('../../lib/lsp-bridge').LspCompletionItem[]
      isIncomplete?: boolean
      error: string | null
    }
  | {
      k: 'LspCompletionResolve'
      requestId: string
      item: import('../../lib/lsp-bridge').LspCompletionItem | null
      error: string | null
    }
  | {
      k: 'LspHover'
      requestId: string
      hover: import('../../lib/lsp-bridge').LspHover | null
      error: string | null
    }
  | {
      k: 'LspDefinition'
      requestId: string
      locations: import('../../lib/lsp-bridge').LspLocation[]
      error: string | null
    }
  | {
      k: 'LspReferences'
      requestId: string
      locations: import('../../lib/lsp-bridge').LspLocation[]
      error: string | null
    }
  | {
      k: 'LspDocumentSymbol'
      requestId: string
      symbols: import('../../lib/lsp-bridge').LspDocumentSymbol[]
      error: string | null
    }
  // Live language-server runtime (starting / indexing / ready / error).
  | {
      k: 'LspRuntime'
      servers: LspRuntimeServer[]
      replace?: boolean
      removed?: string[]
    }
  // Reply to GuiReq GitBranchList (G4) — every local + remote-tracking branch
  // for the branch-switcher popover / graph context menu. Carries
  // `BranchListResult` verbatim (already camelCase) flattened onto the
  // envelope. ALWAYS a reply so the picker never hangs loading.
  | {
      k: 'BranchList'
      branches: BranchInfo[]
      error: string | null
      root: string | null
      requestId?: number | null
    }
  // Reply to GuiReq GitRepos (multi-repo support) — every detected repository
  // root in the workspace + which one is currently active. Carries
  // `RepoEntry[]` verbatim (already camelCase) flattened onto the envelope.
  // ALWAYS a reply so the picker never hangs loading.
  | { k: 'RepoList'; repos: RepoEntry[]; active: string | null }
  // Reply to GuiReq GitStashList (GK4c) — every `git stash list` entry for the
  // toolbar's Stash/Pop buttons. Carries `StashListResult` verbatim (already
  // camelCase) flattened onto the envelope. ALWAYS a reply so a non-repo
  // workdir just shows an empty (Pop-disabled) list rather than hanging.
  | {
      k: 'StashList'
      entries: StashEntry[]
      error: string | null
    }
  // Reply to GuiReq GitActivity (GK5b) — per-commit author/date/lines-changed
  // rows for the bubble/activity chart. Carries `ActivityResult` verbatim
  // (already camelCase) flattened onto the envelope. `error` set means the
  // workdir isn't a git repository (`commits` then empty). `path` echoes the
  // request's pathspec (`null` for the whole-branch case) so the reducer can
  // drop a stale reply for a since-changed path filter. ALWAYS a reply.
  | {
      k: 'Activity'
      commits: ActivityCommit[]
      path: string | null
      error: string | null
    }
  // Result of a daemon-side SetAgent/DeleteAgent operation (attached path).
  // `ok: false` + `error` surfaces the failure as a toast and clears the
  // AgentTab's saving state. On success the authoritative reply is always a
  // fresh `AgentsValues` push, so this envelope only carries failures — the
  // AgentsValues push handler below is what the AgentTab watches for success.
  | { k: 'AgentOp'; ok: boolean; error: string | null; reqSeq: number }
  // One-shot MCP runtime status reply: per-server connection state (tool counts
  // + errors) with optional top-level availability error. Echoes `requestId` so
  // the store can discard a stale reply. The frontend merges `servers` into
  // `config.mcp` by id — this does NOT replace the whole config slice.
  | {
      k: 'McpStatus'
      requestId: string
      servers: { id: string; connected: boolean; toolCount: number; error?: string }[]
      globalError?: string
    }
  // Coding panel workspace file operations (see GuiReq FileTree/FileRead/…).
  // Every reply echoes root/path/requestId for stale-reply rejection.
  | {
      k: 'FileTree'
      root: string
      path: string
      requestId: string
      entries: FileTreeEntry[]
      error: string | null
    }
  | {
      k: 'FileRead'
      root: string
      path: string
      requestId: string
      content: string | null
      fingerprint: string
      binary: boolean
      tooLarge: boolean
      error: string | null
    }
  | {
      k: 'FileSave'
      root: string
      path: string
      requestId: string
      fingerprint: string
      error: string | null
    }
  | {
      k: 'FileCreate'
      root: string
      path: string
      requestId: string
      error: string | null
    }
  | {
      k: 'FileRename'
      root: string
      oldPath: string
      newPath: string
      requestId: string
      error: string | null
    }
  | {
      k: 'FileDelete'
      root: string
      path: string
      requestId: string
      error: string | null
    }
  | {
      k: 'FileWriteBytes'
      root: string
      path: string
      requestId: string
      error: string | null
    }
  | {
      k: 'FileDownloadBytes'
      root: string
      path: string
      requestId: string
      bytesB64: string | null
      size: number
      tooLarge: boolean
      error: string | null
      /** Host already wrote via native save dialog; bytesB64 is empty. */
      saved?: boolean
    }
  | {
      k: 'FileContentSearch'
      root: string
      path: string
      requestId: string
      results: ContentSearchFileHit[]
      error: string | null
      truncated: boolean
    }
  | {
      k: 'FileContentReplace'
      root: string
      path: string
      requestId: string
      filesChanged: number
      matchCount: number
      error: string | null
      truncated: boolean
    }
  // Reply to GuiReq ImportGraphImpact — transitive impact paths.
  | {
      k: 'ImportGraphImpact'
      requestId: string
      sessionId?: string | null
      path: string
      depth: number
      paths: string[]
      total: number
      error: string | null
    }

  // Reply to GuiReq ImportGraph — the linker daemon's code-dependency graph.
  // `not-indexed` = all configured roots not yet scanned; `scanning` =
  // at least one root being scanned; `unavailable` = linker daemon unreachable;
  // `ok` = graph data present. `requestId`/`sessionId` let the store
  // drop stale replies.
  | {
      k: 'ImportGraph'
      status: 'ok' | 'scanning' | 'not-indexed' | 'unavailable'
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
      availableRoots: ImportGraphRootInfo[]
      requestId?: string | null
      sessionId?: string | null
    }

  // Reply to remote host CRUD requests — the authoritative list of saved
  // remote hosts. REPLACED wholesale on each push, never accumulated.
  | { k: 'RemoteHosts'; hosts: RemoteHost[] }

  // Remote connection state pushed to React. State transitions during the
  // SSH connect sequence: disconnected → resolving → auth_required |
  // bootstrapping → ready (host live, remote hub) → connecting → connected | error.
  | {
      k: 'RemoteState'
      state: string
      hostId?: string | null
      user?: string | null
      host?: string | null
      sessionId?: string | null
      error?: string | null
      sessions?: Array<{
        sessionId: string
        name: string
        pwd?: string
        working: boolean
        isForeground: boolean
      }>
    }
  | {
      k: 'RemotePathPicker'
      state: 'idle' | 'listing' | 'ready' | 'error' | 'cancelled'
      path?: string | null
      dirs?: string[]
      error?: string | null
    }
  | ({ k: 'TerminalShells' } & import('../../lib/terminalShells').TerminalShellReply)
  // ─── GUI terminal view ──────────────────────────────────────────────
  | {
      k: 'TerminalOutput'
      id: string
      data: string
    }
  | {
      k: 'TerminalExit'
      id: string
      code: number | null
    }

// GuiReq (JS -> Rust request payloads) is a global ambient type declared in
// koma.d.ts alongside the rest of the window bridge contract.

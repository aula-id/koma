import { broadcastThemeToPanels } from '../lib/panelBridge'
import { DEFAULT_GROUP } from './editorGroups'
import type { PaletteColors } from './types/chat'
import type { GitCommitNode, GitStatus, KeyInfo } from './types/git'
import type { BootstrapState } from './types/session'
import type { ActivityBarLayout, ActivitySlice, AnalyticsSlice, ConfigSlice, GraphSlice, HubSlice, ImportGraphSlice, OAuthSlice, SessionSlice, StoreSlice, TutorialSlice, UiSlice } from './types/slices'
import type { Tab } from './types/tabs'
import type { ModelListEntry, RouteEntry } from '../types/config'

export const ACTIVITY_BAR_STORAGE_KEY = 'koma.activitybar.layout'

export function loadActivityBarLayout(): ActivityBarLayout {
  try {
    const raw = localStorage.getItem(ACTIVITY_BAR_STORAGE_KEY)
    if (!raw) return { order: [], hidden: [] }
    const parsed = JSON.parse(raw)
    const order = Array.isArray(parsed?.order)
      ? parsed.order.filter((x: unknown): x is string => typeof x === 'string')
      : []
    const hidden = Array.isArray(parsed?.hidden)
      ? parsed.hidden.filter((x: unknown): x is string => typeof x === 'string')
      : []
    return { order, hidden }
  } catch {
    return { order: [], hidden: [] }
  }
}

export function saveActivityBarLayout(layout: ActivityBarLayout) {
  try {
    localStorage.setItem(ACTIVITY_BAR_STORAGE_KEY, JSON.stringify(layout))
  } catch {
    /* localStorage unavailable (e.g. privacy mode) — layout just won't persist */
  }
}

// Merge a persisted (possibly stale/partial) order against the CURRENT full id
// list, forward-compatibly: known ids keep their saved relative order; any id
// missing from `savedOrder` (a fresh install, or a newly-added item — e.g. a
// future extension-contributed icon) is appended at the end, default-visible.
// Shared by ActivityBar (bar + overflow menu) and SettingsTab (the Sidebar
// section's toggle list) so both always agree on the effective order.
export function resolveActivityBarOrder(savedOrder: string[], allIds: string[]): string[] {
  // Keep the user's saved relative order (drag-reorder / localStorage). Only
  // filter unknown ids and append anything new (fresh install → empty saved
  // order → allIds as-is; newly added built-in or ext:* → tail).
  // Do NOT re-sort to ACTIVITY_BAR_ITEMS — that nullifies intentional reorder
  // (regression from bcb33ca2).
  const known = savedOrder.filter((id) => allIds.includes(id))
  const missing = allIds.filter((id) => !known.includes(id))
  return [...known, ...missing]
}

export const initialSession: SessionSlice = {
  id: null,
  state: null,
  messages: [],
  messageCount: 0,
  hasMoreOlder: false,
  title: '',
  working: false,
  stream: '',
  reasoning: '',
  subagents: [],
  bash: [],
  fileChanges: [],
  planTodos: [],
  attachments: [],
  searchResults: [],
  mode: 'auto',
  pendingSteer: [],
  awaitingApproval: false,
  modelRoutes: [],
  approvalReason: null,
  pendingCall: null,
  sdlcPhase: null,
  sdlcGoal: null,
  sdlcBranch: null,
  sdlcOpen: null,
  sdlcSealed: null,
  tokensIn: 0,
  tokensCached: 0,
  tokensOut: 0,
  cost: 0,
}

export const initialHub: HubSlice = {
  state: null,
  cooking: [],
  history: [],
}

// The permanent chat tab (id 'chat'), always tabs[0] and never closeable. A
// factory (not a shared const) so every reset gets a fresh array/object.
export const makeChatTab = (): Tab => ({ id: 'chat', kind: 'chat' })

export const makeBootstrapState = (): BootstrapState => ({
  session: 'running',
  config: 'pending',
  settings: 'pending',
  repos: 'pending',
})

export function updateBootstrap(ui: UiSlice, patch: Partial<BootstrapState>): UiSlice {
  if (!ui.bootstrap) return ui
  const bootstrap = { ...ui.bootstrap, ...patch }
  const complete = Object.values(bootstrap).every(
    (phase) => phase === 'done' || phase === 'skipped' || phase === 'failed',
  )
  return {
    ...ui,
    bootstrap: complete ? null : bootstrap,
    ...(complete && !ui.loading?.active ? { switchingTo: null } : {}),
  }
}

export const initialUi: UiSlice = {
  omnisearchOpen: false,
  composerInsert: null,
  diagramChatQueue: [],
  pendingComposerAttachmentInserts: [],
  designChatQueue: [],
  pasteBody: null,
  composerRefill: null,
  pendingRewindIndex: null,
  scrollTick: 0,
  switchingTo: null,
  toast: null,
  toastSeq: 0,
  tabs: [makeChatTab()],
  activeTabId: 'chat',
  groups: [DEFAULT_GROUP],
  tabGroup: { chat: DEFAULT_GROUP },
  groupActive: { [DEFAULT_GROUP]: 'chat' },
  activeGroupId: DEFAULT_GROUP,
  splitDir: 'row',
  groupSizes: { [DEFAULT_GROUP]: 1 },
  splitTree: { type: 'leaf', id: DEFAULT_GROUP },
  groupSplitDir: {},
  focusPlanTick: 0,
  usageScope: 'all',
  loading: null,
  bootstrap: null,
  loadingDismissed: false,
}

// Bundled fallback theme (palette) registry — mirrors the host's theme.rs
// PALETTES names 1:1. Used as the onboarding picker's list when the host build
// doesn't advertise a `themes` array on the Config push yet.
export const KNOWN_THEMES = [
  'dark',
  'light',
  'forest',
  'autumn',
  'warm',
  'cold symphony',
  'winter',
  'monokai',
  'vscode',
  'github dark',
] as const

export const initialConfig: ConfigSlice = {
  mcp: [],
  providers: [],
  models: [],
  loaded: false,
  firstRun: undefined,
  theme: 'dark',
  themes: [...KNOWN_THEMES],
  palettes: [],
}

export const initialOAuth: OAuthSlice = {
  phase: 'idle',
  url: null,
  userCode: null,
  verificationUrl: null,
  error: null,
  conns: [],
  providers: [],
}

export const initialStore: StoreSlice = {
  catalogue: [],
  detail: null,
  installed: [],
  installedDetail: null,
  installedDetailRequestId: null,
  installedDetailLoading: false,
  installedDetailError: null,
  busy: false,
  error: null,
  pendingOp: null,
  pendingOpKind: null,
  opResult: null,
}

export const initialTutorial: TutorialSlice = {
  messages: [],
  busy: false,
  error: null,
  pendingId: null,
  pendingTour: null,
}

// Read once at module load (mirrors how `initialSession`/`initialUi` etc. seed
// the store) — persisted ActivityBar layout, or an empty pair on a fresh
// install / storage failure.
export const initialActivityBar: ActivityBarLayout = loadActivityBarLayout()

export const initialGit: GitStatus = {
  root: null,
  branch: null,
  detached: false,
  ahead: null,
  behind: null,
  staged: [],
  unstaged: [],
  error: null,
  keyName: null,
  inProgress: null,
  conflicted: [],
  pushMode: null,
}

export const initialGraph: GraphSlice = {
  commits: [],
  head: null,
  hasMore: false,
  loading: false,
  loadMode: 'replace',
  pendingRefresh: false,
  selectedSha: null,
  detail: null,
  graphMode: 'rail',
}

export const initialImportGraph: ImportGraphSlice = {
  status: 'idle',
  nodes: [],
  edges: [],
  focus: null,
  generation: 0,
  fileCount: 0,
  edgeCount: 0,
  languages: [],
  nodesTruncated: false,
  edgesTruncated: false,
  totalNodesAvailable: 0,
  totalEdgesAvailable: 0,
  loading: false,
  error: null,
  selectedPath: null,
  depth: 1,
  direction: 'both',
  breadcrumb: [],
  availableRoots: [],
  filterRoots: [],
  filterLanguages: [],
  queuedRefresh: false,
  treeNodes: [],
  impactRequestId: null,
  impactPath: null,
  impactDepth: 3,
  impactStatus: 'idle' as const,
  impactPaths: [],
  impactTotal: 0,
  impactError: null,
  reindexBusy: false,
  reindexError: null,
  activeRequestId: null,
  activeSessionId: null,
}

export const initialActivity: ActivitySlice = {
  commits: [],
  loading: false,
  error: null,
  path: null,
}

export const initialAnalytics: AnalyticsSlice = {
  scope: 'all',
  range: '7d',
  metric: 'cost',
  reqSeq: 0,
  sessionId: null,
  loading: false,
  error: null,
  data: null,
  hasData: false,
}

export const initialKeys: KeyInfo[] = []

export const initialModelList: ModelListEntry[] = []

export const initialRouteList: { provider: string; modelId: string; routes: RouteEntry[] } | null = null

export const initialPalette: PaletteColors = {
  bg: '#0b0e14',
  fg: '#c8d3f5',
  accent: '#39ff14',
  dim: '#adadad',
  panel: '#2b2f38',
  warn: '#ffb43c',
  success: '#00c853',
  info: '#50c8ff',
  error: '#ff3c3c',
  dark: true,
}

export const HEX_RE = /^#[0-9a-fA-F]{6}$/

// Live palette sync: repaint the --koma-* CSS vars whenever a Snapshot lands
// with a palette (home of the glue that used to live in Terminal.tsx's OSC 5380
// handler). Sets the full role set — bg/fg (chrome) plus accent/dim/panel — so
// styles.css can consume the REAL theme roles instead of color-mix guesses, and
// every non-default theme's chat colours track the daemon live. Each var is set
// only when its value is a valid hex, so a partial/legacy push never clobbers a
// role with garbage (the CSS fallback holds).
export function applyPaletteVars(palette: PaletteColors) {
  if (typeof document === 'undefined') return
  const root = document.documentElement.style
  const setVar = (name: string, val: string | undefined) => {
    if (val && HEX_RE.test(val)) root.setProperty(name, val)
  }
  setVar('--koma-bg', palette?.bg)
  setVar('--koma-fg', palette?.fg)
  setVar('--koma-accent', palette?.accent)
  setVar('--koma-dim', palette?.dim)
  setVar('--koma-panel', palette?.panel)
  setVar('--koma-warn', palette?.warn)
  setVar('--koma-success', palette?.success)
  setVar('--koma-info', palette?.info)
  setVar('--koma-error', palette?.error)
  // Extension panels (koma://extension iframes) are theme-aware: broadcast
  // the freshly-applied palette to every registered panel right alongside
  // the CSS-var repaint, so a live panel's colours track the daemon exactly
  // like the chat chrome does (see docs/EXTENSIONS.md "Theme").
  broadcastThemeToPanels(palette)
  // Do not import monaco-setup here — that parsed the 4MB editor on the first
  // Config/Snapshot. Coding/diff tabs call applyKomaTheme on mount; if monaco
  // is already loaded, refresh the live theme without forcing the chunk.
  const monacoReady = (globalThis as unknown as { __komaMonacoReady?: boolean })
    .__komaMonacoReady
  if (monacoReady) {
    void import('../lib/monaco-setup')
      .then((m) => m.refreshKomaThemes())
      .catch(() => {
        /* ignore */
      })
  }
}

// Basename of a path — a diff tab's title (TabBar disambiguates colliding
// basenames with a dim parent-dir suffix at render time).
export function tabBaseName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

// De-duplicate a commit list by sha, keeping the FIRST occurrence's order — the
// load-more append path can re-receive an overlapping commit (a page boundary,
// or a commit reachable from two refs under `--all`), and a stable dedupe keeps
// the layout deterministic + the virtualized row keys unique.
export function dedupCommits(commits: GitCommitNode[]): GitCommitNode[] {
  const seen = new Set<string>()
  const out: GitCommitNode[] = []
  for (const c of commits) {
    if (seen.has(c.sha)) continue
    seen.add(c.sha)
    out.push(c)
  }
  return out
}

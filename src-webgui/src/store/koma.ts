import { create } from 'zustand'
import { codingRequest } from '../lib/coding-service'
import { sendCodingLanguage } from '../lib/coding-language'
import { resolveFilePreviewBytes } from '../lib/filePreview'
import { analyticsActions } from './actions/analytics'
import { codingActions } from './actions/coding'
import { diagramActions } from './actions/diagram'
import { gitActions } from './actions/git'
import { importGraphActions } from './actions/importGraph'
import { marketplaceActions } from './actions/marketplace'
import { remoteActions } from './actions/remote'
import { sessionActions } from './actions/session'
import { tabActions } from './actions/tabs'
import { initialCoding } from './coding'
import { initialDiagram } from './diagram'
import {
  initialActivity,
  initialActivityBar,
  initialAnalytics,
  initialConfig,
  initialGit,
  initialGraph,
  initialHub,
  initialImportGraph,
  initialKeys,
  initialModelList,
  initialOAuth,
  initialPalette,
  initialRouteList,
  initialSession,
  initialStore,
  initialTutorial,
  initialUi,
  KNOWN_THEMES,
  resolveActivityBarOrder,
} from './initial'
import { pushAgents } from './push/agents'
import { pushAnalytics } from './push/analytics'
import { pushCoding } from './push/coding'
import { pushComputer } from './push/computer'
import { pushConfig } from './push/config'
import { pushGit } from './push/git'
import { pushImportGraph } from './push/importGraph'
import { pushMarketplace } from './push/marketplace'
import { pushSession } from './push/session'
import { codingHostViews } from './runtime'
import type { KomaState } from './state'
import type { PushEnvelope } from './types/envelope'

export type {
  AnalyticsData,
  AnalyticsMetric,
  AnalyticsModelRow,
  AnalyticsRange,
  AnalyticsScope,
  AnalyticsSeriesPoint,
  AnalyticsStatus,
} from './types/analytics'
export type {
  ActivityCommit,
  BranchInfo,
  CommitDetail,
  CommitFile,
  GitCommitNode,
  GitFileEntry,
  GitPushMode,
  GitRef,
  GitStatus,
  KeyInfo,
  KeyReveal,
  RepoEntry,
  StashEntry,
} from './types/git'
export type { OAuthConn, OAuthPhase, OAuthProviderEntry } from './types/oauth'
export type {
  ExtPanel,
  InstalledExt,
  InstalledExtDetail,
  StoreContributes,
  StoreDetail,
  StoreItem,
} from './types/marketplace'
export type {
  AgentEntry,
  AttachmentEntry,
  BashJobEntry,
  BootstrapState,
  CatalogueModelEntry,
  CatalogueProviderEntry,
  DiffPayload,
  DyingMark,
  FileChangeEntry,
  HubCookingEntry,
  HubHistoryEntry,
  LoadPhase,
  LspInstallProgress,
  LspRuntimeServer,
  LspServerStatus,
  ModelRoute,
  PendingCall,
  PlanTodoEntry,
  SearchResultEntry,
  SubAgentEntry,
  ToastEntry,
} from './types/session'
export { isDying, resolveModelLabel, visiblePlanTodos } from './types/session'
export type {
  ChatMessage,
  EffortOptions,
  PaletteColors,
  PaletteInfo,
  SettingsValues,
  ToolCallView,
  UsageDayEntry,
  UsageModelEntry,
  UsagePreview,
} from './types/chat'
export type { Tab } from './types/tabs'
export type { PushEnvelope } from './types/envelope'
export type {
  ActivityBarLayout,
  ImportGraphEdge,
  ImportGraphNode,
  ImportGraphRootInfo,
} from './types/slices'
export type { CodingFileState, CodingSlice, DirState, FileTreeEntry } from './coding'
export type { LspDiagnostic } from '../lib/lsp-bridge'
export { fileKey, initialCoding } from './coding'
export { KNOWN_THEMES, resolveActivityBarOrder }

export const useKoma = create<KomaState>((set, get) => ({
  computer: null,
  computerError: null,
  session: initialSession,
  hub: initialHub,
  palette: initialPalette,
  ui: initialUi,
  config: initialConfig,
  oauth: initialOAuth,
  store: initialStore,
  tutorial: initialTutorial,
  activityBar: initialActivityBar,
  modelList: initialModelList,
  routeList: initialRouteList,
  settingsValues: null,
  effortOptions: null,
  usagePreview: null,
  usagePreviewBusy: false,
  mcpStatusBusy: false,
  mcpStatusRequestId: null,
  dyingSessions: [],
  agents: [],
  catalogueModels: [],
  catalogueProviders: [],
  availableTools: [],
  git: initialGit,
  graph: initialGraph,
  activity: initialActivity,
  analytics: initialAnalytics,
  importGraph: initialImportGraph,
  coding: initialCoding,
  diagram: initialDiagram,
  remoteHosts: [],
  remoteState: { state: 'disconnected', hostId: null, user: null, host: null, sessionId: null, error: null, sessions: [] },
  remotePath: { state: 'idle', path: '', dirs: [], error: null },
  remoteBusy: null,
  commitDraft: '',
  keys: initialKeys,
  lspServers: [],
  lspProgress: {},
  lspDiagnostics: {},
  lspDiagCounts: { errors: 0, warnings: 0 },
  lspRuntime: [],
  bottomPanelTab: null,
  keyRevealResult: null,
  branches: [],
  branchesLoading: false,
  branchesRequestId: null,
  repos: [],
  activeRepoRoot: null,
  stashes: [],
  agentSaving: null,

  push: (env) => {
    if (pushComputer(set, get, env)) return
    if (pushCoding(set, get, env)) return
    if (pushGit(set, get, env)) return
    if (pushSession(set, get, env)) return
    if (pushConfig(set, get, env)) return
    if (pushAgents(set, get, env)) return
    if (pushMarketplace(set, get, env)) return
    if (pushAnalytics(set, get, env)) return
    if (pushImportGraph(set, get, env)) return
  },
  req: (g) => {
    if (['FileTree', 'FileRead', 'FileSave', 'FileCreate', 'FileRename', 'FileDelete', 'FileWriteBytes', 'FileDownloadBytes', 'FileContentSearch'].includes(g.r) && 'root' in g && typeof g.root === 'string') {
      const hostId = get().remoteState.hostId ?? 'local'
      const deliver = (value: Record<string, unknown>) => {
        const reply = { ...g, ...value, k: g.r, requestId: 'requestId' in g ? g.requestId : '' } as unknown as PushEnvelope
        if (reply.k === 'FileDownloadBytes' && resolveFilePreviewBytes(reply.requestId, reply.bytesB64, reply.error, reply.tooLarge)) return
        if ((get().remoteState.hostId ?? 'local') === hostId) get().push(reply)
        else codingHostViews.get(hostId)?.replies.push(reply)
      }
      void codingRequest<Record<string, unknown>>({ hostId, root: g.root }, { op: 'file', body: g as unknown as Record<string, unknown> })
        .then(deliver).catch(error => deliver({ error: String(error.message ?? error), entries: [], results: [], content: null, fingerprint: '', binary: false, tooLarge: false }))
      return
    }
    if (['LspDidOpen', 'LspDidChange', 'LspDidSave', 'LspDidClose', 'LspCompletion', 'LspCompletionResolve', 'LspHover', 'LspDefinition', 'LspReferences', 'LspDocumentSymbol'].includes(g.r) && 'root' in g && typeof g.root === 'string') {
      const workspace = { hostId: get().remoteState.hostId ?? 'local', root: g.root }
      void sendCodingLanguage(workspace, g as unknown as Record<string, unknown>).catch(error => {
        const requestId = 'requestId' in g ? g.requestId : null
        if (typeof requestId === 'string') {
          get().push({ k: g.r, requestId, error: String(error.message ?? error) } as PushEnvelope)
        }
      })
      return
    }
    // Coding panel ops have no other UI feedback path when the host bridge is
    // missing or throws — surface a toast rather than silently spinning. Do
    // NOT treat a successful postMessage as delivery success; only missing/
    // throwing ipc is reported here.
    const isCodingReq =
      g.r === 'FileTree' ||
      g.r === 'FileRead' ||
      g.r === 'FileSave' ||
      g.r === 'FileCreate' ||
      g.r === 'FileRename' ||
      g.r === 'FileDelete' ||
      g.r === 'FileWriteBytes' ||
      g.r === 'FileDownloadBytes' ||
      g.r === 'FileContentSearch' ||
      g.r === 'FileContentReplace'
    const ipc = window.ipc
    if (!ipc || typeof ipc.postMessage !== 'function') {
      if (g.r === 'FileSave') get().push({ k: 'FileSave', root: g.root, path: g.path, requestId: g.requestId, fingerprint: '', error: 'IPC unavailable — save was not sent' })
      if (isCodingReq) {
        const text = 'IPC unavailable — coding request was not sent'
        set((s) => {
          if (s.ui.toast?.text === text) return s
          const seq = s.ui.toastSeq + 1
          return {
            ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } },
          }
        })
      }
      return
    }
    try {
      ipc.postMessage(JSON.stringify({ t: 'req', ...g }))
    } catch (e) {
      if (g.r === 'FileSave') get().push({ k: 'FileSave', root: g.root, path: g.path, requestId: g.requestId, fingerprint: '', error: 'IPC error — save was not sent' })
      if (isCodingReq) {
        const msg = e instanceof Error ? e.message : String(e)
        const text = `IPC error — coding request failed: ${msg}`
        set((s) => {
          if (s.ui.toast?.text === text) return s
          const seq = s.ui.toastSeq + 1
          return {
            ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } },
          }
        })
      }
    }
  },

  ...sessionActions(set, get),
  ...remoteActions(set, get),
  ...tabActions(set, get),
  ...gitActions(set, get),
  ...analyticsActions(set, get),
  ...marketplaceActions(set, get),
  ...importGraphActions(set, get),
  ...codingActions(set, get),
  ...diagramActions(set, get),
}))

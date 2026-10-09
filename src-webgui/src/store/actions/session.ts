import { helpContext } from '../../lib/helpContext'
import { codingRefToken } from '../../lib/codingRef'
import type { StoreGet, StoreSet } from '../api'
import { useComputerPreview } from '../computerPreview'
import { DEFAULT_GROUP, normalizeGroups } from '../editorGroups'
import { clampChatTurns } from '../../lib/chatWindow'
import { initialSession, makeBootstrapState, makeChatTab, saveActivityBarLayout, updateBootstrap } from '../initial'
import type { KomaState } from '../state'
import type { LoadPhase } from '../types/session'
import type { ActivityBarLayout, TutorialMsg } from '../types/slices'
import type { Tab } from '../types/tabs'

let agentTabSeq = 0
function mintAgentTabId(): string {
  agentTabSeq += 1
  return `agent-${agentTabSeq}`
}

export function sessionActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openExternal' | 'openOmniSearch' | 'closeOmniSearch' | 'insertToComposer' | 'putCodingPathInChat' | 'askCodingSelectionInChat' | 'addDiagramToChat' | 'consumeDiagramChatQueue' | 'stageComposerAttachmentInsert' | 'consumePendingComposerAttachmentInserts' | 'addDesignToChat' | 'consumeDesignChatQueue' | 'consumePasteBody' | 'consumeComposerInsert' | 'refillComposer' | 'consumeComposerRefill' | 'stageRewind' | 'clearRewind' | 'requestHistoryPage' | 'requestScrollBottom' | 'startSwitching' | 'cancelSwitching' | 'skipBootstrapRemaining' | 'dismissLoading' | 'dismissToast' | 'openSettingsTab' | 'openNotificationsTab' | 'openHelpTab' | 'openTutorialTab' | 'sendTutorialChat' | 'clearTutorialPendingTour' | 'clearTutorialError' | 'setActivityBarOrder' | 'setActivityBarHidden' | 'openAgentTab' | 'renameAgentTab' | 'openStreamTab' | 'syncStreamView' | 'focusPlanSection' | 'setUsageScope' | 'setChatTurns' | 'refreshUsagePreview' | 'refreshMcpStatus' | 'markDying' | 'detachSession' | 'setAgentSaving' | 'clearAgentSaving'> {
  return {
  openExternal: (url) => {
    get().req({ r: 'OpenExternal', url })
  },

  openOmniSearch: () => set((s) => ({ ui: { ...s.ui, omnisearchOpen: true } })),
  closeOmniSearch: () => set((s) => ({ ui: { ...s.ui, omnisearchOpen: false } })),
  insertToComposer: (path) => set((s) => ({ ui: { ...s.ui, composerInsert: path } })),
  putCodingPathInChat: (root, path, opts) => {
    const workdirs = (get().settingsValues?.workdir ?? []).filter(Boolean)
    const token = codingRefToken(root, path, workdirs, { isDir: !!opts?.isDir })
    get().insertToComposer(token)
    get().activateTab('chat')
    queueMicrotask(() => {
      const el = document.querySelector(
        '[data-tour="composer"] textarea',
      ) as HTMLTextAreaElement | null
      el?.focus()
    })
  },
  askCodingSelectionInChat: (payload) => {
    const text = payload?.text?.replace(/\s+$/, '') ?? ''
    if (!text) return
    get().stageComposerAttachmentInsert('pasted_text', {
      name: payload.label,
      text,
      path: payload.path,
    })
    get().req({ r: 'AttachPaste', text })
    get().activateTab('chat')
    queueMicrotask(() => {
      const el = document.querySelector(
        '[data-tour="composer"] textarea, [data-tour="composer"] [contenteditable="true"]',
      ) as HTMLElement | null
      el?.focus()
    })
  },
  addDiagramToChat: (item) => {
    set((s) => ({ ui: { ...s.ui, diagramChatQueue: [...s.ui.diagramChatQueue, item] } }))
    get().activateTab('chat')
  },
  consumeDiagramChatQueue: () => set((s) => ({ ui: { ...s.ui, diagramChatQueue: [] } })),
  stageComposerAttachmentInsert: (kind, extra) => {
    const id = `a${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
    set((s) => ({
      ui: {
        ...s.ui,
        pendingComposerAttachmentInserts: [
          ...s.ui.pendingComposerAttachmentInserts,
          { id, kind, name: extra?.name, text: extra?.text, path: extra?.path },
        ],
      },
    }))
    return id
  },
  consumePendingComposerAttachmentInserts: () =>
    set((s) => ({ ui: { ...s.ui, pendingComposerAttachmentInserts: [] } })),
  addDesignToChat: (item) => {
    set((s) => ({ ui: { ...s.ui, designChatQueue: [...s.ui.designChatQueue, item] } }))
    get().activateTab('chat')
  },
  consumeDesignChatQueue: () => set((s) => ({ ui: { ...s.ui, designChatQueue: [] } })),
  consumePasteBody: () => set((s) => ({ ui: { ...s.ui, pasteBody: null } })),

  consumeComposerInsert: () => set((s) => ({ ui: { ...s.ui, composerInsert: null } })),
  refillComposer: (text) => set((s) => ({ ui: { ...s.ui, composerRefill: text } })),
  consumeComposerRefill: () => set((s) => ({ ui: { ...s.ui, composerRefill: null } })),
  stageRewind: (index) => set((s) => ({ ui: { ...s.ui, pendingRewindIndex: index } })),
  clearRewind: () => set((s) => ({ ui: { ...s.ui, pendingRewindIndex: null } })),
  requestHistoryPage: () => {
    const s = get().session
    if (!s.id || !s.hasMoreOlder) return
    const oldest = s.messages[0]
    const before =
      oldest && typeof oldest.idx === 'number' ? oldest.idx : undefined
    get().req({ r: 'HistoryPage', before })
  },
  requestScrollBottom: () => set((s) => ({ ui: { ...s.ui, scrollTick: s.ui.scrollTick + 1 } })),
  startSwitching: (name) =>
    set((s) => ({
      ui: {
        ...s.ui,
        switchingTo: name,
        loadingDismissed: false,
        bootstrap: makeBootstrapState(),
      },
    })),
  cancelSwitching: () =>
    set((s) => ({
      ui: { ...s.ui, switchingTo: null, loading: null, bootstrap: null },
    })),
  skipBootstrapRemaining: () =>
    set((s) => {
      if (!s.ui.bootstrap) return s
      const b = s.ui.bootstrap
      const term = (p: LoadPhase): LoadPhase =>
        p === 'done' || p === 'skipped' || p === 'failed' ? p : 'skipped'
      return {
        ui: updateBootstrap(s.ui, {
          session: term(b.session),
          config: term(b.config),
          settings: term(b.settings),
          repos: term(b.repos),
        }),
      }
    }),
  dismissLoading: () => set((s) => ({ ui: { ...s.ui, loadingDismissed: true } })),
  dismissToast: (id) =>
    set((s) => (s.ui.toast?.id === id ? { ui: { ...s.ui, toast: null } } : s)),
  openSettingsTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'settings')
      const tabs: Tab[] = exists
        ? s.ui.tabs
        : [...s.ui.tabs, { id: 'settings', kind: 'settings' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'settings' } }
    })
    get().req({ r: 'GetSettings' })
  },
  openNotificationsTab: () => {
    set(s => ({ ui: { ...s.ui, tabs: s.ui.tabs.some(t => t.id === 'notifications') ? s.ui.tabs : [...s.ui.tabs, { id: 'notifications', kind: 'notifications' }], activeTabId: 'notifications' } }))
  },
  openHelpTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'help')
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, { id: 'help', kind: 'help' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'help' } }
    })
  },
  openTutorialTab: () => { get().openHelpTab() },
  sendTutorialChat: (text) => {
    const content = text.trim()
    if (!content) return
    const id =
      typeof crypto !== 'undefined' && 'randomUUID' in crypto
        ? crypto.randomUUID()
        : `t-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
    const userMsg: TutorialMsg = { id: `${id}-u`, role: 'user', content }
    set((s) => ({
      tutorial: {
        ...s.tutorial,
        messages: [...s.tutorial.messages, userMsg],
        busy: true,
        error: null,
        pendingId: id,
        pendingTour: null,
      },
    }))
    // Rolling transcript for the host (user/assistant only).
    const wire = get().tutorial.messages
      .filter((m) => m.role === 'user' || m.role === 'assistant')
      .map((m) => ({ role: m.role, content: m.content }))
    get().req({ r: 'TutorialChat', id, messages: wire, context: helpContext() })
  },
  clearTutorialPendingTour: () => {
    set((s) => ({ tutorial: { ...s.tutorial, pendingTour: null } }))
  },
  clearTutorialError: () => {
    set((s) => ({ tutorial: { ...s.tutorial, error: null } }))
  },
  setActivityBarOrder: (order) => {
    set((s) => {
      const next: ActivityBarLayout = { order, hidden: s.activityBar.hidden }
      saveActivityBarLayout(next)
      return { activityBar: next }
    })
  },
  setActivityBarHidden: (view, hidden) => {
    set((s) => {
      const hiddenSet = new Set(s.activityBar.hidden)
      if (hidden) hiddenSet.add(view)
      else hiddenSet.delete(view)
      const next: ActivityBarLayout = { order: s.activityBar.order, hidden: [...hiddenSet] }
      saveActivityBarLayout(next)
      return { activityBar: next }
    })
  },
  openAgentTab: (agentId) => {
    set((s) => {
      // Dedupe by agentId (NOT id — see the Tab union comment): re-clicking
      // the same agent's row, or "+ Add agent" while a blank create tab is
      // already open, focuses that existing tab instead of opening another.
      const existingTab = s.ui.tabs.find((t) => t.kind === 'agent' && t.agentId === agentId)
      if (existingTab) return { ui: { ...s.ui, activeTabId: existingTab.id } }
      const id = mintAgentTabId()
      const tabs: Tab[] = [...s.ui.tabs, { id, kind: 'agent', agentId }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
  },
  renameAgentTab: (oldAgentId, newAgentId) => {
    if (oldAgentId === newAgentId) return
    // Only `agentId` changes — `id` (and thus the tab's React key/identity
    // and activeTabId) stays exactly as it was, so the open AgentTab instance
    // is never remounted by this rebind.
    set((s) => ({
      ui: {
        ...s.ui,
        tabs: s.ui.tabs.map((t) =>
          t.kind === 'agent' && t.agentId === oldAgentId ? { ...t, agentId: newAgentId } : t,
        ),
      },
    }))
  },
  openStreamTab: (kind, targetId, title) => {
    const id = kind === 'subagent' ? `sa:${targetId}` : `bash:${targetId}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const tab: Tab =
        kind === 'subagent'
          ? { id, kind: 'subagent', agentId: targetId, title }
          : { id, kind: 'bash', jobId: targetId, title }
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, tab]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    // This stream tab is now active → tell the host to stream its target's content.
    get().syncStreamView()
  },
  syncStreamView: () => {
    const { tabs, activeTabId } = get().ui
    const tab = tabs.find((t) => t.id === activeTabId)
    const subagent = tab && tab.kind === 'subagent' ? tab.agentId : null
    const bash = tab && tab.kind === 'bash' ? tab.jobId : null
    // Pin the ids to the current session — they're per-session counters daemon-side, so
    // the daemon needs the session to disambiguate (agent 0 / bash 1 exist in every session).
    get().req({ r: 'SetStreamView', subagent, bash, session: get().session.id })
  },
  focusPlanSection: () => set((s) => ({ ui: { ...s.ui, focusPlanTick: s.ui.focusPlanTick + 1 } })),
  setUsageScope: (scope) => set((s) => ({ ui: { ...s.ui, usageScope: scope } })),
  setChatTurns: (turns) => set((s) => ({ ui: { ...s.ui, chatTurns: clampChatTurns(turns) } })),
  refreshUsagePreview: () => {
    const s = get()
    // Clear any stale preview first so the loading row shows instead of
    // rendering the OTHER scope's (or a since-switched session's) numbers
    // while the fresh reply is in flight — mirrors UsagePanel's own effect.
    set({ usagePreview: null, usagePreviewBusy: true })
    s.req({
      r: 'UsagePreview',
      scope: s.ui.usageScope,
      sessionId: s.ui.usageScope === 'session' ? (s.session.id ?? undefined) : undefined,
    })
  },
  refreshMcpStatus: () => {
    const id = crypto.randomUUID()
    set({ mcpStatusBusy: true, mcpStatusRequestId: id })
    get().req({ r: 'GetMcpStatus', requestId: id })
    // Safety net: if no reply arrives (e.g. no session attached), clear the
    // busy flag after 5 s so the spinner doesn't spin forever.
    setTimeout(() => {
      if (get().mcpStatusRequestId === id) {
        set({ mcpStatusBusy: false, mcpStatusRequestId: null })
      }
    }, 5_000)
  },
  markDying: (id, kind) =>
    set((s) =>
      s.dyingSessions.some((d) => d.id === id && d.kind === kind)
        ? s
        : { dyingSessions: [...s.dyingSessions, { id, kind }] },
    ),
  detachSession: () => {
    useComputerPreview.getState().hide()
    // A guided first-chat retains its captured editor tabs through detach.
    if (!get().ui.preserveTabsOnNextSession) get().closeAllTabsExceptChat({ force: true })
    set((s) => ({
      computer: null,
      computerError: null,
      skillDeletePending: {},
      // Fresh object (not spread from the old session) — nothing about the
      // just-killed session is worth preserving, mirrors initialSession's
      // shape exactly.
      session: { ...initialSession },
      // Terminals may survive detach, but editor-group split state must not:
      // leaving groups/groupSizes from a prior 2-pane layout painted empty
      // tracks on macOS/Windows WebViews after the session died.
      ui: normalizeGroups({
        ...s.ui,
        tabs: [makeChatTab()],
        activeTabId: 'chat',
        groups: [DEFAULT_GROUP],
        tabGroup: {},
        groupActive: { [DEFAULT_GROUP]: 'chat' },
        activeGroupId: DEFAULT_GROUP,
        splitDir: 'row' as const,
        groupSizes: { [DEFAULT_GROUP]: 1 },
        splitTree: { type: 'leaf' as const, id: DEFAULT_GROUP },
        groupSplitDir: {},
        switchingTo: null,
        preserveTabsOnNextSession: s.ui.preserveTabsOnNextSession,
        preservedTabLayout: s.ui.preservedTabLayout,
        preservedTabsTargetSession: s.ui.preservedTabsTargetSession,
        // Defensive: also drop any stale startup splash — it described the
        // now-dead session's warm-up and must not linger over StartScreen.
        loading: null,
        bootstrap: null,
        loadingDismissed: false,
      }),
      // Drop session-scoped Analytics result on detach (all-scope data can stay;
      // filters are a user preference and are preserved).
      analytics: {
        ...s.analytics,
        ...(s.analytics.scope === 'session'
          ? {
              scope: 'all' as const,
              data: null,
              hasData: false,
              loading: false,
              error: null,
              sessionId: null,
            }
          : { sessionId: null }),
      },
    }))
    // Tabs just reset to chat-only → no stream tab is active; tell the host
    // to stop streaming whatever the dead session's stream tab was targeting
    // (mirrors the Snapshot handler's `switched` branch).
    get().syncStreamView()
  },
  // Track a pending agent-save request so the AgentTab can show a spinner,
  // disable the Save button, and receive success/error feedback without
  // premature rename/rebind. `seq` monotonically increases per save so the
  // confirming push (AgentsValues or AgentOp) can reject a stale reply by
  // comparing against `get().agentSaving.seq`.
  setAgentSaving: (tabId, originalName, newName) => {
    const current = get().agentSaving
    const seq = (current?.seq ?? 0) + 1
    set(() => ({ agentSaving: { tabId, seq, originalName, newName } }))
    return seq
  },
  clearAgentSaving: (seq) => {
    set((s) => {
      if (seq !== undefined && s.agentSaving?.seq !== seq) return {} // stale, don't clear
      return { agentSaving: null }
    })
  },
  }
}

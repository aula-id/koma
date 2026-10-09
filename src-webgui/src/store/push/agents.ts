import { parseHelpAnswer } from '../../lib/helpKnowledge'
import type { StoreGet, StoreSet } from '../api'
import { initialCoding } from '../coding'
import { initialDesign } from '../design'
import { initialDiagram } from '../diagram'
import { DEFAULT_GROUP, normalizeGroups } from '../editorGroups'
import { makeChatTab } from '../initial'
import { codingHostViews } from '../runtime'
import type { PushEnvelope } from '../types/envelope'
import type { OAuthPhase } from '../types/oauth'
import type { AgentEntry, CatalogueModelEntry, CatalogueProviderEntry } from '../types/session'

export function pushAgents(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'AgentsValues': {
        // Normalize the wire's snake_case nested structs into the store's
        // camelCase shapes (see the AgentEntry/CatalogueModelEntry/
        // CatalogueProviderEntry comments — the envelope's OWN fields are
        // already camelCase, only the per-item fields need mapping).
        const agents: AgentEntry[] = env.agents.map((a) => ({
          name: a.name,
          description: a.description,
          conditions: a.conditions,
          source:
            a.source === 'global' || a.source === 'builtin' || a.source === 'extension'
              ? a.source
              : 'session',
          modelUuid: a.model_uuid,
          model: a.model,
          tools: a.tools,
          prompt: a.prompt,
          extId: a.source === 'extension' ? (a.ext_id ?? null) : null,
        }))
        const catalogueModels: CatalogueModelEntry[] = env.catalogueModels.map((m) => ({
          uuid: m.uuid,
          name: m.name,
          modelId: m.model_id,
          providerUuid: m.provider_uuid,
        }))
        const catalogueProviders: CatalogueProviderEntry[] = env.catalogueProviders.map((p) => ({
          uuid: p.uuid,
          name: p.name,
          endpoint: p.endpoint,
        }))
        const availableTools = env.availableTools ?? []
        set((s) => {
          const liveNames = new Set(agents.map((a) => a.name))
          // A deleted agent's editor tab has nothing left to show — close it
          // automatically, derived from its absence in this fresh list (no
          // explicit "delete succeeded" ack exists on the wire). An
          // in-progress CREATE tab (agentId === null) is never touched here —
          // it isn't "in" the list yet by definition.
          let tabs = s.ui.tabs
          let activeTabId = s.ui.activeTabId
          const staleIds = new Set(
            tabs
              .filter((t) => t.kind === 'agent' && t.agentId !== null && !liveNames.has(t.agentId))
              .map((t) => t.id),
          )
          if (staleIds.size > 0) {
            tabs = tabs.filter((t) => !staleIds.has(t.id))
            // Multiple tabs could go stale from one push — land on chat
            // rather than compute per-removal left-neighbours (closeTab's
            // approach only makes sense for a single removal).
            if (staleIds.has(activeTabId)) activeTabId = 'chat'
          }
          return { agents, catalogueModels, catalogueProviders, availableTools, ui: { ...s.ui, tabs, activeTabId } }
        })
        // After updating agents, check if a pending agent save was confirmed
        // by the new list. This avoids premature rename/rebind: the tab's
        // `agentId` stays unchanged until the authoritative AgentsValues push
        // confirms the save landed. The seq check prevents a stale reply from
        // clearing a newer save request.
        const saving = get().agentSaving
        // reqSeq MUST match the current agentSaving.seq or this is a stale
        // reply. reqSeq === 0 (uncorrelated fallback from a read-only fetch
        // or host-built reply) never clears a pending save — the proper
        // SetAgent/DeleteAgent path always carries the real seq.
        if (saving && env.reqSeq === saving.seq) {
          const saved = get().agents.find((a) => a.name === saving.newName)
          if (saved) {
            get().clearAgentSaving(saving.seq)
            // Rename the agent tab so re-clicking its row in the sidebar
            // focuses this same tab instead of opening a duplicate.
            get().renameAgentTab(saving.originalName, saving.newName)
            // Toast a SUCCESS confirmation — the daemon's save succeeded.
            const text = saving.originalName && saving.originalName !== saving.newName
              ? `renamed to ${saving.newName}`
              : 'agent saved'
            const toastSeq = get().ui.toastSeq + 1
            set((s) => ({
              ui: { ...s.ui, toastSeq, toast: { id: toastSeq, text, kind: 'success' } },
            }))
          }
        }
        break
      }
      case 'OAuthState': {
        const KNOWN_PHASES: OAuthPhase[] = [
          'idle',
          'starting',
          'waiting_url',
          'waiting_code',
          'paste',
          'success',
          'failed',
        ]
        const phase: OAuthPhase = KNOWN_PHASES.includes(env.phase as OAuthPhase)
          ? (env.phase as OAuthPhase)
          : 'idle'
        set(() => ({
          oauth: {
            phase,
            url: env.url,
            userCode: env.userCode,
            verificationUrl: env.verificationUrl,
            error: env.error,
            // Normalize the wire's snake_case `account_id` to camelCase,
            // matching the AgentsValues normalization pattern.
            conns: env.conns.map((c) => ({
              uuid: c.uuid,
              name: c.name,
              provider: c.provider,
              email: c.email,
              plan: c.plan,
              accountId: c.account_id,
            })),
            providers: env.providers.map((p) => ({ id: p.id, label: p.label, kind: p.kind })),
          },
        }))
        break
      }
      case 'TutorialChatDone': {
        // Stale-drop if a newer turn is in flight.
        const pending = get().tutorial.pendingId
        if (!pending || env.id !== pending) break
        set((s) => {
          const msgs = [...s.tutorial.messages]
          if (env.error) {
            return {
              tutorial: {
                ...s.tutorial,
                busy: false,
                pendingId: null,
                error: env.error,
              },
            }
          }
          let answer: ReturnType<typeof parseHelpAnswer>
          try { answer = parseHelpAnswer(env.text) } catch (error) {
            return { tutorial: { ...s.tutorial, busy: false, pendingId: null, error: String(error) } }
          }
          msgs.push({
            id: env.id,
            role: 'assistant',
            content: answer.answer,
            articles: answer.articles,
            navigation: answer.navigation,
            tour: answer.guide,
          })
          return {
            tutorial: {
              ...s.tutorial,
              messages: msgs,
              busy: false,
              pendingId: null,
              pendingTour: answer.guide ?? null,
              error: null,
            },
          }
        })
        break
      }
      case 'AgentOp': {
        // Daemon SetAgent/DeleteAgent result — surface the error as a toast
        // and clear the pending saving state. Success is authoritative via
        // AgentsValues, so only failures use this envelope.
        // reqSeq MUST match the current agentSaving.seq or this is a stale
        // reply (a prior request that landed after a newer one was issued).
        // reqSeq === 0 (uncorrelated fallback) never clears a pending save
        // — the new DaemonEvent::AgentOp from requests_agents.rs always
        // carries the proper seq, so only that path can clear.
        const saving = get().agentSaving
        if (!saving || env.reqSeq !== saving.seq) break
        if (!env.ok && env.error) {
          const text = `agent: ${env.error}`
          set((s) => {
            const raise = !!text
            const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
            return raise
              ? { ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } }, agentSaving: null }
              : { agentSaving: null }
          })
        } else {
          set(() => ({ agentSaving: null }))
        }
        break
      }
      case 'RemoteHosts':
        set(() => ({ remoteHosts: env.hosts }))
        break
      case 'RemoteState':
        if ((get().remoteState.hostId ?? 'local') !== (env.hostId ?? 'local')) {
          const state = get()
          codingHostViews.set(state.remoteState.hostId ?? 'local', { coding: state.coding, ui: state.ui, diagram: state.diagram, design: state.design, replies: [] })
        }
        // Leaving a remote attach (disconnect / back to hub / connect bounce) must
        // clear session.id so routes flip StartScreen — same job as detachSession
        // after kill. Host no longer owns this GUI session once RemoteState says
        // ready (hub, no session) or disconnected after a live remote.
        {
          const prev = get().remoteState
          const hadLiveRemote =
            prev.state === 'ready' ||
            prev.state === 'connected' ||
            prev.state === 'connecting' ||
            prev.state === 'auth_required' ||
            prev.state === 'bootstrapping' ||
            prev.state === 'resolving'
          const leaveToHub = env.state === 'ready' && get().session.id != null
          const leaveToLocal =
            env.state === 'disconnected' && hadLiveRemote && get().session.id != null
          if (leaveToHub || leaveToLocal) {
            get().detachSession()
          }
        }
        set((s) => {
          const hostChanged = (s.remoteState.hostId ?? 'local') !== (env.hostId ?? 'local')
          const languageReset = hostChanged ? { lspDiagnostics: {}, lspDiagCounts: { errors: 0, warnings: 0 }, lspRuntime: [], lspServers: [], lspProgress: {} } : {}
          const hostView = codingHostViews.get(env.hostId ?? 'local')
          const restoredUi = hostChanged && !s.ui.preserveTabsOnNextSession ? normalizeGroups({
            ...s.ui,
            tabs: [makeChatTab(), ...(hostView?.ui.tabs.filter(t => t.kind === 'codingFile' || t.kind === 'diagram' || t.kind === 'design') ?? [])],
            activeTabId: hostView?.ui.tabs.some(t => t.id === hostView.ui.activeTabId && (t.kind === 'codingFile' || t.kind === 'diagram' || t.kind === 'design')) ? hostView.ui.activeTabId : 'chat',
            groups: hostView?.ui.groups ?? [DEFAULT_GROUP], tabGroup: hostView?.ui.tabGroup ?? {},
            groupActive: hostView?.ui.groupActive ?? { [DEFAULT_GROUP]: 'chat' },
            activeGroupId: hostView?.ui.activeGroupId ?? DEFAULT_GROUP,
            splitDir: hostView?.ui.splitDir ?? 'row', groupSizes: hostView?.ui.groupSizes ?? { [DEFAULT_GROUP]: 1 },
            splitTree: hostView?.ui.splitTree ?? { type: 'leaf' as const, id: DEFAULT_GROUP },
            groupSplitDir: hostView?.ui.groupSplitDir ?? {},
          }) : s.ui
          const codingReset = hostChanged ? { coding: { ...(hostView?.coding ?? initialCoding), _sessionGen: s.coding._sessionGen + 1 } } : {}
          const diagramReset = hostChanged ? { diagram: hostView?.diagram ?? initialDiagram } : {}
          const designReset = hostChanged ? { design: hostView?.design ?? initialDesign } : {}
          const remoteState = {
            state: env.state,
            hostId: env.hostId ?? null,
            user: env.user ?? null,
            host: env.host ?? null,
            sessionId: env.sessionId ?? null,
            error: env.error ?? null,
            sessions: env.sessions ?? [],
          }
          if (env.state === 'connected' || env.state === 'disconnected' || env.state === 'ready') {
            return { ...languageReset, ...codingReset, ...diagramReset, ...designReset, remoteState, ui: { ...restoredUi, switchingTo: null } }
          }
          if (env.state === 'error') {
            const text = env.error ? `SSH: ${env.error}` : 'SSH connection failed'
            const seq = s.ui.toastSeq + 1
            return {
              ...languageReset,
              ...codingReset,
              ...diagramReset, ...designReset,
              remoteState,
              ui: {
                ...restoredUi,
                switchingTo: null,
                toastSeq: seq,
                toast: { id: seq, text, kind: 'error' as const },
              },
            }
          }
          return { ...languageReset, ...codingReset, ...diagramReset, ...designReset, remoteState, ui: restoredUi }
        })
        {
          const host = get().remoteState.hostId ?? 'local'
          const view = codingHostViews.get(host)
          const replies = view?.replies.splice(0) ?? []
          for (const reply of replies) get().push(reply)
        }
        break
      case 'RemotePathPicker':
        // Keep the previous dir list while state=listing (host sends dirs:[] on
        // navigate). Clearing would collapse the picker height and blink.
        set((s) => ({
          remotePath: {
            state: env.state,
            path: env.path ?? s.remotePath.path,
            dirs:
              env.dirs != null && env.dirs.length > 0
                ? env.dirs
                : env.state === 'listing'
                  ? s.remotePath.dirs
                  : (env.dirs ?? []),
            error: env.error ?? null,
          },
        }))
        break
    default:
      return false
  }
  return true
}

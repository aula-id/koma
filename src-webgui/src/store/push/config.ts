import type { StoreGet, StoreSet } from '../api'
import { applyPaletteVars, updateBootstrap } from '../initial'
import type { PushEnvelope } from '../types/envelope'

export function pushConfig(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'WebSearchValues':
        set({ webSearchValues: { req_seq: env.req_seq, status: env.status, error: env.error } })
        break
      case 'Config':
        // Empty/swapper theme: Config is pushed in BOTH the empty/swapper
        // state and the attached state, so it's the reliable carrier for the
        // active palette — adopt it (store + CSS vars) whenever present, same
        // consumption path as the Snapshot palette. No-op when omitted.
        if (env.palette) applyPaletteVars(env.palette)
        set((s) => ({
          config: {
            mcp: env.mcp,
            providers: env.providers,
            models: env.models,
            loaded: true,
            // Preserve the derived-default when the host omits these (keeps the
            // theme picker populated + the gate on the config-inference path).
            firstRun: env.firstRun,
            theme: env.theme ?? s.config.theme,
            themes: env.themes && env.themes.length > 0 ? env.themes : s.config.themes,
            // Adopt the resolved palette catalogue when present; keep the current
            // one otherwise (host build not projecting it yet).
            palettes: env.palettes && env.palettes.length > 0 ? env.palettes : s.config.palettes,
          },
          ...(env.palette ? { palette: env.palette } : {}),
          ...(s.ui.bootstrap ? { ui: updateBootstrap(s.ui, { config: 'done' }) } : {}),
        }))
        break
      case 'McpStatus': {
        // Drop stale replies: only apply if the requestId matches the current in-flight.
        const current = get().mcpStatusRequestId
        if (current !== env.requestId) break
        set((s) => {
          // Merge runtime status into config.mcp by server id.
          const serverMap = new Map(env.servers.map((sv) => [sv.id, sv]))
          const mcp = s.config.mcp.map((srv) => {
            const rt = serverMap.get(srv.id)
            if (!rt) return srv
            return {
              ...srv,
              toolCount: rt.toolCount,
              connected: rt.connected,
              error: rt.error,
            }
          })
          return {
            config: { ...s.config, mcp },
            mcpStatusBusy: false,
          }
        })
        break
      }
      case 'ModelList':
        set(() => ({ modelList: env.models }))
        break
      case 'RouteList':
        set(() => ({
          routeList: { provider: env.provider, modelId: env.modelId, routes: env.routes },
        }))
        break
      case 'SettingsValues': {
        const prevWorkdir = get().settingsValues?.workdir
        set((s) => ({
          settingsValues: {
            name: env.name,
            workdir: env.workdir,
            shortSend: env.shortSend,
            slidingCache: env.slidingCache,
            bashSaving: env.bashSaving,
            codingAutosave: !!env.codingAutosave,
            internetMode: env.internetMode,
            palette: env.palette,
            effort: env.effort ?? '',
            subagentMaxTurns: env.subagentMaxTurns ?? 500,
            shortSendEngageN: env.shortSendEngageN ?? 80,
            shortSendTailN: env.shortSendTailN ?? 40,
            maxOutputTokens: env.maxOutputTokens ?? 0,
            contextWindowLimit: env.contextWindowLimit ?? 0,
            contextModelAlias: env.contextModelAlias ?? '',
          },
          ...(s.ui.bootstrap ? { ui: updateBootstrap(s.ui, { settings: 'done' }) } : {}),
        }))
        // Prune importGraph state when the workdir list changes (same session).
        // Use canonical availableRoots (backend-provided) as ground truth —
        // raw settings strings may differ from canonical roots (symlinks,
        // relative paths).  Also union with new settings entries so not-yet-
        // indexed roots aren't immediately pruned.
        if (prevWorkdir && JSON.stringify(prevWorkdir) !== JSON.stringify(env.workdir)) {
          set((s) => {
            const ig = s.importGraph
            // Best-effort prune against new workdir (availableRoots is stale).
            // refreshImportGraph below brings authoritative backend roots.
            const validRoots = new Set(env.workdir.filter((r: string) => r.length > 0))
            const prunedFilters = ig.filterRoots.filter((r) => validRoots.has(r))
            // Prune treeNodes to only those under valid roots
            const prunedTreeNodes = ig.treeNodes.filter((n) =>
              !n.workspaceRoot || validRoots.has(n.workspaceRoot)
            )
            // Prune languages to those still present in availableRoots.
            const scopedRoots = ig.availableRoots.filter((r) => validRoots.has(r.root))
            let prunedLangs = ig.filterLanguages
            if (scopedRoots.length > 0) {
              const validLangs = new Set<string>()
              for (const r of scopedRoots) {
                for (const lc of r.languages) validLangs.add(lc.name)
              }
              prunedLangs = ig.filterLanguages.filter((l) => validLangs.has(l))
            }
            // Prune focus: if the focused node's root is no longer valid, clear it
            let focus = ig.focus
            let selectedPath = ig.selectedPath
            let breadcrumb = ig.breadcrumb
            if (focus) {
              const focusNode = ig.nodes.find((n) => n.path === focus)
              if (focusNode?.workspaceRoot && !validRoots.has(focusNode.workspaceRoot)) {
                focus = null
                selectedPath = null
                breadcrumb = []
              }
            }
            return {
              importGraph: {
                ...ig,
                filterRoots: prunedFilters,
                filterLanguages: prunedLangs,
                treeNodes: prunedTreeNodes,
                focus,
                selectedPath,
                breadcrumb,
              },
            }
          })
          // Trigger a fresh graph fetch so the backend re-scopes with the
          // new settings and the authoritative availableRoots arrive.
          get().refreshImportGraph()
        }
        break
      }
      case 'EffortOptions':
        set(() => ({
          effortOptions: {
            options: env.options,
            selected: env.selected,
            note: env.note,
            state: env.state,
          },
        }))
        break
      case 'Loading':
        set((s) => ({
          ui: {
            ...s.ui,
            loading: env.active
              ? { active: env.active, workspace: env.workspace, awareness: env.awareness }
              : null,
            // Warm-up finished — drop switch chrome even if config/settings/repos
            // bootstrap rows are still finishing. Those must not block chat.
            ...(env.active ? {} : { switchingTo: null }),
          },
        }))
        break
    default:
      return false
  }
  return true
}

import type { StoreGet, StoreSet } from '../api'
import type { PushEnvelope } from '../types/envelope'
import type { ImportGraphNode } from '../types/slices'

const IMPORT_GRAPH_RETRY_DELAYS_MS = [500, 1_000, 2_000, 3_000, 5_000] as const
let importGraphRetryTimer: ReturnType<typeof setTimeout> | null = null
let importGraphRetryAttempt = 0

function clearImportGraphRetry() {
  if (importGraphRetryTimer) clearTimeout(importGraphRetryTimer)
  importGraphRetryTimer = null
  importGraphRetryAttempt = 0
}

export function pushImportGraph(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'ImportGraph': {
        // Stale-reply guard: sessionId must always match the foreground
        // session. When a request is active, requestId must also match exactly;
        // null/uncorrelated replies are rejected.
        {
          const s = get().importGraph
          if (env.sessionId !== get().session.id) break
          if (s.activeRequestId != null && env.requestId !== s.activeRequestId) break
          if (s.activeSessionId != null && env.sessionId !== s.activeSessionId) break
        }

        if (env.status !== 'ok') {
          const status = (env.status === 'scanning' ? 'scanning'
            : env.status === 'not-indexed' ? 'not-indexed'
            : 'unavailable') as 'scanning' | 'not-indexed' | 'unavailable'
          // reindexBusy transitions: scanning retains it (still working);
          // not-indexed / unavailable are terminal — clear it.
          const reindexTerminal = status !== 'scanning'
          set((s) => ({
            importGraph: {
              ...s.importGraph,
              status,
              loading: false,
              queuedRefresh: false,
              error: status === 'unavailable' ? 'Linker daemon is not reachable.'
                : status === 'not-indexed' ? 'Workspace not indexed — use reindex to scan.'
                : null,
              availableRoots: env.availableRoots ?? [],
              ...(reindexTerminal
                ? { reindexBusy: false, reindexError: status === 'unavailable' ? 'Linker daemon is not reachable.' : null }
                : {}),
            },
          }))

          // Registration and the initial full scan run asynchronously. Retry a
          // bounded number of times without replacing the last valid graph.
          // Only retry for scanning / unavailable; not-indexed is terminal.
          if (status !== 'not-indexed' && !importGraphRetryTimer && importGraphRetryAttempt < IMPORT_GRAPH_RETRY_DELAYS_MS.length) {
            const sessionId = get().session.id
            const delay = IMPORT_GRAPH_RETRY_DELAYS_MS[importGraphRetryAttempt]
            importGraphRetryAttempt += 1
            importGraphRetryTimer = setTimeout(() => {
              importGraphRetryTimer = null
              const state = get()
              if (state.session.id === sessionId
                && (state.importGraph.status === 'scanning' || state.importGraph.status === 'unavailable')) {
                state.refreshImportGraph(state.importGraph.focus)
              }
            }, delay)
          }
          break
        }

        clearImportGraphRetry()
        const queued = get().importGraph.queuedRefresh
        if (queued) {
          // Stale/coalesced reply: discard view data but keep workspace metadata.
          // treeNodes must NOT be mutated — the queued refresh will produce fresh data.
          set((s) => ({
            importGraph: {
              ...s.importGraph,
              availableRoots: env.availableRoots ?? [],
              status: 'ok',
              loading: false,
              queuedRefresh: false,
              error: null,
              reindexBusy: false,
              reindexError: null,
            },
          }))
          get().refreshImportGraph(get().importGraph.focus)
          break
        }
        // Replace the browser catalogue only when the overview represents every
        // active root/language. Filtered overviews are partial and must be merged.
        const replaceTree =
          env.focus === null &&
          get().importGraph.filterRoots.length === 0 &&
          get().importGraph.filterLanguages.length === 0
        set((s) => {
          let treeNodes: ImportGraphNode[]
          if (replaceTree) {
            treeNodes = env.nodes
          } else {
            const existing = new Map(s.importGraph.treeNodes.map((n) => [n.path, n]))
            for (const node of env.nodes) {
              existing.set(node.path, node)
            }
            treeNodes = Array.from(existing.values())
          }
          return {
            importGraph: {
              ...s.importGraph,
              nodes: env.nodes,
              edges: env.edges,
              focus: env.focus,
              generation: env.generation,
              fileCount: env.fileCount,
              edgeCount: env.edgeCount,
              languages: env.languages,
              nodesTruncated: env.nodesTruncated,
              edgesTruncated: env.edgesTruncated,
              totalNodesAvailable: env.totalNodesAvailable,
              totalEdgesAvailable: env.totalEdgesAvailable,
              availableRoots: env.availableRoots ?? [],
              status: 'ok',
              loading: false,
              error: null,
              reindexBusy: false,
              reindexError: null,
              treeNodes,
            },
          }
        })
        break
      }
      case 'ImportGraphImpact': {
        // Reject stale: requestId must match, path must match current impact
        // target, and sessionId must match (prevents cross-session bleed).
        set((s) => {
          if (env.requestId !== s.importGraph.impactRequestId) return s
          if (env.path !== s.importGraph.impactPath) return s
          if (env.sessionId != null && env.sessionId !== s.session.id) return s
          return {
            importGraph: {
              ...s.importGraph,
              impactStatus: env.error ? 'error' as const : 'loaded' as const,
              impactPaths: env.paths,
              impactTotal: env.total,
              impactError: env.error,
            },
          }
        })
        break
      }
    default:
      return false
  }
  return true
}

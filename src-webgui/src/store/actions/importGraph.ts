import type { StoreGet, StoreSet } from '../api'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

export function importGraphActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openImportGraphTab' | 'refreshImportGraph' | 'setImportGraphDepth' | 'setImportGraphDirection' | 'selectImportGraphNode' | 'clearImportGraphSelection' | 'navigateBreadcrumb' | 'popBreadcrumb' | 'clearBreadcrumb' | 'setImportGraphRootFilter' | 'setImportGraphLanguageFilter' | 'reindexImportGraph' | 'requestImportGraphImpact'> {
  return {
  openImportGraphTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'import-graph')
      const tabs: Tab[] = exists
        ? s.ui.tabs
        : [...s.ui.tabs, { id: 'import-graph', kind: 'importGraph' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'import-graph' } }
    })
    // No wire fetch here — the ImportGraphTab fires refreshImportGraph on mount.
  },
  refreshImportGraph: (path) => {
    const g = get().importGraph
    if (g.loading) {
      // A request is in flight — coalesce: set the queued flag so the
      // ImportGraph reducer replays once the old reply lands.
      const focus = path !== undefined ? path : g.focus
      set((s) => ({
        importGraph: {
          ...s.importGraph,
          queuedRefresh: true,
          focus,
        },
      }))
      return
    }
    const focus = path !== undefined ? path : g.focus
    const isFullRefresh = path === undefined
    const requestId = `graph-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
    const sessionId = get().session.id
    // Daemon graph keys use canonical registered roots. Never send a raw or
    // stale settings path as a server-side filter; an unmatched filter makes a
    // healthy graph look empty.
    const canonicalFilterRoots = g.filterRoots.filter((root) =>
      g.availableRoots.some((available) => available.root === root),
    )
    set((s) => ({
      importGraph: {
        ...s.importGraph,
        loading: true,
        error: null,
        focus,
        queuedRefresh: false,
        activeRequestId: requestId,
        activeSessionId: sessionId,
        // Strict: every focused request sends literal depth:1, direction:'both'.
        depth: 1,
        direction: 'both' as const,
        // Keep the last valid graph visible while the replacement is in flight.
        // Impact data is request-specific and must still be cleared.
        impactRequestId: null,
        impactPath: null,
        impactStatus: 'idle' as const,
        impactPaths: [],
        impactTotal: 0,
        impactError: null,
        ...(isFullRefresh ? { breadcrumb: [] } : {}),
      },
    }))
    get().req({
      r: 'ImportGraph',
      path: focus ?? null,
      depth: 1,
      direction: 'both',
      filterRoots: canonicalFilterRoots.length > 0 ? canonicalFilterRoots : null,
      filterLanguages: g.filterLanguages.length > 0 ? g.filterLanguages : null,
      requestId,
    })
  },
  setImportGraphDepth: (depth) => {
    set((s) => ({ importGraph: { ...s.importGraph, depth } }))
    // Re-fetch if there's a focus or any data
    if (get().importGraph.focus || get().importGraph.nodes.length > 0) {
      get().refreshImportGraph(get().importGraph.focus)
    }
  },
  setImportGraphDirection: (dir) => {
    set((s) => ({ importGraph: { ...s.importGraph, direction: dir } }))
    if (get().importGraph.focus || get().importGraph.nodes.length > 0) {
      get().refreshImportGraph(get().importGraph.focus)
    }
  },
  selectImportGraphNode: (path) => {
    if (!path) {
      set((s) => ({
        importGraph: { ...s.importGraph, selectedPath: null },
      }))
      return
    }
    // Push to breadcrumb chain for navigation history
    const s = get().importGraph
    const alreadyFocused = s.focus === path
    const newBreadcrumb = alreadyFocused
      ? s.breadcrumb
      : [...s.breadcrumb, path]
    set((s) => ({
      importGraph: {
        ...s.importGraph,
        selectedPath: path,
        focus: path,
        breadcrumb: newBreadcrumb,
        // Clear impact state atomically before new request.
        impactRequestId: null,
        impactPath: null,
        impactStatus: 'idle' as const,
        impactPaths: [],
        impactTotal: 0,
        impactError: null,
      },
    }))
    // Fetch the new neighborhood (merged in push reducer)
    get().refreshImportGraph(path)
  },
  clearImportGraphSelection: () => {
    set((s) => ({
      importGraph: {
        ...s.importGraph,
        selectedPath: null,
      },
    }))
  },
  navigateBreadcrumb: (idx) => {
    const bc = get().importGraph.breadcrumb
    if (idx < 0 || idx >= bc.length) return
    const path = bc[idx]
    // Truncate breadcrumb to this point
    set((s) => ({
      importGraph: {
        ...s.importGraph,
        breadcrumb: s.importGraph.breadcrumb.slice(0, idx + 1),
        selectedPath: path,
        focus: path,
      },
    }))
    get().refreshImportGraph(path)
  },
  popBreadcrumb: () => {
    const bc = get().importGraph.breadcrumb
    if (bc.length < 2) {
      // Back to overview: clear focus/selection/breadcrumb and fetch overview.
      set((s) => ({
        importGraph: { ...s.importGraph, breadcrumb: [], selectedPath: null, focus: null },
      }))
      get().refreshImportGraph(null)
      return
    }
    const newBc = bc.slice(0, -1)
    const path = newBc[newBc.length - 1]
    set((s) => ({
      importGraph: {
        ...s.importGraph,
        breadcrumb: newBc,
        selectedPath: path,
        focus: path,
      },
    }))
    get().refreshImportGraph(path)
  },
  clearBreadcrumb: () => {
    set((s) => ({
      importGraph: { ...s.importGraph, breadcrumb: [] },
    }))
  },
  setImportGraphRootFilter: (requestedRoots) => {
    const s = get().importGraph
    const roots = requestedRoots.filter((root) =>
      s.availableRoots.some((available) => available.root === root),
    )
    // Determine if the current focus is excluded by the new root filter.
    let focusCleared = false
    let focus = s.focus
    let selectedPath = s.selectedPath
    let breadcrumb = s.breadcrumb
    if (focus) {
      const focusNode = s.nodes.find((n) => n.path === focus)
      if (focusNode && roots.length > 0 && focusNode.workspaceRoot && !roots.includes(focusNode.workspaceRoot)) {
        focus = null
        selectedPath = null
        breadcrumb = []
        focusCleared = true
      }
    }
    // Also clear selectedPath if the selected node would be excluded even if focus stays.
    if (!focusCleared && selectedPath) {
      const selNode = s.nodes.find((n) => n.path === selectedPath)
      if (selNode && roots.length > 0 && selNode.workspaceRoot && !roots.includes(selNode.workspaceRoot)) {
        selectedPath = null
      }
    }
    // Prune language selections that have no files under the newly selected roots.
    let filterLanguages = s.filterLanguages
    if (filterLanguages.length > 0 && roots.length > 0) {
      // Build set of languages present under the selected roots.
      const langsUnderRoots = new Set<string>()
      for (const node of s.nodes) {
        if (node.workspaceRoot && roots.includes(node.workspaceRoot)) {
          langsUnderRoots.add(node.language)
        }
      }
      // Also check availableRoots for a comprehensive picture.
      for (const r of s.availableRoots) {
        if (roots.includes(r.root)) {
          for (const lc of r.languages) {
            langsUnderRoots.add(lc.name)
          }
        }
      }
      const pruned = filterLanguages.filter((l) => langsUnderRoots.has(l))
      if (pruned.length !== filterLanguages.length) {
        filterLanguages = pruned.length > 0 ? pruned : []
      }
    }
    set((st) => ({
      importGraph: { ...st.importGraph, filterRoots: roots, focus, selectedPath, breadcrumb, filterLanguages },
    }))
    if (focusCleared) {
      // Fetch overview (no focus) since the focused node is excluded.
      get().refreshImportGraph(null)
    } else {
      get().refreshImportGraph(focus)
    }
  },
  setImportGraphLanguageFilter: (langs) => {
    const s = get().importGraph
    // Determine if the current focus is excluded by the new language filter.
    let focusCleared = false
    let focus = s.focus
    let selectedPath = s.selectedPath
    let breadcrumb = s.breadcrumb
    if (focus) {
      const focusNode = s.nodes.find((n) => n.path === focus)
      if (focusNode && langs.length > 0 && !langs.includes(focusNode.language)) {
        focus = null
        selectedPath = null
        breadcrumb = []
        focusCleared = true
      }
    }
    // Also clear selectedPath if excluded even if focus stays.
    if (!focusCleared && selectedPath) {
      const selNode = s.nodes.find((n) => n.path === selectedPath)
      if (selNode && langs.length > 0 && !langs.includes(selNode.language)) {
        selectedPath = null
      }
    }
    set((st) => ({
      importGraph: { ...st.importGraph, filterLanguages: langs, focus, selectedPath, breadcrumb },
    }))
    if (focusCleared) {
      get().refreshImportGraph(null)
    } else {
      get().refreshImportGraph(focus)
    }
  },
  reindexImportGraph: () => {
    if (get().importGraph.reindexBusy) return
    const requestId = `reindex-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
    const sessionId = get().session.id
    set((s) => ({
      importGraph: { ...s.importGraph, reindexBusy: true, reindexError: null, activeRequestId: requestId, activeSessionId: sessionId },
    }))
    get().req({ r: 'ImportGraphReindex', requestId })
    // Safety-net timeout: if no ImportGraph reply arrives within 15s
    // (e.g. daemon lost mid-reindex), clear reindexBusy with an actionable
    // error so the UI never stays in a permanent spinner.
    setTimeout(() => {
      if (get().importGraph.reindexBusy && get().importGraph.activeRequestId === requestId) {
        set((s) => ({
          importGraph: {
            ...s.importGraph,
            reindexBusy: false,
            reindexError: 'Reindex timed out — linker daemon may be unresponsive. Try again.',
          },
        }))
      }
    }, 15_000)
  },
  requestImportGraphImpact: (path: string) => {
    const id = `impact-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
    set((s) => ({
      importGraph: {
        ...s.importGraph,
        impactRequestId: id,
        impactPath: path,
        impactStatus: 'loading' as const,
        impactPaths: [],
        impactTotal: 0,
        impactError: null,
      },
    }))
    get().req({ r: 'ImportGraphImpact', path, depth: 3, requestId: id, sessionId: get().session.id })
  },
  }
}

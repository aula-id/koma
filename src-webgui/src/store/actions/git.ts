import type { StoreGet, StoreSet } from '../api'
import { initialActivity, initialGraph, initialImportGraph, tabBaseName } from '../initial'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

let branchListRequestSeq = 0

export function gitActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'refreshGitStatus' | 'openGitDiffTab' | 'openGraphTab' | 'refreshGraph' | 'loadMoreGraph' | 'selectCommit' | 'clearSelection' | 'setGraphMode' | 'refreshActivity' | 'openCommitDiffTab' | 'setCommitDraft' | 'gitStage' | 'gitUnstage' | 'gitDiscard' | 'gitCommit' | 'setGitKey' | 'gitFetch' | 'gitPull' | 'gitPush' | 'refreshBranches' | 'refreshRepos' | 'setActiveRepo' | 'gitStash' | 'gitStashPop' | 'refreshStashes' | 'gitCheckout' | 'gitCreateBranch' | 'gitCherryPick' | 'gitRevert' | 'gitReset' | 'gitMerge' | 'gitRebase' | 'gitOpAbort' | 'gitOpContinue' | 'refreshKeys' | 'keyGenerate' | 'keyImport' | 'keyReveal' | 'clearKeyReveal' | 'keyDelete'> {
  return {
  refreshGitStatus: () => {
    get().req({ r: 'GitStatus' })
  },
  openGitDiffTab: (path, staged) => {
    const root = get().git.root
    if (root) {
      const id = `gittool:${root}:diff:${staged}:${path}`
      const entry = [...get().git.staged, ...get().git.unstaged].find(e => e.path === path)
      set(s => ({ ui: { ...s.ui, tabs: s.ui.tabs.some(t => t.id === id) ? s.ui.tabs : [...s.ui.tabs,
        { id, kind: 'gitTool', view: 'diff', root, path, staged, oldPath: entry?.origPath ?? undefined, title: `${tabBaseName(path)}${staged ? ' (staged)' : ''}` }], activeTabId: id } }))
      return
    }
    const id = `gitdiff:${staged ? 'staged' : 'unstaged'}:${path}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const title = `${tabBaseName(path)}${staged ? ' (staged)' : ''}`
      const tabs: Tab[] = exists
        ? s.ui.tabs.map((t) => (t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t))
        : [...s.ui.tabs, { id, kind: 'diff', path, title, loading: true, staged }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    get().req({ r: 'GitDiff', path, staged })
  },
  openGraphTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'graph')
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, { id: 'graph', kind: 'graph' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'graph' } }
    })
    // No wire fetch here — the GraphTab fires refreshGraph on mount.
  },
  refreshGraph: () => {
    // Serialize: at most one GitGraph request in flight, ever. If a load-more
    // (or another refresh) is already in flight, defer instead of racing it —
    // the 'GitGraph' reducer replays this once that reply lands and clears
    // `loading`. This keeps `loadMode` unambiguous for whichever reply comes
    // back next.
    if (get().graph.loading) {
      set((s) => ({ graph: { ...s.graph, pendingRefresh: true } }))
      return
    }
    set((s) => ({
      graph: { ...s.graph, loading: true, loadMode: 'replace', pendingRefresh: false },
    }))
    get().req({ r: 'GitGraph', limit: 200, skip: 0 })
  },
  loadMoreGraph: () => {
    const g = get().graph
    // Guard against a duplicate in-flight load and a no-op past the last page.
    if (g.loading || !g.hasMore) return
    set((s) => ({ graph: { ...s.graph, loading: true, loadMode: 'append' } }))
    get().req({ r: 'GitGraph', limit: 200, skip: g.commits.length })
  },
  selectCommit: (sha) => {
    set((s) => ({
      graph: {
        ...s.graph,
        selectedSha: sha,
        // Clear stale detail when the selection actually changes so the pane
        // shows a loading state instead of the previous commit's detail.
        detail: s.graph.selectedSha === sha ? s.graph.detail : null,
      },
    }))
    get().req({ r: 'GitCommitDetail', sha })
  },
  clearSelection: () => set((s) => ({ graph: { ...s.graph, selectedSha: null, detail: null } })),
  setGraphMode: (mode) => {
    set((s) => ({ graph: { ...s.graph, graphMode: mode } }))
  },
  // ─── Import Graph actions ──────────────────────────────────────────,
  refreshActivity: (path) => {
    const p = path ?? null
    set((s) => ({ activity: { ...s.activity, loading: true, path: p } }))
    get().req({ r: 'GitActivity', path: p, limit: 800 })
  },
  openCommitDiffTab: (sha, path, oldPath) => {
    const root = get().git.root
    if (root) {
      const id = `gittool:${root}:diff:${sha}:${path}`
      set(s => ({ ui: { ...s.ui, tabs: s.ui.tabs.some(t => t.id === id) ? s.ui.tabs : [...s.ui.tabs,
        { id, kind: 'gitTool', view: 'diff', root, path, commit: sha, oldPath, title: `${tabBaseName(path)} @ ${sha.slice(0, 7)}` }], activeTabId: id } }))
      return
    }
    const id = `commitdiff:${sha}:${path}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const title = `${tabBaseName(path)} @ ${sha.slice(0, 7)}`
      const tabs: Tab[] = exists
        ? s.ui.tabs.map((t) => (t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t))
        : [...s.ui.tabs, { id, kind: 'diff', path, title, loading: true, commitSha: sha }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    get().req({ r: 'GitCommitDiff', sha, path })
  },
  setCommitDraft: (text) => set(() => ({ commitDraft: text })),
  gitStage: (paths) => {
    if (paths.length === 0) return
    get().req({ r: 'GitStage', paths })
  },
  gitUnstage: (paths) => {
    if (paths.length === 0) return
    get().req({ r: 'GitUnstage', paths })
  },
  gitDiscard: (paths) => {
    if (paths.length === 0) return
    get().req({ r: 'GitDiscard', paths })
  },
  gitCommit: (message) => {
    if (!message.trim()) return
    get().req({ r: 'GitCommit', message })
  },
  setGitKey: (name) => {
    get().req({ r: 'SetGitKey', name })
  },
  gitFetch: () => {
    set(() => ({ remoteBusy: 'fetch' }))
    get().req({ r: 'GitFetch' })
  },
  gitPull: () => {
    set(() => ({ remoteBusy: 'pull' }))
    get().req({ r: 'GitPull' })
  },
  gitPush: (mode = 'automatic') => {
    set(() => ({ remoteBusy: 'push' }))
    get().req({ r: 'GitPush', mode, root: get().git.root })
  },
  refreshBranches: () => {
    const requestId = ++branchListRequestSeq
    set(() => ({ branchesLoading: true, branchesRequestId: requestId }))
    get().req({ r: 'GitBranchList', requestId })
  },
  refreshRepos: () => {
    get().req({ r: 'GitRepos' })
  },
  setActiveRepo: (root) => {
    set((s) => ({
      activeRepoRoot: root,
      graph: { ...initialGraph, graphMode: s.graph.graphMode },
      importGraph: initialImportGraph,
      activity: initialActivity,
      branches: [],
      branchesLoading: false,
      branchesRequestId: null,
    }))
    get().req({ r: 'SetActiveRepo', root })
  },
  gitStash: () => {
    get().req({ r: 'GitStash' })
  },
  gitStashPop: () => {
    get().req({ r: 'GitStashPop' })
  },
  refreshStashes: () => {
    get().req({ r: 'GitStashList' })
  },
  gitCheckout: (ref) => {
    if (!ref.trim()) return
    get().req({ r: 'GitCheckout', ref, root: get().git.root })
  },
  gitCreateBranch: (name, start, checkout) => {
    if (!name.trim()) return
    get().req({ r: 'GitCreateBranch', name: name.trim(), start, checkout, root: get().git.root })
  },
  gitCherryPick: (sha) => {
    get().req({ r: 'GitCherryPick', sha })
  },
  gitRevert: (sha) => {
    get().req({ r: 'GitRevert', sha })
  },
  gitReset: (sha, mode) => {
    get().req({ r: 'GitReset', sha, mode })
  },
  gitMerge: (ref) => {
    if (!ref.trim()) return
    get().req({ r: 'GitMerge', ref })
  },
  gitRebase: (upstream, branch) => {
    if (!upstream.trim()) return
    get().req({ r: 'GitRebase', upstream, branch: branch ?? null })
  },
  gitOpAbort: (kind) => {
    get().req({ r: 'GitOpAbort', kind })
  },
  gitOpContinue: (kind) => {
    get().req({ r: 'GitOpContinue', kind })
  },
  refreshKeys: () => {
    get().req({ r: 'KeyList' })
  },
  keyGenerate: (name, comment) => {
    if (!name.trim()) return
    get().req({ r: 'KeyGenerate', name: name.trim(), comment })
  },
  keyImport: (name, privateKey) => {
    if (!name.trim() || !privateKey.trim()) return
    get().req({ r: 'KeyImport', name: name.trim(), privateKey })
  },
  keyReveal: (name, priv) => {
    get().req({ r: 'KeyReveal', name, private: priv })
  },
  clearKeyReveal: () => set(() => ({ keyRevealResult: null })),
  keyDelete: (name) => {
    get().req({ r: 'KeyDelete', name })
  },
  }
}

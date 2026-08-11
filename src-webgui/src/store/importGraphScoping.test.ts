import assert from 'node:assert/strict'

// ── Browser globals shim ──
const browser = globalThis as unknown as {
  window?: {
    ipc?: { postMessage(message: string): void }
    confirm?: (message?: string) => boolean
  }
}
const ipcCalls: unknown[] = []
browser.window = {
  ipc: { postMessage: (msg: string) => { ipcCalls.push(JSON.parse(msg)) } },
  confirm: () => false,
}

const { useKoma } = await import('../store/koma')
const { sourceLanguage } = await import('../lib/importGraphLanguages')

// ── Helpers ──
const WORKDIRS = ['/ws/a', '/ws/b']
const BACKEND_ROOTS = [
  { root: '/ws/a', fileCount: 42, languages: [{ name: 'Rust', count: 30 }, { name: 'TypeScript', count: 12 }], indexedState: 'indexed' as const },
  { root: '/ws/b', fileCount: 10, languages: [{ name: 'Python', count: 10 }], indexedState: 'indexed' as const },
  { root: '/foreign', fileCount: 5, languages: [{ name: 'Go', count: 5 }], indexedState: 'indexed' as const },
]
const BACKEND_ROOTS_WITH_PATHS = [
  { root: '/canonical/a', configuredPath: '/symlink/to/a', displayPath: 'a', fileCount: 42, languages: [{ name: 'Rust', count: 42 }], indexedState: 'indexed' as const },
  { root: '/canonical/b', displayPath: 'b', fileCount: 10, languages: [{ name: 'Python', count: 10 }], indexedState: 'indexed' as const },
]

function resetStore() {
  ipcCalls.length = 0
  useKoma.setState((s) => ({
    importGraph: {
      ...s.importGraph, status: 'idle', nodes: [], edges: [], focus: null, generation: 0,
      fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false,
      totalNodesAvailable: 0, totalEdgesAvailable: 0, loading: false, error: null,
      selectedPath: null, depth: 1, direction: 'both', breadcrumb: [], availableRoots: [],
      filterRoots: [], filterLanguages: [], queuedRefresh: false, treeNodes: [],
      impactRequestId: null, impactPath: null, impactDepth: 3, impactStatus: 'idle',
      impactPaths: [], impactTotal: 0, impactError: null, reindexBusy: false,
      reindexError: null, activeRequestId: null, activeSessionId: null,
    },
    settingsValues: {
      name: 'test', workdir: WORKDIRS, shortSend: false, slidingCache: false,
      bashSaving: false, codingAutosave: false, internetMode: 'simple', palette: 'dark', effort: '',
    },
    session: { ...s.session, id: 'test-session' },
  }))
}

function igPush(p: Record<string, unknown>) {
  useKoma.getState().push({ k: 'ImportGraph', sessionId: 'test-session', ...p } as never)
}

// ── TEST 1: reindexImportGraph sends ImportGraphReindex with requestId
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const s = useKoma.getState().importGraph
  assert.equal(s.reindexBusy, true)
  assert.ok(s.activeRequestId!.startsWith('reindex-'))
  assert.equal(s.activeSessionId, 'test-session')
  assert.equal(ipcCalls.length, 1)
  assert.equal((ipcCalls[0] as Record<string, unknown>).requestId, s.activeRequestId)
  console.log('TEST 1 passed: reindexImportGraph sends requestId')
}

// ── TEST 2: reindexImportGraph coalesces
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const first = useKoma.getState().importGraph.activeRequestId
  useKoma.getState().reindexImportGraph()
  assert.equal(ipcCalls.length, 1)
  assert.equal(useKoma.getState().importGraph.activeRequestId, first)
  console.log('TEST 2 passed: reindexImportGraph coalesces')
}

// ── TEST 3: ImportGraph success clears reindexBusy
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const reqId = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'ok', nodes: [{ path: 'a.rs', language: 'Rust', outDegree: 1, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [{ from: 'a.rs', to: 'b.rs' }], focus: null, generation: 1, fileCount: 42, edgeCount: 10, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 10, availableRoots: BACKEND_ROOTS, requestId: reqId, sessionId: 'test-session' })
  const s = useKoma.getState().importGraph
  assert.equal(s.reindexBusy, false)
  assert.equal(s.status, 'ok')
  assert.equal(s.nodes.length, 1)
  console.log('TEST 3 passed: ImportGraph success clears reindexBusy')
}

// ── TEST 4: Stale reply ignored (wrong requestId)
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const cur = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'ok', nodes: [{ path: 'x.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 100, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 100, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: 'wrong', sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.reindexBusy, true, 'stale not applied')
  igPush({ status: 'ok', nodes: [{ path: 'v.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 42, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: cur, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.reindexBusy, false, 'matching applied')
  console.log('TEST 4 passed: stale requestId ignored, matching applied')
}

// ── TEST 5: Stale reply ignored (wrong sessionId)
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const reqId = useKoma.getState().importGraph.activeRequestId
  useKoma.setState((s) => ({ session: { ...s.session, id: 'new-session' } }))
  igPush({ status: 'ok', nodes: [{ path: 'x.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 100, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 100, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: reqId, sessionId: 'old-session' })
  assert.equal(useKoma.getState().importGraph.nodes.length, 0, 'wrong sessionId rejected')
  console.log('TEST 5 passed: stale sessionId rejected')
}

// ── TEST 6: scanning keeps reindexBusy; not-indexed/unavailable clears it
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const r6 = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'scanning', nodes: [], edges: [], focus: null, generation: 0, fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 0, totalEdgesAvailable: 0, availableRoots: [], requestId: r6, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.reindexBusy, true)
  assert.equal(useKoma.getState().importGraph.status, 'scanning')
  igPush({ status: 'not-indexed', nodes: [], edges: [], focus: null, generation: 0, fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 0, totalEdgesAvailable: 0, availableRoots: [{ root: '/ws/a', fileCount: 0, languages: [], indexedState: 'not-indexed' }, { root: '/ws/b', fileCount: 0, languages: [], indexedState: 'not-indexed' }], requestId: r6, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.reindexBusy, false)
  assert.equal(useKoma.getState().importGraph.status, 'not-indexed')
  useKoma.getState().reindexImportGraph()
  const r6b = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'unavailable', nodes: [], edges: [], focus: null, generation: 0, fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 0, totalEdgesAvailable: 0, availableRoots: [], requestId: r6b, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.reindexBusy, false)
  assert.equal(useKoma.getState().importGraph.reindexError, 'Linker daemon is not reachable.')
  console.log('TEST 6 passed: scanning keeps reindexBusy; terminal clears it')
}

// ── TEST 7: Reindex retains graph
{
  resetStore()
  igPush({ status: 'ok', nodes: [{ path: 'x.rs', language: 'Rust', outDegree: 0, inDegree: 1, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [{ from: 'y.rs', to: 'x.rs' }], focus: null, generation: 1, fileCount: 100, edgeCount: 50, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 100, totalEdgesAvailable: 50, availableRoots: BACKEND_ROOTS })
  useKoma.getState().reindexImportGraph()
  const r7 = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'scanning', nodes: [], edges: [], focus: null, generation: 0, fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 0, totalEdgesAvailable: 0, availableRoots: [], requestId: r7, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.nodes.length, 1, 'graph retained during reindex')
  console.log('TEST 7 passed: reindex retains graph')
}

// ── TEST 8: Root ordering from backend availableRoots
{
  resetStore()
  igPush({ status: 'ok', nodes: [], edges: [], focus: null, generation: 1, fileCount: 52, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 52, totalEdgesAvailable: 0, availableRoots: [
    { root: '/canonical/b', fileCount: 10, languages: [], indexedState: 'indexed' as const },
    { root: '/canonical/a', fileCount: 42, languages: [], indexedState: 'indexed' as const },
    { root: '/foreign', fileCount: 5, languages: [], indexedState: 'indexed' as const },
  ]})
  const roots = useKoma.getState().importGraph.availableRoots.map((r) => r.root)
  assert.deepEqual(roots, ['/canonical/b', '/canonical/a', '/foreign'])
  console.log('TEST 8 passed: root ordering from backend')
}

// ── TEST 9: configuredPath/displayPath from backend
{
  resetStore()
  igPush({ status: 'ok', nodes: [], edges: [], focus: null, generation: 1, fileCount: 52, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 52, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS_WITH_PATHS })
  const r = useKoma.getState().importGraph.availableRoots
  const a = r.find((x) => x.root === '/canonical/a')!
  assert.equal(a.configuredPath, '/symlink/to/a')
  assert.equal(a.displayPath, 'a')
  const b = r.find((x) => x.root === '/canonical/b')!
  assert.equal(b.configuredPath, undefined)
  assert.equal(b.displayPath, 'b')
  console.log('TEST 9 passed: configuredPath/displayPath')
}

// ── TEST 10: refreshImportGraph sends requestId on wire
{
  resetStore()
  useKoma.getState().refreshImportGraph(null)
  const s = useKoma.getState().importGraph
  assert.ok(s.activeRequestId!.startsWith('graph-'))
  assert.equal(s.activeSessionId, 'test-session')
  assert.equal(ipcCalls.length, 1)
  assert.equal((ipcCalls[0] as Record<string, unknown>).requestId, s.activeRequestId)
  console.log('TEST 10 passed: requestId on wire')
}

// ── TEST 11: Strict rejection — null requestId when active
{
  resetStore()
  useKoma.getState().refreshImportGraph(null)
  const ari = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'ok', nodes: [{ path: 'no.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 99, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 99, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.nodes.length, 0, 'null requestId rejected')
  igPush({ status: 'ok', nodes: [{ path: 'ok.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 42, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: ari, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.nodes.length, 1, 'matching accepted')
  console.log('TEST 11 passed: null requestId rejected when active')
}

// ── TEST 12: Strict rejection — null sessionId when active
{
  resetStore()
  useKoma.getState().reindexImportGraph()
  const rid = useKoma.getState().importGraph.activeRequestId
  igPush({ status: 'ok', nodes: [{ path: 'x.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 99, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 99, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: rid, sessionId: null })
  assert.equal(useKoma.getState().importGraph.nodes.length, 0, 'null sessionId rejected')
  igPush({ status: 'ok', nodes: [{ path: 'ok.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 42, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: rid, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.nodes.length, 1, 'matching accepted')
  console.log('TEST 12 passed: null sessionId rejected when active')
}

// ── TEST 13: Impact sessionId validation
{
  resetStore()
  useKoma.getState().requestImportGraphImpact('/ws/a/main.rs')
  const irid = useKoma.getState().importGraph.impactRequestId
  useKoma.getState().push({ k: 'ImportGraphImpact', requestId: irid!, sessionId: 'wrong', path: '/ws/a/main.rs', depth: 3, paths: [], total: 0, error: null })
  assert.equal(useKoma.getState().importGraph.impactStatus, 'loading', 'wrong sessionId rejected')
  useKoma.getState().push({ k: 'ImportGraphImpact', requestId: irid!, sessionId: 'test-session', path: '/ws/a/main.rs', depth: 3, paths: ['/ws/a/other.rs'], total: 1, error: null })
  assert.equal(useKoma.getState().importGraph.impactStatus, 'loaded')
  console.log('TEST 13 passed: impact sessionId validation')
}

// ── TEST 14: Queued refresh coalescing
{
  resetStore()
  igPush({ status: 'ok', nodes: [{ path: 'a.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 42, edgeCount: 10, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 10, availableRoots: BACKEND_ROOTS })
  useKoma.getState().refreshImportGraph(null)
  const qrid = useKoma.getState().importGraph.activeRequestId
  assert.equal(useKoma.getState().importGraph.loading, true)
  useKoma.getState().refreshImportGraph(null)
  assert.equal(useKoma.getState().importGraph.queuedRefresh, true)
  igPush({ status: 'ok', nodes: [], edges: [], focus: null, generation: 1, fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 0, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS, requestId: qrid, sessionId: 'test-session' })
  assert.equal(useKoma.getState().importGraph.queuedRefresh, false, 'queuedRefresh cleared after replay')
  console.log('TEST 14 passed: queuedRefresh coalescing')
}

// ── TEST 15: SettingsValues prunes using canonical availableRoots
{
  resetStore()
  igPush({ status: 'ok', nodes: [{ path: '/ws/a/a.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 42, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 0, availableRoots: [
    { root: '/ws/a', fileCount: 42, languages: [{ name: 'Rust', count: 42 }], indexedState: 'indexed' as const },
    { root: '/ws/b', fileCount: 10, languages: [{ name: 'Python', count: 10 }], indexedState: 'indexed' as const },
  ]})
  useKoma.setState((s) => ({ importGraph: { ...s.importGraph, filterRoots: ['/ws/a', '/ws/b'], filterLanguages: ['Rust', 'Python'], treeNodes: [
    { path: '/ws/a/a.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview' as const, depthFromFocus: null, workspaceRoot: '/ws/a' },
    { path: '/ws/b/b.py', language: 'Python', outDegree: 0, inDegree: 0, role: 'Overview' as const, depthFromFocus: null, workspaceRoot: '/ws/b' },
  ] } }))
  useKoma.getState().push({ k: 'SettingsValues', name: 'test', workdir: ['/ws/a'], shortSend: false, slidingCache: false, bashSaving: false, codingAutosave: false, internetMode: 'simple', palette: 'dark', effort: '' })
  const s = useKoma.getState().importGraph
  assert.deepEqual(s.filterRoots, ['/ws/a'])
  assert.deepEqual(s.filterLanguages, ['Rust'])
  assert.equal(s.treeNodes.length, 1)
  console.log('TEST 15 passed: SettingsValues prunes using canonical availableRoots')
}

// ── TEST 16: indexedState 'unavailable' accepted
{
  resetStore()
  igPush({ status: 'unavailable', nodes: [], edges: [], focus: null, generation: 0, fileCount: 0, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 0, totalEdgesAvailable: 0, availableRoots: [{ root: '/ws/a', fileCount: 0, languages: [], indexedState: 'unavailable' }] })
  assert.equal(useKoma.getState().importGraph.availableRoots[0].indexedState, 'unavailable')
  console.log('TEST 16 passed: indexedState unavailable accepted')
}

// ── TEST 17: sourceLanguage covers extensions
{
  assert.equal(sourceLanguage('foo.rs'), 'Rust')
  assert.equal(sourceLanguage('bar.py'), 'Python')
  assert.equal(sourceLanguage('baz.go'), 'Go')
  assert.equal(sourceLanguage('Q.java'), 'Java')
  assert.equal(sourceLanguage('a.ts'), 'TypeScript')
  assert.equal(sourceLanguage('b.tsx'), 'TypeScript')
  assert.equal(sourceLanguage('c.js'), 'JavaScript')
  assert.equal(sourceLanguage('d.jsx'), 'JavaScript')
  assert.equal(sourceLanguage('e.mjs'), 'JavaScript')
  assert.equal(sourceLanguage('e.cjs'), 'JavaScript')
  assert.equal(sourceLanguage('f.php'), 'Php')
  assert.equal(sourceLanguage('g.c'), 'C')
  assert.equal(sourceLanguage('g.h'), 'C')
  assert.equal(sourceLanguage('h.cpp'), 'Cpp')
  assert.equal(sourceLanguage('h.cc'), 'Cpp')
  assert.equal(sourceLanguage('h.hpp'), 'Cpp')
  assert.equal(sourceLanguage('i.dart'), 'Dart')
  assert.equal(sourceLanguage('j.swift'), 'Swift')
  assert.equal(sourceLanguage('readme.md'), null)
  assert.equal(sourceLanguage('Makefile'), null)
  assert.equal(sourceLanguage('foo'), null)
  console.log('TEST 17 passed: sourceLanguage covers extensions')
}

// ── TEST 18: sourceLanguage case-insensitive
{
  assert.equal(sourceLanguage('Foo.RS'), 'Rust')
  assert.equal(sourceLanguage('Bar.PY'), 'Python')
  assert.equal(sourceLanguage('Baz.GO'), 'Go')
  console.log('TEST 18 passed: sourceLanguage case-insensitive')
}

// ── TEST 19: Reply without active request accepted
{
  resetStore()
  igPush({ status: 'ok', nodes: [{ path: 'x.rs', language: 'Rust', outDegree: 0, inDegree: 0, role: 'Overview', depthFromFocus: null, workspaceRoot: '/ws/a' }], edges: [], focus: null, generation: 1, fileCount: 42, edgeCount: 0, languages: ['Rust'], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 42, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS })
  assert.equal(useKoma.getState().importGraph.nodes.length, 1, 'accepted when no active request')
  console.log('TEST 19 passed: no active request accepts reply')
}

// ── TEST 20: Canonical root IDs + displayPath labels
{
  resetStore()
  igPush({ status: 'ok', nodes: [], edges: [], focus: null, generation: 1, fileCount: 52, edgeCount: 0, languages: [], nodesTruncated: false, edgesTruncated: false, totalNodesAvailable: 52, totalEdgesAvailable: 0, availableRoots: BACKEND_ROOTS_WITH_PATHS })
  const a = useKoma.getState().importGraph.availableRoots.find((r) => r.root === '/canonical/a')!
  assert.equal(a.configuredPath, '/symlink/to/a')
  assert.equal(a.displayPath, 'a')
  useKoma.getState().setImportGraphRootFilter(['/canonical/a'])
  assert.deepEqual(useKoma.getState().importGraph.filterRoots, ['/canonical/a'])
  console.log('TEST 20 passed: canonical root IDs + displayPath labels')
}

console.log('importGraphScoping.test.ts: all assertions passed')

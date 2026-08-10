import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Network, RefreshCw, X, AlertTriangle, Search, ChevronRight } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { ImportGraphScene } from './ImportGraphScene'

// Detail pane width constants (mirrors GraphTab).
const DETAIL_W_MIN = 220
const DETAIL_W_MAX = 480
const DETAIL_W_DEFAULT = 300

// Role → color mapping for detail pane badges (lightweight, no Three.js).
const ROLE_COLORS: Record<string, string> = {
  Focus: '#3b82f6',
  Dependency: '#22c55e',
  Dependent: '#f97316',
  Overview: '#6b7280',
}

// ─── Main ImportGraphTab ────────────────────────────────────────────────
export default function ImportGraphTab() {
  // Store selectors
  const nodes = useKoma((s) => s.importGraph.nodes)
  const edges = useKoma((s) => s.importGraph.edges)
  const focus = useKoma((s) => s.importGraph.focus)
  const loading = useKoma((s) => s.importGraph.loading)
  const error = useKoma((s) => s.importGraph.error)
  const fileCount = useKoma((s) => s.importGraph.fileCount)
  const edgeCount = useKoma((s) => s.importGraph.edgeCount)
  const languages = useKoma((s) => s.importGraph.languages)
  const nodesTruncated = useKoma((s) => s.importGraph.nodesTruncated)
  const edgesTruncated = useKoma((s) => s.importGraph.edgesTruncated)
  const totalNodesAvailable = useKoma((s) => s.importGraph.totalNodesAvailable)
  const totalEdgesAvailable = useKoma((s) => s.importGraph.totalEdgesAvailable)
  const depth = useKoma((s) => s.importGraph.depth)
  const direction = useKoma((s) => s.importGraph.direction)
  const selectedPath = useKoma((s) => s.importGraph.selectedPath)
  const generation = useKoma((s) => s.importGraph.generation)
  const breadcrumb = useKoma((s) => s.importGraph.breadcrumb)

  const refreshImportGraph = useKoma((s) => s.refreshImportGraph)
  const setImportGraphDepth = useKoma((s) => s.setImportGraphDepth)
  const setImportGraphDirection = useKoma((s) => s.setImportGraphDirection)
  const selectImportGraphNode = useKoma((s) => s.selectImportGraphNode)
  const clearImportGraphSelection = useKoma((s) => s.clearImportGraphSelection)
  const navigateBreadcrumb = useKoma((s) => s.navigateBreadcrumb)
  const popBreadcrumb = useKoma((s) => s.popBreadcrumb)

  const isActiveTab = useKoma((s) => s.ui.activeTabId === 'import-graph')
  const sessionId = useKoma((s) => s.session.id)

  // Fetch on mount and session change.
  useEffect(() => {
    refreshImportGraph()
  }, [refreshImportGraph, sessionId])

  // ── Detail pane resize ────────────────────────────────────────────
  const [detailW, setDetailW] = useState(DETAIL_W_DEFAULT)
  const startDetailResize = (e: React.MouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startW = detailW
    const onMove = (ev: MouseEvent) => {
      setDetailW(Math.min(DETAIL_W_MAX, Math.max(DETAIL_W_MIN, startW - (ev.clientX - startX))))
    }
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.body.style.cursor = 'ew-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  // ── Omnisearch (Cmd+K) ──────────────────────────────────────────────
  const [showOmni, setShowOmni] = useState(false)
  const [omniQuery, setOmniQuery] = useState('')
  const [omniIdx, setOmniIdx] = useState(0)
  const omniInputRef = useRef<HTMLInputElement | null>(null)

  // Global Cmd+K / Ctrl+K listener.
  useEffect(() => {
    if (!isActiveTab) return
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault()
        setShowOmni((prev) => !prev)
        setOmniQuery('')
        setOmniIdx(0)
      }
      if (e.key === 'Escape' && showOmni) {
        setShowOmni(false)
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [isActiveTab, showOmni])

  // Focus the input when overlay opens.
  useEffect(() => {
    if (showOmni) {
      requestAnimationFrame(() => omniInputRef.current?.focus())
    }
  }, [showOmni])

  const omniResults = useMemo(() => {
    if (!omniQuery.trim()) {
      // Show nodes with highest connectivity first, then alphabetical.
      return [...nodes]
        .sort((a, b) => (b.inDegree + b.outDegree) - (a.inDegree + a.outDegree))
        .slice(0, 50)
    }
    const q = omniQuery.toLowerCase()
    return nodes
      .filter((n) => n.path.toLowerCase().includes(q) || n.language.toLowerCase().includes(q))
      .sort((a, b) => {
        // Prioritize exact filename match, then path match.
        const aName = a.path.split('/').pop()?.toLowerCase() ?? ''
        const bName = b.path.split('/').pop()?.toLowerCase() ?? ''
        const aExact = aName === q ? 0 : aName.startsWith(q) ? 1 : a.path.toLowerCase().includes(q) ? 2 : 3
        const bExact = bName === q ? 0 : bName.startsWith(q) ? 1 : b.path.toLowerCase().includes(q) ? 2 : 3
        if (aExact !== bExact) return aExact - bExact
        return (b.inDegree + b.outDegree) - (a.inDegree + a.outDegree)
      })
      .slice(0, 30)
  }, [nodes, omniQuery])

  const selectOmniResult = useCallback(
    (path: string) => {
      selectImportGraphNode(path)
      setShowOmni(false)
      setOmniQuery('')
    },
    [selectImportGraphNode],
  )

  // ── Selected node details ─────────────────────────────────────────
  const selectedNode = useMemo(
    () => (selectedPath ? nodes.find((n) => n.path === selectedPath) : null),
    [nodes, selectedPath],
  )
  const directDeps = useMemo(() => {
    if (!selectedPath) return []
    return edges.filter((e) => e.from === selectedPath).map((e) => e.to)
  }, [edges, selectedPath])
  const directDependents = useMemo(() => {
    if (!selectedPath) return []
    return edges.filter((e) => e.to === selectedPath).map((e) => e.from)
  }, [edges, selectedPath])

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      {/* ── Header bar ─────────────────────────────────────────────── */}
      <div className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[12px] text-koma-dim">
        <Network size={13} className="flex-none opacity-70" />

        {/* Back button */}
        {breadcrumb.length > 1 && (
          <button
            type="button"
            onClick={popBreadcrumb}
            title="Go back"
            className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
          >
            ←
          </button>
        )}

        {/* Breadcrumb trail */}
        {breadcrumb.length > 0 && (
          <div className="flex min-w-0 items-center gap-0.5 overflow-hidden">
            {breadcrumb.map((path, idx) => {
              const isLast = idx === breadcrumb.length - 1
              const label = path.split('/').pop() ?? path
              return (
                <span key={`${path}-${idx}`} className="flex flex-none items-center gap-0.5">
                  {idx > 0 && <ChevronRight size={10} className="flex-none text-koma-dim opacity-40" />}
                  <button
                    type="button"
                    onClick={() => navigateBreadcrumb(idx)}
                    className={`max-w-[120px] truncate rounded px-1 py-0.5 text-[11px] ${
                      isLast
                        ? 'font-medium text-koma-accent'
                        : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
                    }`}
                    title={path}
                  >
                    {label}
                  </button>
                </span>
              )
            })}
          </div>
        )}

        {/* Search trigger — only show when no breadcrumb */}
        {breadcrumb.length === 0 && (
          <button
            type="button"
            onClick={() => {
              setShowOmni(true)
              setOmniQuery('')
              setOmniIdx(0)
            }}
            className="flex items-center gap-1.5 rounded border border-koma-border bg-koma-bg px-2 py-0.5 text-[11px] text-koma-dim hover:border-koma-accent/40 hover:text-koma-fg"
          >
            <Search size={11} />
            <span>Search files…</span>
            <kbd className="ml-1 rounded bg-koma-panel2 px-1 py-px text-[9px] opacity-60">⌘K</kbd>
          </button>
        )}

        {/* Depth selector */}
        <span className="text-[10px] text-koma-dim opacity-60">Depth:</span>
        {[1, 2, 3].map((d) => (
          <button
            key={d}
            type="button"
            onClick={() => setImportGraphDepth(d)}
            className={`rounded px-1.5 py-0.5 text-[11px] ${
              depth === d
                ? 'bg-koma-accent/20 text-koma-accent'
                : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
            }`}
          >
            {d}
          </button>
        ))}

        {/* Direction selector */}
        <span className="ml-1 text-[10px] text-koma-dim opacity-60">Dir:</span>
        {(['dependencies', 'dependents', 'both'] as const).map((d) => (
          <button
            key={d}
            type="button"
            onClick={() => setImportGraphDirection(d)}
            className={`rounded px-1.5 py-0.5 text-[10px] ${
              direction === d
                ? 'bg-koma-accent/20 text-koma-accent'
                : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
            }`}
          >
            {d === 'dependencies' ? 'deps' : d === 'dependents' ? 'rdeps' : 'both'}
          </button>
        ))}

        <span className="flex-1" />

        {/* Truncation warning */}
        {(nodesTruncated || edgesTruncated) && (
          <span className="flex items-center gap-1 text-[10px] text-koma-warn">
            <AlertTriangle size={11} />
            {nodesTruncated
              ? `Showing ${nodes.length} of ${totalNodesAvailable} nodes`
              : `Showing ${edges.length} of ${totalEdgesAvailable} edges`}
          </span>
        )}

        <span className="font-mono text-[11px]">
          {fileCount} files, {edgeCount} edges
        </span>

        {loading && <BrailleSpinner size={12} className="opacity-70" />}

        <button
          type="button"
          onClick={() => refreshImportGraph()}
          title="Refresh graph"
          aria-label="Refresh graph"
          className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <RefreshCw size={13} />
        </button>
        <span className="text-[9px] text-koma-dim opacity-40">gen {generation}</span>
      </div>

      {/* ── Main content ──────────────────────────────────────────── */}
      <div className="flex min-h-0 min-w-0 flex-1">
        {/* 3D Graph Canvas */}
        <div className="min-h-0 min-w-0 flex-1">
          {error ? (
            <div className="flex h-full w-full flex-col items-center justify-center gap-2 text-center text-[12px] text-koma-dim">
              <AlertTriangle size={24} className="text-koma-warn opacity-70" />
              <span className="max-w-xs">{error}</span>
              <span className="text-[10px] opacity-50">
                Start with koma --linker-daemon
              </span>
            </div>
          ) : nodes.length === 0 ? (
            <div className="flex h-full w-full flex-col items-center justify-center gap-2 text-center text-[12px] text-koma-dim">
              {loading ? (
                <BrailleSpinner size={18} className="opacity-70" />
              ) : (
                <>
                  <Network size={28} className="opacity-40" />
                  <span>
                    {fileCount === 0
                      ? 'No import graph data. Start the linker daemon.'
                      : 'Select a file to visualize.'}
                  </span>
                  {fileCount > 0 && (
                    <span className="text-[10px] opacity-50">
                      {fileCount} files, {edgeCount} edges · {languages.join(', ')}
                    </span>
                  )}
                </>
              )}
            </div>
          ) : (
            <ImportGraphScene
              nodes={nodes}
              edges={edges}
              focus={focus}
              selectedPath={selectedPath}
              onNodeClick={selectImportGraphNode}
              onNodeSelect={(path) => {
                useKoma.getState().clearImportGraphSelection()
                // Small delay then select for detail pane (no chain navigation)
                setTimeout(() => {
                  useKoma.setState((s) => ({
                    importGraph: { ...s.importGraph, selectedPath: path }
                  }))
                }, 0)
              }}
            />
          )}
        </div>

        {/* ── Detail pane (right) ──────────────────────────────────── */}
        {selectedNode && (
          <>
            <div
              onMouseDown={startDetailResize}
              className="w-[5px] flex-none cursor-ew-resize border-l border-koma-border hover:bg-koma-grip"
            />
            <div
              style={{ width: detailW }}
              className="flex min-h-0 min-w-0 flex-none flex-col overflow-y-auto border-l border-koma-border bg-koma-panel2"
            >
              <div className="flex flex-none items-center justify-between border-b border-koma-border px-3 py-1.5">
                <span className="text-[11px] font-medium uppercase tracking-wide text-koma-dim">
                  File Detail
                </span>
                <button
                  type="button"
                  onClick={clearImportGraphSelection}
                  title="Close detail"
                  aria-label="Close detail"
                  className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
                >
                  <X size={13} />
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2 text-[11px]">
                {/* Full path */}
                <div className="mb-2 break-all font-mono text-[10px] text-koma-dim">
                  {selectedNode.path}
                </div>

                {/* Meta */}
                <div className="mb-3 flex flex-wrap gap-2">
                  <span className="rounded bg-koma-bg px-1.5 py-0.5 text-[10px] text-koma-fg">
                    {selectedNode.language}
                  </span>
                  <span
                    className="rounded px-1.5 py-0.5 text-[10px]"
                    style={{
                      backgroundColor: (ROLE_COLORS[selectedNode.role] ?? ROLE_COLORS.Overview) + '30',
                      color: ROLE_COLORS[selectedNode.role] ?? ROLE_COLORS.Overview,
                    }}
                  >
                    {selectedNode.role}
                  </span>
                  <span className="text-[10px] text-koma-dim">
                    in: {selectedNode.inDegree} · out: {selectedNode.outDegree}
                  </span>
                </div>

                {/* Dependencies (outgoing) */}
                {directDeps.length > 0 && (
                  <div className="mb-3">
                    <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-koma-dim">
                      Dependencies ({directDeps.length})
                    </div>
                    <ul className="flex flex-col gap-0.5">
                      {directDeps.map((p) => (
                        <li key={p}>
                          <button
                            type="button"
                            onClick={() => selectImportGraphNode(p)}
                            className="w-full truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100"
                          >
                            {p.split('/').slice(-2).join('/')}
                          </button>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}

                {/* Dependents (incoming) */}
                {directDependents.length > 0 && (
                  <div className="mb-3">
                    <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-koma-dim">
                      Dependents ({directDependents.length})
                    </div>
                    <ul className="flex flex-col gap-0.5">
                      {directDependents.map((p) => (
                        <li key={p}>
                          <button
                            type="button"
                            onClick={() => selectImportGraphNode(p)}
                            className="w-full truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100"
                          >
                            {p.split('/').slice(-2).join('/')}
                          </button>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}

                {/* Impact button */}
                <button
                  type="button"
                  onClick={() => {
                    setImportGraphDepth(3)
                    setImportGraphDirection('both')
                    refreshImportGraph(selectedNode.path)
                  }}
                  className="mt-2 w-full rounded bg-koma-accent/15 px-2 py-1.5 text-[11px] font-medium text-koma-accent hover:bg-koma-accent/25"
                >
                  Impact Analysis (depth 3, both)
                </button>
              </div>
            </div>
          </>
        )}
      </div>

      {/* ── Omnisearch overlay (Cmd+K) ──────────────────────────────── */}
      {showOmni && (
        <div
          className="absolute inset-0 z-50 flex items-start justify-center bg-black/40 pt-[15vh]"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setShowOmni(false)
          }}
        >
          <div className="w-[520px] max-h-[60vh] flex flex-col overflow-hidden rounded-lg border border-koma-border bg-koma-panel shadow-2xl">
            {/* Search input */}
            <div className="flex items-center gap-2 border-b border-koma-border px-3 py-2">
              <Search size={14} className="flex-none text-koma-dim" />
              <input
                ref={omniInputRef}
                type="text"
                value={omniQuery}
                onChange={(e) => {
                  setOmniQuery(e.target.value)
                  setOmniIdx(0)
                }}
                onKeyDown={(e) => {
                  if (e.key === 'ArrowDown') {
                    e.preventDefault()
                    setOmniIdx((i) => Math.min(i + 1, omniResults.length - 1))
                  } else if (e.key === 'ArrowUp') {
                    e.preventDefault()
                    setOmniIdx((i) => Math.max(i - 1, 0))
                  } else if (e.key === 'Enter' && omniResults[omniIdx]) {
                    selectOmniResult(omniResults[omniIdx].path)
                  } else if (e.key === 'Escape') {
                    setShowOmni(false)
                  }
                }}
                placeholder="Search files by name or language…"
                className="flex-1 bg-transparent text-[13px] text-koma-fg placeholder:text-koma-dim/50 focus:outline-none"
              />
              <kbd className="flex-none rounded bg-koma-bg px-1 py-px text-[9px] text-koma-dim">ESC</kbd>
            </div>
            {/* Results */}
            <div className="min-h-0 flex-1 overflow-y-auto">
              {omniResults.length === 0 ? (
                <div className="px-3 py-4 text-center text-[12px] text-koma-dim opacity-60">
                  No matching files
                </div>
              ) : (
                <ul>
                  {omniResults.map((n, i) => {
                    const isGraphSelected = n.path === selectedPath
                    const isHighlighted = i === omniIdx
                    return (
                      <li key={n.path}>
                        <button
                          type="button"
                          onClick={() => selectOmniResult(n.path)}
                          onMouseEnter={() => setOmniIdx(i)}
                          className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] ${
                            isHighlighted
                              ? 'bg-koma-accent/15 text-koma-accent'
                              : isGraphSelected
                                ? 'bg-koma-accent/5 text-koma-accent'
                                : 'text-koma-fg hover:bg-koma-hover'
                          }`}
                        >
                          <span className="min-w-0 flex-1 truncate font-mono text-[11px]">{n.path}</span>
                          <span className="flex-none text-[9px] text-koma-dim opacity-60">{n.language}</span>
                          <span className="flex-none text-[9px] text-koma-dim opacity-40" title="dependents / dependencies">
                            {n.inDegree}↑ {n.outDegree}↓
                          </span>
                        </button>
                      </li>
                    )
                  })}
                </ul>
              )}
            </div>
            {/* Footer hint */}
            <div className="flex-none border-t border-koma-border px-3 py-1.5 text-[10px] text-koma-dim opacity-50">
              ↑↓ navigate · Enter select · Esc close
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

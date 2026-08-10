import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Network, RefreshCw, X, Maximize2, AlertTriangle } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import {
  computeImportGraphLayout,
  type LayoutNode,
  type LayoutEdge,
} from '../lib/importGraphLayout'

// Detail pane width constants (mirrors GraphTab).
const DETAIL_W_MIN = 220
const DETAIL_W_MAX = 480
const DETAIL_W_DEFAULT = 300

// Role → color mapping for nodes.
const ROLE_COLORS: Record<string, { fill: string; stroke: string; text: string }> = {
  Focus: { fill: '#3b82f6', stroke: '#2563eb', text: '#ffffff' },
  Dependency: { fill: '#22c55e', stroke: '#16a34a', text: '#ffffff' },
  Dependent: { fill: '#f97316', stroke: '#ea580c', text: '#ffffff' },
  Overview: { fill: '#6b7280', stroke: '#4b5563', text: '#ffffff' },
}

// SVG arrowhead marker definition.
function ArrowDefs() {
  return (
    <defs>
      <marker
        id="arrowhead"
        markerWidth="8"
        markerHeight="6"
        refX="8"
        refY="3"
        orient="auto"
      >
        <path d="M0,0 L8,3 L0,6" fill="#94a3b8" />
      </marker>
    </defs>
  )
}

// ─── Main ImportGraphTab ────────────────────────────────────────────────
export default function ImportGraphTab() {
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

  const refreshImportGraph = useKoma((s) => s.refreshImportGraph)
  const setImportGraphDepth = useKoma((s) => s.setImportGraphDepth)
  const setImportGraphDirection = useKoma((s) => s.setImportGraphDirection)
  const selectImportGraphNode = useKoma((s) => s.selectImportGraphNode)
  const clearImportGraphSelection = useKoma((s) => s.clearImportGraphSelection)

  const isActiveTab = useKoma((s) => s.ui.activeTabId === 'import-graph')
  const sessionId = useKoma((s) => s.session.id)

  // Fetch on mount and session change (mirrors GraphTab's useEffect).
  useEffect(() => {
    refreshImportGraph()
  }, [refreshImportGraph, sessionId])

  // ── Layout ────────────────────────────────────────────────────────
  const layout = useMemo(
    () => computeImportGraphLayout(nodes, edges, focus),
    [nodes, edges, focus],
  )

  // ── Pan / Zoom ────────────────────────────────────────────────────
  const canvasRef = useRef<HTMLDivElement | null>(null)
  const [offsetX, setOffsetX] = useState(0)
  const [offsetY, setOffsetY] = useState(0)
  const [scale, setScale] = useState(1)
  const dragRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null)

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault()
    setScale((prev) => {
      const delta = e.deltaY > 0 ? -0.1 : 0.1
      return Math.max(0.2, Math.min(3, prev + delta))
    })
  }, [])

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      // Only start pan on left-click on the background (not on nodes).
      if (e.button !== 0) return
      const target = e.target as HTMLElement
      if (target.closest('[data-graph-node]')) return
      dragRef.current = { startX: e.clientX, startY: e.clientY, origX: offsetX, origY: offsetY }
      document.body.style.cursor = 'grabbing'
      document.body.style.userSelect = 'none'
    },
    [offsetX, offsetY],
  )

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    if (!dragRef.current) return
    const dx = e.clientX - dragRef.current.startX
    const dy = e.clientY - dragRef.current.startY
    setOffsetX(dragRef.current.origX + dx)
    setOffsetY(dragRef.current.origY + dy)
  }, [])

  const handleMouseUp = useCallback(() => {
    if (dragRef.current) {
      dragRef.current = null
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
  }, [])

  const fitToContent = useCallback(() => {
    if (layout.width === 0 || layout.height === 0) return
    const el = canvasRef.current
    if (!el) return
    const vw = el.clientWidth
    const vh = el.clientHeight
    const sx = vw / layout.width
    const sy = vh / layout.height
    const s = Math.min(sx, sy, 1.5) * 0.9
    setScale(s)
    setOffsetX((vw - layout.width * s) / 2)
    setOffsetY((vh - layout.height * s) / 2)
  }, [layout])

  // Auto-fit on first data load.
  const prevNodeCount = useRef(0)
  useEffect(() => {
    if (nodes.length > 0 && prevNodeCount.current === 0) {
      requestAnimationFrame(fitToContent)
    }
    prevNodeCount.current = nodes.length
  }, [nodes.length, fitToContent])

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

  // ── Focus file search ─────────────────────────────────────────────
  const [focusQuery, setFocusQuery] = useState('')
  const focusNode = useMemo(() => nodes.find((n) => n.path === focus), [nodes, focus])
  const [showFocusDropdown, setShowFocusDropdown] = useState(false)
  const focusInputRef = useRef<HTMLInputElement | null>(null)

  const focusSuggestions = useMemo(() => {
    if (!focusQuery.trim()) return nodes.slice(0, 20)
    const q = focusQuery.toLowerCase()
    return nodes.filter((n) => n.path.toLowerCase().includes(q)).slice(0, 20)
  }, [nodes, focusQuery])

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

  // ── Node click ────────────────────────────────────────────────────
  const onNodeClick = useCallback(
    (path: string) => {
      selectImportGraphNode(path)
    },
    [selectImportGraphNode],
  )

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      {/* ── Header bar ─────────────────────────────────────────────── */}
      <div className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[12px] text-koma-dim">
        <Network size={13} className="flex-none opacity-70" />

        {/* Focus file picker */}
        <div className="relative">
          <input
            ref={focusInputRef}
            type="text"
            value={focusQuery || (focusNode ? focusNode.path.split('/').slice(-2).join('/') : '')}
            onChange={(e) => {
              setFocusQuery(e.target.value)
              setShowFocusDropdown(true)
            }}
            onFocus={() => setShowFocusDropdown(true)}
            onBlur={() => {
              // Delay so clicks on dropdown register.
              setTimeout(() => setShowFocusDropdown(false), 150)
            }}
            placeholder={focus ? '' : 'Select file to focus…'}
            className="w-48 rounded border border-koma-border bg-koma-bg px-2 py-0.5 text-[11px] text-koma-fg placeholder:text-koma-dim/50 focus:border-koma-accent focus:outline-none"
          />
          {showFocusDropdown && focusSuggestions.length > 0 && (
            <div className="absolute top-full left-0 z-30 mt-1 max-h-48 w-64 overflow-y-auto rounded border border-koma-border bg-koma-panel2 shadow-lg">
              {focusSuggestions.map((n) => (
                <button
                  key={n.path}
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault()
                    selectImportGraphNode(n.path)
                    setFocusQuery('')
                    setShowFocusDropdown(false)
                  }}
                  className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[11px] hover:bg-koma-hover ${
                    n.path === focus ? 'bg-koma-accent/10 text-koma-accent' : 'text-koma-fg'
                  }`}
                >
                  <span className="min-w-0 flex-1 truncate">{n.path}</span>
                  <span className="flex-none text-[9px] text-koma-dim">{n.language}</span>
                </button>
              ))}
            </div>
          )}
        </div>

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
          onClick={() => fitToContent()}
          title="Fit to content"
          aria-label="Fit to content"
          className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <Maximize2 size={12} />
        </button>
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
        {/* SVG Canvas */}
        <div
          ref={canvasRef}
          onWheel={handleWheel}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
          className="min-h-0 min-w-0 flex-1 overflow-hidden"
        >
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
            <svg
              width="100%"
              height="100%"
              style={{
                transform: `translate(${offsetX}px, ${offsetY}px) scale(${scale})`,
                transformOrigin: '0 0',
              }}
            >
              <ArrowDefs />

              {/* Edges */}
              {layout.edges.map((e, i) => {
                const pts = e.points
                if (pts.length < 2) return null
                const d = pts.map((p, j) => `${j === 0 ? 'M' : 'L'}${p.x},${p.y}`).join(' ')
                return (
                  <path
                    key={`${e.from}->${e.to}-${i}`}
                    d={d}
                    fill="none"
                    stroke="#94a3b8"
                    strokeWidth={1}
                    strokeOpacity={0.5}
                    markerEnd="url(#arrowhead)"
                  />
                )
              })}

              {/* Nodes */}
              {layout.nodes.map((n) => {
                const colors = ROLE_COLORS[n.role] ?? ROLE_COLORS.Overview
                const isSelected = n.id === selectedPath
                return (
                  <g
                    key={n.id}
                    data-graph-node
                    onClick={() => onNodeClick(n.id)}
                    style={{ cursor: 'pointer' }}
                  >
                    <rect
                      x={n.x}
                      y={n.y}
                      width={n.width}
                      height={n.height}
                      rx={4}
                      fill={colors.fill}
                      fillOpacity={isSelected ? 1 : 0.85}
                      stroke={isSelected ? '#ffffff' : colors.stroke}
                      strokeWidth={isSelected ? 2 : 1}
                    />
                    <text
                      x={n.x + 8}
                      y={n.y + NODE_H_CENTER}
                      fill={colors.text}
                      fontSize={10}
                      dominantBaseline="central"
                      fontFamily="monospace"
                    >
                      {n.label.length > 20 ? n.label.slice(0, 19) + '…' : n.label}
                    </text>
                    {/* Language badge */}
                    <text
                      x={n.x + n.width - 6}
                      y={n.y + NODE_H_CENTER}
                      fill={colors.text}
                      fontSize={8}
                      textAnchor="end"
                      dominantBaseline="central"
                      opacity={0.6}
                    >
                      {n.language}
                    </text>
                  </g>
                )
              })}
            </svg>
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
                      backgroundColor: ROLE_COLORS[selectedNode.role]?.fill + '30',
                      color: ROLE_COLORS[selectedNode.role]?.fill,
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
    </div>
  )
}

// Center y for 32px-high nodes (must match NODE_H in importGraphLayout.ts).
const NODE_H_CENTER = 16

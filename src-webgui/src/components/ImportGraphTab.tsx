import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  Network,
  RefreshCw,
  ChevronRight,
  ChevronDown,
  FolderClosed,
  FolderOpen,
  FileText,
  X,
  AlertTriangle,
} from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

// Role → color mapping.
const ROLE_COLORS: Record<string, string> = {
  Focus: '#3b82f6',
  Dependency: '#22c55e',
  Dependent: '#f97316',
  Overview: '#6b7280',
}

// ── Tree node (directory or file) ──────────────────────────────────────────
type TreeNodeData = {
  name: string
  fullPath: string
  isDir: boolean
  language?: string
  inDegree?: number
  outDegree?: number
  role?: string
  children: TreeNodeData[]
}

// Build a directory tree from the flat nodes list.
function buildTree(nodes: { path: string; language: string; inDegree: number; outDegree: number; role: string }[]): TreeNodeData {
  const root: TreeNodeData = { name: '', fullPath: '', isDir: true, children: [] }

  for (const node of nodes) {
    const parts = node.path.split('/')
    let current = root

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]
      const isLast = i === parts.length - 1
      const isDir = !isLast
      const fullPath = parts.slice(0, i + 1).join('/')

      let child = current.children.find((c) => c.name === part && c.isDir === isDir)
      if (!child) {
        child = isLast
          ? { name: part, fullPath: node.path, isDir: false, language: node.language, inDegree: node.inDegree, outDegree: node.outDegree, role: node.role, children: [] }
          : { name: part, fullPath, isDir: true, children: [] }
        current.children.push(child)
      }
      current = child
    }
  }

  // Sort children: dirs first, then alphabetical.
  function sortTree(node: TreeNodeData) {
    node.children.sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
      return a.name.localeCompare(b.name)
    })
    for (const child of node.children) {
      if (child.isDir) sortTree(child)
    }
  }
  sortTree(root)

  return root
}

// ── TreeView component ─────────────────────────────────────────────────────
function TreeView({
  node,
  depth,
  expanded,
  onToggleDir,
  onSelectFile,
  selectedPath,
  edgeFrom,
  edgeTo,
}: {
  node: TreeNodeData
  depth: number
  expanded: Set<string>
  onToggleDir: (path: string) => void
  onSelectFile: (path: string) => void
  selectedPath: string | null
  edgeFrom: Set<string> | null
  edgeTo: Set<string> | null
}) {
  if (node.isDir) {
    const isOpen = expanded.has(node.fullPath)
    return (
      <div>
        <button
          type="button"
          onClick={() => onToggleDir(node.fullPath)}
          className="flex w-full items-center gap-1 px-1 py-0.5 text-[11px] text-koma-fg/80 hover:bg-koma-hover"
          style={{ paddingLeft: depth * 12 }}
        >
          {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          {isOpen ? <FolderOpen size={12} className="text-koma-accent opacity-70" /> : <FolderClosed size={12} className="opacity-60" />}
          <span className="truncate">{node.name}</span>
        </button>
        {isOpen &&
          node.children.map((child) => (
            <TreeView
              key={child.fullPath}
              node={child}
              depth={depth + 1}
              expanded={expanded}
              onToggleDir={onToggleDir}
              onSelectFile={onSelectFile}
              selectedPath={selectedPath}
              edgeFrom={edgeFrom}
              edgeTo={edgeTo}
            />
          ))}
      </div>
    )
  }

  // File node
  const isSelected = node.fullPath === selectedPath
  const hasDeps = edgeFrom?.has(node.fullPath)
  const hasDependents = edgeTo?.has(node.fullPath)
  const roleColor = ROLE_COLORS[node.role ?? 'Overview'] ?? ROLE_COLORS.Overview

  return (
    <button
      type="button"
      onClick={() => onSelectFile(node.fullPath)}
      className={`flex w-full items-center gap-1 px-1 py-0.5 text-[11px] hover:bg-koma-hover ${
        isSelected ? 'bg-koma-accent/15 text-koma-accent' : 'text-koma-fg/80'
      }`}
      style={{ paddingLeft: depth * 12 }}
    >
      <span className="flex-none w-[12px]" />
      <FileText size={12} className="flex-none opacity-60" />
      <span className="truncate flex-1 text-left">{node.name}</span>
      {/* Language badge */}
      {node.language && (
        <span className="flex-none text-[9px] text-koma-dim">{node.language}</span>
      )}
      {/* Deps/dependents indicators */}
      {hasDeps && (
        <span
          className="flex-none rounded px-0.5 text-[8px] font-bold"
          style={{ color: '#22c55e' }}
          title={`${node.outDegree} deps`}
        >
          ↓{node.outDegree}
        </span>
      )}
      {hasDependents && (
        <span
          className="flex-none rounded px-0.5 text-[8px] font-bold"
          style={{ color: '#f97316' }}
          title={`${node.inDegree} dependents`}
        >
          ↑{node.inDegree}
        </span>
      )}
      {/* Role dot */}
      {node.role && node.role !== 'Overview' && (
        <span
          className="flex-none h-[6px] w-[6px] rounded-full"
          style={{ backgroundColor: roleColor }}
          title={node.role}
        />
      )}
    </button>
  )
}

// ── Main ImportGraphTab ────────────────────────────────────────────────────
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

  const sessionId = useKoma((s) => s.session.id)

  // Fetch on mount and session change.
  useEffect(() => {
    refreshImportGraph()
  }, [refreshImportGraph, sessionId])

  // ── Build tree ──────────────────────────────────────────────────────────
  const tree = useMemo(() => buildTree(nodes), [nodes])

  // Edge lookup sets for indicator badges.
  const edgeFrom = useMemo(() => {
    const s = new Set<string>()
    for (const e of edges) s.add(e.from)
    return s
  }, [edges])
  const edgeTo = useMemo(() => {
    const s = new Set<string>()
    for (const e of edges) s.add(e.to)
    return s
  }, [edges])

  // ── Expand/collapse ─────────────────────────────────────────────────────
  const [expanded, setExpanded] = useState<Set<string>>(new Set())

  // Auto-expand on first load.
  const prevNodeCount = useMemo(() => nodes.length, []) // only initial
  const [initialized, setInitialized] = useState(false)
  useEffect(() => {
    if (!initialized && nodes.length > 0) {
      // Expand root-level directories.
      const newExpanded = new Set<string>()
      const topLevel = tree.children
      for (const child of topLevel) {
        if (child.isDir) newExpanded.add(child.fullPath)
      }
      setExpanded(newExpanded)
      setInitialized(true)
    }
  }, [nodes.length, tree, initialized])

  // Auto-expand path to selected file.
  useEffect(() => {
    if (selectedPath) {
      setExpanded((prev) => {
        const next = new Set(prev)
        const parts = selectedPath.split('/')
        for (let i = 1; i < parts.length; i++) {
          next.add(parts.slice(0, i).join('/'))
        }
        return next
      })
    }
  }, [selectedPath])

  const onToggleDir = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  const onSelectFile = useCallback(
    (path: string) => {
      selectImportGraphNode(path)
    },
    [selectImportGraphNode],
  )

  // ── Selected file detail ────────────────────────────────────────────────
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
        {/* File Tree */}
        <div className="flex-1 min-h-0 overflow-y-auto">
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
            tree.children.map((child) => (
              <TreeView
                key={child.fullPath}
                node={child}
                depth={0}
                expanded={expanded}
                onToggleDir={onToggleDir}
                onSelectFile={onSelectFile}
                selectedPath={selectedPath}
                edgeFrom={edgeFrom}
                edgeTo={edgeTo}
              />
            ))
          )}
        </div>

        {/* ── Detail pane (right) ──────────────────────────────────── */}
        {selectedNode && (
          <div
            className="flex min-h-0 min-w-0 flex-none flex-col overflow-y-auto border-l border-koma-border bg-koma-panel2"
            style={{ width: 300 }}
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

              {/* Meta badges */}
              <div className="mb-3 flex flex-wrap gap-2">
                <span className="rounded bg-koma-bg px-1.5 py-0.5 text-[10px] text-koma-fg">
                  {selectedNode.language}
                </span>
                <span
                  className="rounded px-1.5 py-0.5 text-[10px]"
                  style={{
                    backgroundColor: (ROLE_COLORS[selectedNode.role] ?? '#6b7280') + '30',
                    color: ROLE_COLORS[selectedNode.role] ?? '#6b7280',
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
                          className="flex w-full items-center gap-1 truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100"
                        >
                          <FileText size={10} className="flex-none opacity-50" />
                          <span className="truncate">{p.split('/').slice(-2).join('/')}</span>
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
                          className="flex w-full items-center gap-1 truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100"
                        >
                          <FileText size={10} className="flex-none opacity-50" />
                          <span className="truncate">{p.split('/').slice(-2).join('/')}</span>
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
        )}
      </div>
    </div>
  )
}

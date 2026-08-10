import { useState, useMemo, useEffect } from 'react'
import { Network, ChevronRight, FolderOpen, FileCode } from 'lucide-react'
import { useKoma } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'
import type { ImportGraphNode } from '../../store/koma'

// ── Folder-tree types ────────────────────────────────────────────────────────

type FolderGroup = {
  path: string // directory path
  name: string // short folder name
  files: ImportGraphNode[]
  children: FolderGroup[]
}

// ── Tree helpers ─────────────────────────────────────────────────────────────

function findFolder(groups: FolderGroup[], path: string): FolderGroup | null {
  for (const g of groups) {
    if (g.path === path) return g
    const found = findFolder(g.children, path)
    if (found) return found
  }
  return null
}

function countFiles(group: FolderGroup): number {
  return (
    group.files.length +
    group.children.reduce((sum, c) => sum + countFiles(c), 0)
  )
}

function buildFolderTree(nodes: ImportGraphNode[]): FolderGroup[] {
  const sorted = [...nodes].sort((a, b) => a.path.localeCompare(b.path))
  const root: FolderGroup[] = []

  for (const node of sorted) {
    const parts = node.path.split('/')
    const fileParentPath = parts.slice(0, -1).join('/')

    // Walk folder parts, creating intermediates as needed
    for (let i = 0; i < parts.length - 1; i++) {
      const folderName = parts[i]
      const parentGroups = i === 0 ? root : findFolder(root, parts.slice(0, i).join('/'))?.children
      if (!parentGroups) continue

      let folder = parentGroups.find((f) => f.name === folderName)
      if (!folder) {
        folder = {
          path: parts.slice(0, i + 1).join('/'),
          name: folderName,
          files: [],
          children: [],
        }
        parentGroups.push(folder)
      }
    }

    // Add file to its parent folder (or an implicit root group)
    const fileParent = findFolder(root, fileParentPath)
    if (fileParent) {
      fileParent.files.push(node)
    } else {
      let rootGroup = root.find((f) => f.path === '')
      if (!rootGroup) {
        rootGroup = { path: '', name: '.', files: [], children: [] }
        root.push(rootGroup)
      }
      rootGroup.files.push(node)
    }
  }

  return root
}

// ── Recursive folder node ────────────────────────────────────────────────────

function FolderNode({
  group,
  depth,
  onFileClick,
  selectedPath,
  breadcrumb,
}: {
  group: FolderGroup
  depth: number
  onFileClick: (path: string) => void
  selectedPath: string | null
  breadcrumb: string[]
}) {
  const [expanded, setExpanded] = useState(depth < 1)

  // Auto-expand folders on the breadcrumb chain
  const isInBreadcrumb = breadcrumb.some((b) => b.startsWith(group.path + '/') || b === group.path)
  const effectiveExpanded = isInBreadcrumb || expanded

  return (
    <div>
      {group.path !== '' && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex w-full items-center gap-1 py-0.5 text-[11px] text-koma-dim hover:bg-koma-hover"
          style={{ paddingLeft: 8 + depth * 12 }}
        >
          <ChevronRight
            size={10}
            className={`flex-none transition-transform ${effectiveExpanded ? 'rotate-90' : ''}`}
          />
          <FolderOpen size={12} className="flex-none opacity-50" />
          <span className="truncate">{group.name}</span>
          <span className="ml-auto text-[9px] opacity-40">{countFiles(group)}</span>
        </button>
      )}
      {effectiveExpanded && (
        <>
          {group.children.map((child) => (
            <FolderNode
              key={child.path}
              group={child}
              depth={depth + 1}
              onFileClick={onFileClick}
              selectedPath={selectedPath}
              breadcrumb={breadcrumb}
            />
          ))}
          {group.files.map((f) => (
            <button
              key={f.path}
              onClick={() => onFileClick(f.path)}
              className={`flex w-full items-center gap-1.5 py-0.5 text-[11px] hover:bg-koma-hover ${
                f.path === selectedPath
                  ? 'bg-koma-accent/10 text-koma-accent'
                  : 'text-koma-fg'
              }`}
              style={{ paddingLeft: group.path === '' ? 20 + depth * 12 : 20 + (depth + 1) * 12 }}
              title={f.path}
            >
              <FileCode size={12} className="flex-none opacity-50" />
              <span className="min-w-0 flex-1 truncate">{f.path.split('/').pop()}</span>
              <span className="flex-none text-[9px] text-koma-dim opacity-50">{f.language}</span>
            </button>
          ))}
        </>
      )}
    </div>
  )
}

// ── Panel ────────────────────────────────────────────────────────────────────

export function ImportGraphPanel() {
  const nodes = useKoma((s) => s.importGraph.nodes)
  const loading = useKoma((s) => s.importGraph.loading)
  const error = useKoma((s) => s.importGraph.error)
  const fileCount = useKoma((s) => s.importGraph.fileCount)
  const edgeCount = useKoma((s) => s.importGraph.edgeCount)
  const languages = useKoma((s) => s.importGraph.languages)
  const selectedPath = useKoma((s) => s.importGraph.selectedPath)
  const breadcrumb = useKoma((s) => s.importGraph.breadcrumb)
  const openImportGraphTab = useKoma((s) => s.openImportGraphTab)
  const selectImportGraphNode = useKoma((s) => s.selectImportGraphNode)
  const navigateBreadcrumb = useKoma((s) => s.navigateBreadcrumb)
  const refreshImportGraph = useKoma((s) => s.refreshImportGraph)
  const [query, setQuery] = useState('')

  useEffect(() => {
    refreshImportGraph()
  }, [refreshImportGraph])

  const filtered = useMemo(() => {
    if (!query.trim()) return nodes
    const q = query.toLowerCase()
    return nodes.filter((n) => n.path.toLowerCase().includes(q))
  }, [nodes, query])

  const folderTree = useMemo(() => buildFolderTree(filtered), [filtered])

  const handleFileClick = (path: string) => {
    openImportGraphTab()
    selectImportGraphNode(path)
  }

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* Open Graph button */}
        <button
          onClick={() => openImportGraphTab()}
          className="flex items-center gap-2 px-3 py-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
        >
          <Network size={14} className="flex-none opacity-80" />
          <span>Open Import Graph</span>
        </button>

        {/* Breadcrumb chain */}
        {breadcrumb.length > 0 && (
          <div className="border-b border-koma-border px-3 py-1.5">
            <div className="mb-1 text-[10px] text-koma-dim opacity-60">EXPLORATION CHAIN</div>
            <div className="flex flex-wrap gap-1">
              {breadcrumb.map((path, idx) => (
                <button
                  key={idx}
                  onClick={() => navigateBreadcrumb(idx)}
                  className={`rounded px-1.5 py-0.5 text-[10px] ${
                    idx === breadcrumb.length - 1
                      ? 'bg-koma-accent/20 font-medium text-koma-accent'
                      : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
                  }`}
                  title={path}
                >
                  {path.split('/').pop()}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Summary */}
        {fileCount > 0 && (
          <div className="px-3 py-1.5 text-[11px] text-koma-dim">
            <span className="font-mono">{fileCount}</span> files,{' '}
            <span className="font-mono">{edgeCount}</span> edges
            {languages.length > 0 && (
              <span className="ml-1 opacity-60">({languages.join(', ')})</span>
            )}
          </div>
        )}

        {/* Search */}
        {nodes.length > 0 && (
          <div className="px-3 pb-1">
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter files…"
              className="w-full rounded border border-koma-border bg-koma-bg px-2 py-1 text-[11px] text-koma-fg placeholder:text-koma-dim/50 focus:border-koma-accent focus:outline-none"
            />
          </div>
        )}

        {/* Files heading */}
        <div className="px-3 pb-1 pt-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
          Files{fileCount > 0 ? ` (${filtered.length})` : ''}
        </div>

        {/* Content */}
        {loading && nodes.length === 0 ? (
          <div className="flex items-center justify-center px-3 py-4">
            <BrailleSpinner size={14} className="opacity-70" />
          </div>
        ) : error ? (
          <div className="px-3 py-2 text-[11px] text-koma-error">{error}</div>
        ) : nodes.length === 0 ? (
          <div className="px-3 py-2 text-[11px] text-koma-dim opacity-70">
            No import graph data. Start the linker daemon.
          </div>
        ) : (
          <div className="flex flex-col">
            {folderTree.map((group) => (
              <FolderNode
                key={group.path}
                group={group}
                depth={0}
                onFileClick={handleFileClick}
                selectedPath={selectedPath}
                breadcrumb={breadcrumb}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

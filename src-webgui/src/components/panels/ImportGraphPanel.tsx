import { useState, useMemo, useEffect } from 'react'
import { Network, FolderOpen, RefreshCw } from 'lucide-react'
import { useKoma } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'

// Sidebar panel for the Import Graph feature. Compact summary + searchable
// file list + "Open Graph" launcher. Content lives in the `importGraph`
// store slice; the panel fires refreshImportGraph on mount so the count
// is fresh without opening the tab.
export function ImportGraphPanel() {
  const nodes = useKoma((s) => s.importGraph.nodes)
  const loading = useKoma((s) => s.importGraph.loading)
  const error = useKoma((s) => s.importGraph.error)
  const fileCount = useKoma((s) => s.importGraph.fileCount)
  const edgeCount = useKoma((s) => s.importGraph.edgeCount)
  const languages = useKoma((s) => s.importGraph.languages)
  const openImportGraphTab = useKoma((s) => s.openImportGraphTab)
  const selectImportGraphNode = useKoma((s) => s.selectImportGraphNode)
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

        {/* File list */}
        <div className="mt-1 px-3 pb-1 pt-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
          Files{fileCount > 0 ? ` (${filtered.length})` : ''}
        </div>

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
          <ul className="flex flex-col">
            {filtered.map((n) => (
              <li key={n.path}>
                <button
                  onClick={() => {
                    openImportGraphTab()
                    selectImportGraphNode(n.path)
                  }}
                  title={n.path}
                  className="flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-koma-hover"
                >
                  <FolderOpen size={12} className="flex-none opacity-50" />
                  <span className="min-w-0 flex-1 truncate text-[11px] text-koma-fg">
                    {n.path.split('/').slice(-2).join('/')}
                  </span>
                  <span className="flex-none text-[9px] text-koma-dim opacity-50">
                    {n.language}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}

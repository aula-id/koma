import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Network, RefreshCw, X, AlertTriangle, Search, ChevronRight } from 'lucide-react'
import { useKoma, type ImportGraphNode } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { ImportGraphFlow } from './ImportGraphFlow'
import { sourceLanguage } from '../lib/importGraphLanguages'

// Detail pane width constants.
const DETAIL_W_MIN = 260
const DETAIL_W_MAX = 500
const DETAIL_W_DEFAULT = 320

// Role → color mapping for detail pane badges.
const ROLE_COLORS: Record<string, string> = {
  Focus: '#3b82f6',
  Dependency: '#6ba3b0',
  Dependent: '#b09070',
  Overview: '#8896a4',
}

// Robust basename helper — handles both / and \ separators.
function baseName(p: string): string {
  const parts = p.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || p
}

function parentDir(p: string): string {
  const parts = p.replace(/\\/g, '/').split('/')
  if (parts.length <= 2) return ''
  return parts.slice(-2, -1).join('')
}

// ─── Compact filter dropdown ───────────────────────────────────────────────

function FilterDropdown<T extends string>({
  label,
  allLabel,
  countLabel,
  oneLabel,
  items,
  selected,
  onChange,
}: {
  label: string
  allLabel: string
  countLabel: string
  oneLabel?: string
  items: { key: T; label: string; count?: number; sublabel?: string }[]
  selected: T[]
  onChange: (selected: T[]) => void
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open])

  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && ref.current?.contains(document.activeElement)) {
        setOpen(false)
      }
    }
    document.addEventListener('keydown', handler, true)
    return () => document.removeEventListener('keydown', handler, true)
  }, [open])

  const isAll = selected.length === 0
  const buttonLabel = isAll
    ? allLabel
    : selected.length === 1
      ? (oneLabel ?? items.find((i) => i.key === selected[0])?.label ?? selected[0])
      : countLabel.replace('{n}', String(selected.length))

  return (
    <div ref={ref} className="relative flex-none">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] whitespace-nowrap ${
          isAll
            ? 'border-koma-border text-koma-dim hover:border-koma-accent/40 hover:text-koma-fg'
            : 'border-koma-accent/40 bg-koma-accent/10 text-koma-accent'
        }`}
        aria-label={label}
        aria-expanded={open}
      >
        <span className="opacity-60">{label}:</span>
        <span className="font-medium">{buttonLabel}</span>
      </button>
      {open && (
        <div
          role="menu"
          className="absolute left-0 top-full z-50 mt-1 min-w-[200px] max-h-[280px] overflow-y-auto rounded border border-koma-border bg-koma-panel shadow-lg"
        >
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={isAll}
            onClick={() => { onChange([]); setOpen(false) }}
            className={`flex w-full items-center justify-between px-2 py-1 text-[11px] ${
              isAll ? 'bg-koma-accent/10 text-koma-accent' : 'text-koma-fg hover:bg-koma-hover'
            }`}
          >
            <span>All</span>
            {isAll && <span className="text-[9px] opacity-50">✓</span>}
          </button>
          <div className="border-t border-koma-border" />
          {items.map((item) => {
            const checked = selected.includes(item.key)
            return (
              <button
                key={item.key}
                type="button"
                role="menuitemcheckbox"
                aria-checked={checked}
                onClick={() => {
                  onChange(checked ? selected.filter((k) => k !== item.key) : [...selected, item.key])
                }}
                className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[11px] ${
                  checked ? 'bg-koma-accent/10 text-koma-accent' : 'text-koma-fg hover:bg-koma-hover'
                }`}
              >
                <span className="w-3 flex-none text-center text-[10px]">{checked ? '✓' : ''}</span>
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
                {item.sublabel && (
                  <span className="flex-none truncate text-[9px] text-koma-dim opacity-50 max-w-[120px]" title={item.sublabel}>
                    {item.sublabel}
                  </span>
                )}
                {item.count !== undefined && (
                  <span className="flex-none text-[9px] text-koma-dim opacity-60">{item.count}</span>
                )}
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
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
  const selectedPath = useKoma((s) => s.importGraph.selectedPath)
  const generation = useKoma((s) => s.importGraph.generation)
  const breadcrumb = useKoma((s) => s.importGraph.breadcrumb)
  const availableRoots = useKoma((s) => s.importGraph.availableRoots)
  const filterRoots = useKoma((s) => s.importGraph.filterRoots)
  const filterLanguages = useKoma((s) => s.importGraph.filterLanguages)

  // Impact selectors
  const impactStatus = useKoma((s) => s.importGraph.impactStatus)
  const impactPaths = useKoma((s) => s.importGraph.impactPaths)
  const impactTotal = useKoma((s) => s.importGraph.impactTotal)
  const impactError = useKoma((s) => s.importGraph.impactError)
  const impactPath = useKoma((s) => s.importGraph.impactPath)

  // FileSearch selectors
  const searchResults = useKoma((s) => s.session.searchResults)
  const req = useKoma((s) => s.req)

  const refreshImportGraph = useKoma((s) => s.refreshImportGraph)
  const selectImportGraphNode = useKoma((s) => s.selectImportGraphNode)
  const clearImportGraphSelection = useKoma((s) => s.clearImportGraphSelection)
  const navigateBreadcrumb = useKoma((s) => s.navigateBreadcrumb)
  const popBreadcrumb = useKoma((s) => s.popBreadcrumb)
  const setImportGraphRootFilter = useKoma((s) => s.setImportGraphRootFilter)
  const setImportGraphLanguageFilter = useKoma((s) => s.setImportGraphLanguageFilter)
  const requestImportGraphImpact = useKoma((s) => s.requestImportGraphImpact)

  const isActiveTab = useKoma((s) => s.ui.activeTabId === 'import-graph')
  const sessionId = useKoma((s) => s.session.id)

  const availableLanguages = useMemo(() => {
    const langCounts = new Map<string, number>()
    const rootsToCount = filterRoots.length > 0
      ? availableRoots.filter((r) => filterRoots.includes(r.root))
      : availableRoots
    for (const root of rootsToCount) {
      for (const lc of root.languages) {
        langCounts.set(lc.name, (langCounts.get(lc.name) ?? 0) + lc.count)
      }
    }
    return [...langCounts.entries()]
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([name, count]) => ({ name, count }))
  }, [availableRoots, filterRoots])

  const rootItems = useMemo(
    () => availableRoots.map((r) => {
      const base = r.root.split('/').pop() ?? r.root
      return { key: r.root, label: base, count: r.fileCount, sublabel: r.root }
    }),
    [availableRoots],
  )
  const langItems = useMemo(
    () => availableLanguages.map((l) => ({ key: l.name, label: l.name, count: l.count })),
    [availableLanguages],
  )

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

  // ── Cmd+K omnisearch — uses FileSearch from the session ─────────
  const [showOmni, setShowOmni] = useState(false)
  const [omniQuery, setOmniQuery] = useState('')
  const [omniIdx, setOmniIdx] = useState(0)
  const omniInputRef = useRef<HTMLInputElement | null>(null)
  const omniDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)

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

  useEffect(() => {
    if (showOmni) {
      requestAnimationFrame(() => omniInputRef.current?.focus())
    }
  }, [showOmni])

  // Debounced FileSearch request while omni is open.
  useEffect(() => {
    if (!showOmni) return
    if (omniDebounceRef.current) clearTimeout(omniDebounceRef.current)
    omniDebounceRef.current = setTimeout(() => {
      req({ r: 'FileSearch', query: omniQuery })
    }, 150)
    return () => {
      if (omniDebounceRef.current) clearTimeout(omniDebounceRef.current)
    }
  }, [omniQuery, showOmni, req])

  // Transform search results: separate dirs (path=='') from files.
  // Only show files with a supported source language.
  const omniResults = useMemo(() => {
    const dirs: { label: string; query: string }[] = []
    const files: { path: string; label: string; language: string }[] = []
    for (const r of searchResults) {
      if (r.path === '') {
        // Directory drill row — label is the directory name, clicking drills into it.
        dirs.push({ label: r.label, query: r.label })
      } else {
        const lang = sourceLanguage(r.path)
        if (lang) {
          files.push({ path: r.path, label: r.label, language: lang })
        }
      }
    }
    // Prioritize dirs first, then files.
    return [...dirs, ...files].slice(0, 50)
  }, [searchResults])

  const selectOmniResult = useCallback(
    (result: typeof omniResults[number]) => {
      if ('query' in result) {
        // Dir drill: set query, keep open
        setOmniQuery(result.query)
        setOmniIdx(0)
      } else {
        // File: select and close
        selectImportGraphNode(result.path)
        setShowOmni(false)
        setOmniQuery('')
      }
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

  // ── Flow callbacks ────────────────────────────────────────────────
  const handleFlowNodeClick = useCallback(
    (path: string) => {
      // Single click: select for detail pane
      useKoma.setState((s) => ({
        importGraph: { ...s.importGraph, selectedPath: path },
      }))
    },
    [],
  )

  const handleFlowNodeDoubleClick = useCallback(
    (path: string) => {
      // Double click: refocus
      selectImportGraphNode(path)
    },
    [selectImportGraphNode],
  )

  // ── Format stats badge ────────────────────────────────────────────
  const statsText = useMemo(() => {
    const fc = fileCount.toLocaleString()
    const ec = edgeCount.toLocaleString()
    return `${fc} files · ${ec} edges`
  }, [fileCount, edgeCount])

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      {/* ── Header bar (grouped + responsive) ────────────────────── */}
      <div className="flex flex-wrap items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[12px] text-koma-dim">
        {/* Navigation group */}
        <div className="flex min-w-0 flex-none items-center gap-1">
          <Network size={13} className="flex-none opacity-70" />

          {/* Back button */}
          {breadcrumb.length > 0 && (
            <button
              type="button"
              onClick={popBreadcrumb}
              title={breadcrumb.length === 1 ? 'Back to overview' : 'Go back'}
              aria-label={breadcrumb.length === 1 ? 'Back to overview' : 'Go back'}
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
                const label = baseName(path)
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
        </div>

        {/* Filters group */}
        <div className="flex flex-none items-center gap-1.5">
          {rootItems.length > 1 && (
            <FilterDropdown
              label="Root"
              allLabel="All roots"
              countLabel="{n} roots"
              items={rootItems}
              selected={filterRoots}
              onChange={setImportGraphRootFilter}
            />
          )}
          {langItems.length > 1 && (
            <FilterDropdown
              label="Lang"
              allLabel="All languages"
              countLabel="{n} languages"
              items={langItems}
              selected={filterLanguages}
              onChange={setImportGraphLanguageFilter}
            />
          )}
        </div>

        <span className="flex-1" />

        {/* Status/actions group */}
        <div className="flex flex-none items-center gap-2 whitespace-nowrap">
          {/* Truncation warning — says nodes/edges clearly */}
          {(nodesTruncated || edgesTruncated) && (
            <span className="flex items-center gap-1 text-[10px] whitespace-nowrap text-koma-warn">
              <AlertTriangle size={11} />
              {nodesTruncated
                ? `${nodes.length} of ${totalNodesAvailable} nodes`
                : `${edges.length} of ${totalEdgesAvailable} edges`}
            </span>
          )}

          {/* Stats badge */}
          <span className="whitespace-nowrap rounded bg-koma-bg/50 border border-koma-border/30 px-1.5 py-0.5 font-mono text-[11px]">
            {statsText}
          </span>

          {loading && <BrailleSpinner size={12} className="opacity-70" />}

          <button
            type="button"
            onClick={() => refreshImportGraph()}
            title={`Refresh graph (gen ${generation})`}
            aria-label={`Refresh graph (gen ${generation})`}
            className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
          >
            <RefreshCw size={13} />
          </button>
        </div>
      </div>

      {/* ── Main content ──────────────────────────────────────────── */}
      <div className="flex min-h-0 min-w-0 flex-1">
        {/* Graph Canvas */}
        <div className="flex min-h-0 min-w-[280px] flex-1">
          {error ? (
            <div className="flex h-full w-full flex-col items-center justify-center gap-3 text-center text-[12px] text-koma-dim">
              <AlertTriangle size={24} className="text-koma-warn opacity-70" />
              <span className="max-w-xs">{error}</span>
              <button
                type="button"
                onClick={() => refreshImportGraph(null)}
                className="flex items-center gap-1.5 rounded border border-koma-border bg-koma-panel px-2.5 py-1.5 text-[11px] text-koma-fg hover:border-koma-accent/40 hover:bg-koma-hover"
              >
                <RefreshCw size={12} />
                Retry graph
              </button>
            </div>
          ) : nodes.length === 0 ? (
            <div className="flex h-full w-full flex-col items-center justify-center gap-3 text-center text-[12px] text-koma-dim">
              {loading ? (
                <>
                  <BrailleSpinner size={18} className="opacity-70" />
                  <span className="text-[11px]">Loading neighborhood…</span>
                </>
              ) : (
                <>
                  <Network size={28} className="opacity-40" />
                  {focus ? (
                    <span>No neighborhood data for this file.</span>
                  ) : (
                    <>
                      <span>Select or search a file to visualize its import neighborhood.</span>
                      <span className="text-[10px] opacity-50">
                        {fileCount} files · {edgeCount} edges · {languages.join(', ')}
                      </span>
                    </>
                  )}
                </>
              )}
            </div>
          ) : focus ? (
            <ImportGraphFlow
              nodes={nodes}
              edges={edges}
              focus={focus}
              selectedPath={selectedPath}
              onNodeClick={handleFlowNodeClick}
              onNodeDoubleClick={handleFlowNodeDoubleClick}
            />
          ) : (
            <div className="flex h-full w-full flex-col items-center justify-center gap-3 text-center text-[12px] text-koma-dim">
              <Network size={28} className="opacity-40" />
              <span>Select or search a file to visualize its import neighborhood.</span>
              <span className="text-[10px] opacity-50">
                {fileCount} files · {edgeCount} edges · {languages.join(', ')}
              </span>
            </div>
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
              className="flex min-h-0 min-w-0 flex-none flex-col overflow-hidden border-l border-koma-border bg-koma-panel2"
            >
              {/* Header */}
              <div className="flex flex-none items-center justify-between border-b border-koma-border px-3 py-1.5">
                <span className="text-[11px] font-medium uppercase tracking-wide text-koma-dim">
                  Detail
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

              {/* Content — scrollable */}
              <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2 text-[11px]">
                {/* Filename dominant, full path selectable, Windows-safe */}
                <div className="mb-1 truncate font-medium text-[13px]" style={{ color: ROLE_COLORS[selectedNode.role] ?? '#8896a4' }}>
                  {baseName(selectedNode.path)}
                </div>
                <div className="mb-2 min-w-0 break-all font-mono text-[10px] text-koma-dim select-all" title="Click to select full path">
                  {selectedNode.path}
                </div>

                {/* Metadata chips — whitespace-nowrap per chip */}
                <div className="mb-3 flex flex-wrap gap-1.5">
                  <span className="whitespace-nowrap rounded bg-koma-bg px-1.5 py-0.5 text-[10px] text-koma-fg">
                    {selectedNode.language}
                  </span>
                  <span
                    className="whitespace-nowrap rounded px-1.5 py-0.5 text-[10px]"
                    style={{
                      backgroundColor: (ROLE_COLORS[selectedNode.role] ?? ROLE_COLORS.Overview) + '30',
                      color: ROLE_COLORS[selectedNode.role] ?? ROLE_COLORS.Overview,
                    }}
                  >
                    {selectedNode.role}
                  </span>
                  <span className="whitespace-nowrap text-[10px] text-koma-dim">
                    in: {selectedNode.inDegree} · out: {selectedNode.outDegree}
                  </span>
                </div>

                {/* Dependencies (outgoing) */}
                <div className="mb-3">
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-koma-dim">
                    Dependencies ({directDeps.length})
                  </div>
                  {directDeps.length === 0 ? (
                    <div className="text-[10px] text-koma-dim opacity-50 italic">None</div>
                  ) : (
                    <ul className="max-h-[160px] overflow-y-auto flex flex-col gap-0.5">
                      {directDeps.map((p) => (
                        <li key={p}>
                          <button
                            type="button"
                            onClick={() => selectImportGraphNode(p)}
                            title={p}
                            className="w-full truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100 focus:outline-none focus:ring-1 focus:ring-koma-accent/30 rounded"
                          >
                            <span className="font-medium">{baseName(p)}</span>
                            <span className="ml-1 text-koma-dim opacity-60">{parentDir(p)}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>

                {/* Dependents (incoming) */}
                <div className="mb-3">
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-koma-dim">
                    Dependents ({directDependents.length})
                  </div>
                  {directDependents.length === 0 ? (
                    <div className="text-[10px] text-koma-dim opacity-50 italic">None</div>
                  ) : (
                    <ul className="max-h-[160px] overflow-y-auto flex flex-col gap-0.5">
                      {directDependents.map((p) => (
                        <li key={p}>
                          <button
                            type="button"
                            onClick={() => selectImportGraphNode(p)}
                            title={p}
                            className="w-full truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100 focus:outline-none focus:ring-1 focus:ring-koma-accent/30 rounded"
                          >
                            <span className="font-medium">{baseName(p)}</span>
                            <span className="ml-1 text-koma-dim opacity-60">{parentDir(p)}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>

                {/* Impact section */}
                <div className="mt-2 border-t border-koma-border pt-2">
                  {impactStatus === 'idle' ? (
                    <button
                      type="button"
                      onClick={() => requestImportGraphImpact(selectedNode.path)}
                      className="w-full rounded bg-koma-accent/15 px-2 py-1.5 text-[11px] font-medium text-koma-accent hover:bg-koma-accent/25"
                    >
                      Analyze impact
                      <span className="ml-1 text-[9px] opacity-60">depth 3 · full graph</span>
                    </button>
                  ) : impactStatus === 'loading' ? (
                    <div className="flex items-center gap-2 rounded bg-koma-accent/10 px-2 py-1.5 text-[11px] text-koma-accent">
                      <BrailleSpinner size={12} />
                      <span>Analyzing…</span>
                    </div>
                  ) : impactStatus === 'error' ? (
                    <div>
                      <div className="mb-1 text-[10px] text-koma-error">{impactError ?? 'Impact analysis failed'}</div>
                      <button
                        type="button"
                        onClick={() => requestImportGraphImpact(selectedNode.path)}
                        className="rounded border border-koma-border bg-koma-bg px-2 py-1 text-[10px] text-koma-fg hover:bg-koma-hover"
                      >
                        Retry
                      </button>
                    </div>
                  ) : (
                    <div>
                      <div className="mb-1 flex items-center justify-between">
                        <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">
                          Impact ({impactPaths.length}{impactTotal > impactPaths.length ? ` of ${impactTotal}` : ''})
                        </span>
                        <button
                          type="button"
                          onClick={() => requestImportGraphImpact(selectedNode.path)}
                          title="Rerun impact analysis"
                          className="text-[9px] text-koma-dim hover:text-koma-accent"
                        >
                          ↻ rerun
                        </button>
                      </div>
                      <ul className="max-h-[160px] overflow-y-auto flex flex-col gap-0.5">
                        {impactPaths.map((p) => (
                          <li key={p}>
                            <button
                              type="button"
                              onClick={() => selectImportGraphNode(p)}
                              title={p}
                              className="w-full truncate text-left text-[10px] text-koma-fg opacity-80 hover:text-koma-accent hover:opacity-100 focus:outline-none focus:ring-1 focus:ring-koma-accent/30 rounded"
                            >
                              <span className="font-medium">{baseName(p)}</span>
                              <span className="ml-1 text-koma-dim opacity-60">{parentDir(p)}</span>
                            </button>
                          </li>
                        ))}
                      </ul>
                      {impactTotal > impactPaths.length && (
                        <div className="mt-1 text-[9px] text-koma-dim opacity-50">
                          …and {impactTotal - impactPaths.length} more
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            </div>
          </>
        )}
      </div>

      {/* ── Cmd+K overlay ─────────────────────────────────────────── */}
      {showOmni && (
        <div
          className="absolute inset-0 z-50 flex items-start justify-center bg-black/40 pt-[15vh]"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setShowOmni(false)
          }}
        >
          <div className="w-[520px] max-h-[60vh] flex flex-col overflow-hidden rounded-lg border border-koma-border bg-koma-panel shadow-2xl">
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
                    selectOmniResult(omniResults[omniIdx])
                  } else if (e.key === 'Escape') {
                    setShowOmni(false)
                  }
                }}
                placeholder="Search source files…"
                className="flex-1 bg-transparent text-[13px] text-koma-fg placeholder:text-koma-dim/50 focus:outline-none"
              />
              <kbd className="flex-none rounded bg-koma-bg px-1 py-px text-[9px] text-koma-dim">ESC</kbd>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {omniResults.length === 0 ? (
                <div className="px-3 py-4 text-center text-[12px] text-koma-dim opacity-60">
                  No matching source files
                </div>
              ) : (
                <ul>
                  {omniResults.map((result, i) => {
                    const isDir = 'query' in result
                    const isHighlighted = i === omniIdx
                    return (
                      <li key={isDir ? `dir:${result.label}` : result.path}>
                        <button
                          type="button"
                          onClick={() => selectOmniResult(result)}
                          onMouseEnter={() => setOmniIdx(i)}
                          className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] ${
                            isHighlighted
                              ? 'bg-koma-accent/15 text-koma-accent'
                              : 'text-koma-fg hover:bg-koma-hover'
                          }`}
                        >
                          <span className="min-w-0 flex-1 truncate font-mono text-[11px]">
                            {isDir ? `📁 ${result.label}/` : result.label}
                          </span>
                          {!isDir && (
                            <span className="flex-none text-[9px] text-koma-dim opacity-60">{result.language}</span>
                          )}
                          {isDir && (
                            <span className="flex-none text-[9px] text-koma-dim opacity-40">Enter to drill</span>
                          )}
                        </button>
                      </li>
                    )
                  })}
                </ul>
              )}
            </div>
            <div className="flex-none border-t border-koma-border px-3 py-1.5 text-[10px] text-koma-dim opacity-50">
              ↑↓ navigate · Enter select/drill · Esc close
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

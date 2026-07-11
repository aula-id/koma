import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { GitGraph, Loader2, RefreshCw } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { GitRef } from '../store/koma'
import { computeGitGraph } from '../lib/gitGraphLayout'
import { GraphRow, ROW_H } from './GraphRow'
import { GraphDetail } from './GraphDetail'
import { GraphContextMenu, type GraphMenuTarget } from './GraphContextMenu'
import { ConflictBanner } from './ConflictBanner'

// Rows outside the viewport rendered as a buffer above/below (smooth fast scroll).
const OVERSCAN = 8
// Distance from the bottom (px) that arms an auto load-more.
const LOAD_MORE_PX = 400
// Detail split-pane clamp (px).
const DETAIL_MIN = 120
const DETAIL_MAX = 560

// The GitKraken-style commit-graph tab (G2). A row-VIRTUALIZED commit list with
// an SVG lane gutter, over a resizable detail split. Lazy-loaded (its chunk +
// framer-motion land only when first opened). Read-only in this wave.
export default function GraphTab() {
  const commits = useKoma((s) => s.graph.commits)
  const head = useKoma((s) => s.graph.head)
  const hasMore = useKoma((s) => s.graph.hasMore)
  const loading = useKoma((s) => s.graph.loading)
  const selectedSha = useKoma((s) => s.graph.selectedSha)
  const refreshGraph = useKoma((s) => s.refreshGraph)
  const loadMoreGraph = useKoma((s) => s.loadMoreGraph)
  const selectCommit = useKoma((s) => s.selectCommit)

  // Fetch the first page on mount. The tab persists mounted once opened (see
  // TabbedMain), so this fires exactly once per open.
  useEffect(() => {
    refreshGraph()
  }, [refreshGraph])

  // Recompute the whole layout whenever the commit array changes (a new array is
  // pushed on every GitGraph reply — refresh replaces, load-more concatenates).
  const { rows, laneCount } = useMemo(() => computeGitGraph(commits), [commits])

  const scrollerRef = useRef<HTMLDivElement | null>(null)
  const splitRef = useRef<HTMLDivElement | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportH, setViewportH] = useState(0)
  const [hoveredSha, setHoveredSha] = useState<string | null>(null)
  const [detailH, setDetailH] = useState(240)
  // The right-click context menu's position + target (G4): `null` when
  // closed. Left in local state (not the store) — purely a transient UI
  // overlay, unlike selectedSha/commits which are host-authoritative.
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; target: GraphMenuTarget } | null>(null)

  // Track the scroller's height so the window math has a real viewport (also
  // re-measures when the detail split resizes the list area).
  useEffect(() => {
    const el = scrollerRef.current
    if (!el) return
    const ro = new ResizeObserver(() => setViewportH(el.clientHeight))
    ro.observe(el)
    setViewportH(el.clientHeight)
    return () => ro.disconnect()
  }, [])

  // Windowed slice: only the visible rows (+ overscan) ever hit the DOM, so a
  // 10k-commit graph renders ~40 rows, not 10k.
  const total = rows.length
  const totalH = total * ROW_H
  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN)
  const end = Math.min(total, Math.ceil((scrollTop + viewportH) / ROW_H) + OVERSCAN)
  const visible = rows.slice(start, end)

  // Which shas have already animated in — a sha animates the FIRST time it
  // becomes visible (initial load stagger / load-more / scroll-in), never again
  // on a later scroll. Updated in an effect after each render (mutating in render
  // would break under React's double-invoke).
  const animatedRef = useRef<Set<string>>(new Set())
  useEffect(() => {
    for (const r of visible) animatedRef.current.add(r.sha)
  })

  const onScroll = useCallback(() => {
    const el = scrollerRef.current
    if (!el) return
    setScrollTop(el.scrollTop)
    // Near-bottom infinite scroll — the store's own `loading`/`hasMore` guard
    // makes a duplicate call a no-op, but gating here avoids the churn.
    if (hasMore && !loading && el.scrollTop + el.clientHeight >= el.scrollHeight - LOAD_MORE_PX) {
      loadMoreGraph()
    }
  }, [hasMore, loading, loadMoreGraph])

  const onSelect = useCallback((sha: string) => selectCommit(sha), [selectCommit])
  const onHover = useCallback((sha: string | null) => setHoveredSha(sha), [])
  const onRowContextMenu = useCallback((e: ReactMouseEvent, sha: string) => {
    setCtxMenu({ x: e.clientX, y: e.clientY, target: { kind: 'commit', sha } })
  }, [])
  const onRefContextMenu = useCallback((e: ReactMouseEvent, name: string, refKind: GitRef['kind']) => {
    setCtxMenu({ x: e.clientX, y: e.clientY, target: { kind: 'ref', name, refKind } })
  }, [])
  const closeCtxMenu = useCallback(() => setCtxMenu(null), [])

  // Smooth-scroll a freshly-selected row into view when it's off-screen (a parent
  // chip click can jump far). Centres it roughly in the viewport.
  useEffect(() => {
    if (!selectedSha) return
    const idx = rows.findIndex((r) => r.sha === selectedSha)
    if (idx < 0) return
    const el = scrollerRef.current
    if (!el) return
    const rowTop = idx * ROW_H
    const rowBottom = rowTop + ROW_H
    if (rowTop < el.scrollTop || rowBottom > el.scrollTop + el.clientHeight) {
      el.scrollTo({ top: Math.max(0, rowTop - el.clientHeight / 2), behavior: 'smooth' })
    }
  }, [selectedSha, rows])

  // Detail split drag (vertical): dragging the handle UP grows the bottom detail
  // pane. Clamped to [DETAIL_MIN, min(DETAIL_MAX, 70% of the split height)].
  const startDetailResize = (e: ReactMouseEvent) => {
    e.preventDefault()
    const startY = e.clientY
    const startH = detailH
    const splitH = splitRef.current?.clientHeight ?? 0
    const onMove = (ev: MouseEvent) => {
      const max = Math.min(DETAIL_MAX, splitH > 0 ? splitH * 0.7 : DETAIL_MAX)
      const next = Math.min(max, Math.max(DETAIL_MIN, startH - (ev.clientY - startY)))
      setDetailH(next)
    }
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.body.style.cursor = 'ns-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      <ConflictBanner />
      {/* Toolbar */}
      <div className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[12px] text-koma-dim">
        <GitGraph size={13} className="flex-none opacity-70" />
        <span className="font-mono">
          {total} commit{total === 1 ? '' : 's'}
          {hasMore ? '+' : ''}
        </span>
        <span className="flex-1" />
        {loading && <Loader2 size={13} className="flex-none animate-spin opacity-70" />}
        <button
          type="button"
          onClick={refreshGraph}
          title="Refresh graph"
          aria-label="Refresh graph"
          className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <RefreshCw size={13} />
        </button>
      </div>

      {/* List (top) + detail (bottom) split */}
      <div ref={splitRef} className="flex min-h-0 flex-1 flex-col">
        <div
          ref={scrollerRef}
          onScroll={onScroll}
          className="relative min-h-0 flex-1 overflow-y-auto"
        >
          {total === 0 ? (
            <div className="flex h-full w-full items-center justify-center px-6 text-center text-[12px] text-koma-dim">
              {loading ? (
                <Loader2 size={18} className="animate-spin opacity-70" />
              ) : (
                'No commits to display.'
              )}
            </div>
          ) : (
            <>
              {/* Spacer sized to the FULL list; visible rows absolutely placed. */}
              <div style={{ height: totalH }} className="relative">
                {visible.map((row, i) => {
                  const globalIdx = start + i
                  return (
                    <div key={row.sha} className="absolute inset-x-0" style={{ top: globalIdx * ROW_H }}>
                      <GraphRow
                        row={row}
                        laneCount={laneCount}
                        isHead={row.sha === head}
                        selected={row.sha === selectedSha}
                        dim={hoveredSha !== null && hoveredSha !== row.sha}
                        animate={!animatedRef.current.has(row.sha)}
                        staggerIndex={i}
                        onSelect={onSelect}
                        onHover={onHover}
                        onContextMenu={onRowContextMenu}
                        onRefContextMenu={onRefContextMenu}
                      />
                    </div>
                  )
                })}
              </div>
              {/* Manual load-more fallback (below the spacer, in normal flow). */}
              {hasMore && (
                <div className="flex justify-center py-2">
                  <button
                    type="button"
                    onClick={loadMoreGraph}
                    disabled={loading}
                    className="flex items-center gap-1.5 rounded border border-koma-border px-2.5 py-1 text-[11px] text-koma-dim transition hover:bg-koma-hover hover:text-koma-fg disabled:opacity-50"
                  >
                    {loading && <Loader2 size={12} className="animate-spin" />}
                    Load more
                  </button>
                </div>
              )}
            </>
          )}
        </div>

        {selectedSha && (
          <>
            <div
              onMouseDown={startDetailResize}
              className="h-[5px] flex-none cursor-ns-resize border-t border-koma-border hover:bg-koma-grip"
            />
            <div style={{ height: detailH }} className="min-h-0 flex-none bg-koma-panel2">
              <GraphDetail />
            </div>
          </>
        )}
      </div>

      {ctxMenu && (
        <GraphContextMenu x={ctxMenu.x} y={ctxMenu.y} target={ctxMenu.target} onClose={closeCtxMenu} />
      )}
    </div>
  )
}

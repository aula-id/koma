import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react'
import { GitGraph, Loader2, RefreshCw, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { GitRef } from '../store/koma'
import { computeGitGraph } from '../lib/gitGraphLayout'
import { GraphRow, ROW_H } from './GraphRow'
import { GraphDetail } from './GraphDetail'
import { GraphContextMenu, type GraphMenuTarget } from './GraphContextMenu'
import { ConflictBanner } from './ConflictBanner'
import { RebaseDropConfirm } from './RebaseDropConfirm'
import { GraphBreadcrumb } from './GraphBreadcrumb'
import { GraphRefTree } from './GraphRefTree'
import GraphBubble from './GraphBubble'

// Rows outside the viewport rendered as a buffer above/below (smooth fast scroll).
const OVERSCAN = 8
// Distance from the bottom (px) that arms an auto load-more.
const LOAD_MORE_PX = 400
// Right detail-pane resizable-WIDTH clamp (px) — GK4b moved the detail pane
// from a bottom split to a right split (GitKraken-style).
const DETAIL_W_MIN = 240
const DETAIL_W_MAX = 560
const DETAIL_W_DEFAULT = 320
// Left sidebar (GK4b: ref-tree; the GK3 Changes accordion was removed —
// working-tree changes live only in the sidebar Source Control panel now)
// resizable-width clamp (px).
const SIDEBAR_MIN = 180
const SIDEBAR_MAX = 420
const SIDEBAR_DEFAULT = 240

// The GitKraken-style commit-graph tab (G2). A row-VIRTUALIZED commit list with
// an SVG lane gutter, beside a resizable RIGHT detail pane (GK4b). Lazy-loaded
// (its chunk + framer-motion land only when first opened). Read-only in this
// wave.
export default function GraphTab() {
  const commits = useKoma((s) => s.graph.commits)
  const head = useKoma((s) => s.graph.head)
  const hasMore = useKoma((s) => s.graph.hasMore)
  const loading = useKoma((s) => s.graph.loading)
  // How the LAST GitGraph reply was folded in — 'append' (load-more) keeps the
  // viewport pinned where the user was; 'replace' (refresh / first load) is the
  // only case that's SUPPOSED to land back at the top. Read (not a dep) inside
  // the scroll-preserving effect below so it reflects the mode of whichever
  // reply just landed, not a stale value from the click that triggered it.
  const loadMode = useKoma((s) => s.graph.loadMode)
  const selectedSha = useKoma((s) => s.graph.selectedSha)
  const refreshGraph = useKoma((s) => s.refreshGraph)
  const loadMoreGraph = useKoma((s) => s.loadMoreGraph)
  const selectCommit = useKoma((s) => s.selectCommit)
  const clearSelection = useKoma((s) => s.clearSelection)
  const gitRebase = useKoma((s) => s.gitRebase)
  const graphMode = useKoma((s) => s.graph.graphMode)
  // Whether THIS tab is the one currently showing (routes/index.tsx toggles
  // tabs via a `hidden` class, it never unmounts them) — used below to
  // re-measure `viewportH` on the display:none -> visible flip, since a
  // ResizeObserver frequently misses that transition on WebKitGTK.
  const isActiveTab = useKoma((s) => s.ui.activeTabId === 'graph')

  // Fetch the first page on mount. The tab persists mounted once opened (see
  // TabbedMain), so this fires exactly once per open.
  useEffect(() => {
    refreshGraph()
  }, [refreshGraph])

  // Recompute the whole layout whenever the commit array changes (a new array is
  // pushed on every GitGraph reply — refresh replaces, load-more concatenates).
  const { rows, laneCount } = useMemo(() => computeGitGraph(commits), [commits])

  const scrollerRef = useRef<HTMLDivElement | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  // Non-zero from the first render (falls back to the window's height when
  // `window` isn't available, e.g. SSR) — a real ResizeObserver measurement
  // corrects this shortly after, but the window calc below must never see a
  // 0 here or it collapses to just OVERSCAN rows (see the effects below for
  // why the observer alone isn't reliable on this tab).
  const [viewportH, setViewportH] = useState(() =>
    typeof window !== 'undefined' ? window.innerHeight : 800,
  )
  const [hoveredSha, setHoveredSha] = useState<string | null>(null)
  // Right detail-pane WIDTH (GK4b) — a plain drag-grip resize, mirroring
  // startSidebarResize below but on the opposite edge.
  const [detailW, setDetailW] = useState(DETAIL_W_DEFAULT)
  // Left sidebar width (GK3) — a plain drag-grip resize, mirroring
  // startDetailResize below but on the horizontal axis.
  const [sidebarW, setSidebarW] = useState(SIDEBAR_DEFAULT)
  // The right-click context menu's position + target (G4): `null` when
  // closed. Left in local state (not the store) — purely a transient UI
  // overlay, unlike selectedSha/commits which are host-authoritative.
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; target: GraphMenuTarget } | null>(null)

  // ---- GitKraken-style drag-to-rebase (G6) ----
  // The branch name currently being dragged (from a local/HEAD ref chip), or
  // null when nothing is dragging — lifted here so it's shared across every
  // virtualized row (a drag can span rows the source row is no longer near).
  const [draggedBranch, setDraggedBranch] = useState<string | null>(null)
  // The drop-target id (`commit:<sha>` / `ref:<name>`) currently hovered by
  // the drag — drives the hover highlight, the main drop affordance.
  const [dropHoverId, setDropHoverId] = useState<string | null>(null)
  // A valid (non-no-op) drop just landed — the pending confirm popover's
  // position + branch/target, or null when none is showing. Cleared on
  // confirm, cancel, or dismiss; never fires the rebase speculatively.
  const [rebaseConfirm, setRebaseConfirm] = useState<{
    x: number
    y: number
    branch: string
    target: string
    targetLabel: string
  } | null>(null)

  const handleBranchDragStart = useCallback((name: string) => setDraggedBranch(name), [])
  // dragend is the authoritative clear point (fires on both a successful drop
  // and a cancelled drag) — also called defensively right after a drop.
  const handleBranchDragEnd = useCallback(() => {
    setDraggedBranch(null)
    setDropHoverId(null)
  }, [])
  const handleDropHover = useCallback((id: string | null) => setDropHoverId(id), [])
  const handleRebaseDrop = useCallback(
    (branch: string, target: string, targetLabel: string, x: number, y: number) => {
      setRebaseConfirm({ x, y, branch, target, targetLabel })
    },
    [],
  )

  // Belt-and-suspenders drag-state cleanup (G6 fix): the graph rows are
  // virtualized, so if the source chip's row scrolls out of the overscan
  // window mid-drag, React unmounts that chip and the native dragend/drop
  // fires on the now-detached node — it never reaches GraphRow's own
  // onDragEnd (React's delegated handler), so draggedBranch/dropHoverId get
  // stuck forever (the `if (!draggedBranch) onSelect(...)` guard in
  // GraphRow then blocks selecting ANY commit until reload). Per the HTML DnD
  // spec, when an event's original target is no longer in a document, the
  // user agent redirects dispatch to the Window — so a capture-phase window
  // listener always sees dragend/drop, detached chip or not. Harmless on a
  // normal drop too: it just re-clears the same transient state GraphRow's
  // own onDragEnd already cleared; the rebaseConfirm popover below holds its
  // own branch/target copy (staged by onRebaseDrop before this runs), so a
  // pending confirm is never wiped by this.
  useEffect(() => {
    window.addEventListener('dragend', handleBranchDragEnd, true)
    window.addEventListener('drop', handleBranchDragEnd, true)
    return () => {
      window.removeEventListener('dragend', handleBranchDragEnd, true)
      window.removeEventListener('drop', handleBranchDragEnd, true)
    }
  }, [handleBranchDragEnd])

  // Track the scroller's height so the window math has a real viewport (also
  // re-fires on a sidebar/detail-pane WIDTH resize — those are ResizeObserver
  // events too, just harmless no-ops for `viewportH` since they don't change
  // the scroller's height). Guarded to only ever apply a POSITIVE height: a
  // transient 0 (e.g. mid-hide, or this tab currently being the non-visible
  // one) must never stomp a good measurement back down to 0.
  useEffect(() => {
    const el = scrollerRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      if (el.clientHeight > 0) setViewportH(el.clientHeight)
    })
    ro.observe(el)
    if (el.clientHeight > 0) setViewportH(el.clientHeight)
    // Belt-and-suspenders: measure once more after layout settles, in case
    // the very first synchronous read above raced the initial paint.
    const raf = requestAnimationFrame(() => {
      if (el.clientHeight > 0) setViewportH(el.clientHeight)
    })
    return () => {
      ro.disconnect()
      cancelAnimationFrame(raf)
    }
  }, [])

  // Re-measure when this tab actually becomes the visible one. The tab is
  // mounted (and may be auto-opened) while its container is still
  // `display:none` (routes/index.tsx toggles tabs via a `hidden` class
  // rather than unmounting), so the mount-time `clientHeight` above can read
  // 0 — and on WebKitGTK the ResizeObserver frequently never fires on the
  // subsequent display:none -> visible flip. Rerun the measurement on that
  // transition specifically, inside a `requestAnimationFrame` so the reveal's
  // layout has actually settled before reading `clientHeight`.
  useEffect(() => {
    if (!isActiveTab) return
    const raf = requestAnimationFrame(() => {
      const el = scrollerRef.current
      if (el && el.clientHeight > 0) setViewportH(el.clientHeight)
    })
    return () => cancelAnimationFrame(raf)
  }, [isActiveTab])

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

  // The scrollTop captured the instant a load-more fires (button click OR the
  // near-bottom auto-trigger), replayed once the appended page actually lands
  // — see the layout effect below. `null` when no restore is pending.
  const pendingScrollRestoreRef = useRef<number | null>(null)

  // Shared load-more trigger for both the explicit button and the near-bottom
  // auto-load. Re-guards `loading`/`hasMore` itself (loadMoreGraph() also
  // guards store-side, but re-checking here means a no-op click never
  // stomps a stale scrollTop into `pendingScrollRestoreRef`).
  const handleLoadMore = useCallback(() => {
    if (loading || !hasMore) return
    const el = scrollerRef.current
    if (el) pendingScrollRestoreRef.current = el.scrollTop
    loadMoreGraph()
  }, [loading, hasMore, loadMoreGraph])

  // Load-more append growing `commits` re-renders the whole windowed list
  // (new `rows`/`totalH` from the layout useMemo), but never itself touches
  // the scroller's native scrollTop — so by default the browser leaves it
  // exactly where it was, which is the behaviour we want (new rows only
  // extend the content BELOW the fold). What used to visibly "teleport to
  // top" was the OLD `disabled` attribute on the load-more button: disabling
  // a focused control forces an immediate blur, and the browser's implicit
  // focus-revert-to-<body> step scrolls the nearest scrollable ancestor back
  // toward its default (0,0) origin — a real WebKit/Blink quirk, not
  // something React caused. The button below now uses `aria-disabled` +
  // pointer-events instead of `disabled`, so it never loses focus and the
  // browser never gets a chance to "helpfully" re-scroll. This effect is the
  // belt-and-suspenders half: it restores the captured scrollTop after every
  // reply (a no-op if nothing moved), and explicitly snaps back to the top
  // for a genuine refresh/first-load — the one case that SHOULD reset.
  useLayoutEffect(() => {
    const el = scrollerRef.current
    if (!el) return
    const pending = pendingScrollRestoreRef.current
    if (pending != null) {
      el.scrollTop = pending
      pendingScrollRestoreRef.current = null
    } else if (loadMode === 'replace') {
      el.scrollTop = 0
    }
    // Intentionally keyed on `commits` alone (the actual GitGraph reply
    // landing), not `loadMode` — `loadMode` flips to 'append' synchronously
    // on the click, well before the reply arrives, and firing this early
    // would consume `pendingScrollRestoreRef` before the row count it was
    // meant to compensate for ever changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [commits])

  const onScroll = useCallback(() => {
    const el = scrollerRef.current
    if (!el) return
    setScrollTop(el.scrollTop)
    // Near-bottom auto-load — a gentle assist; the explicit "See more" button
    // below is the primary, always-visible trigger.
    if (hasMore && !loading && el.scrollTop + el.clientHeight >= el.scrollHeight - LOAD_MORE_PX) {
      handleLoadMore()
    }
  }, [hasMore, loading, handleLoadMore])

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

  // Jump the graph scroller straight to a already-known sha (GK4b — the
  // ref-tree's click handler resolves a ref to a loaded commit, then calls
  // this to bring it into view before/alongside `selectCommit`). Distinct
  // from the effect above (which only nudges when the selection is
  // off-screen): this always scrolls, roughly centering the row. A no-op
  // when the sha isn't in the currently-loaded page.
  const scrollToSha = useCallback(
    (sha: string) => {
      const idx = rows.findIndex((r) => r.sha === sha)
      if (idx < 0) return
      const el = scrollerRef.current
      if (!el) return
      const rowTop = idx * ROW_H
      const target = Math.max(0, Math.min(rowTop - el.clientHeight / 2, el.scrollHeight - el.clientHeight))
      el.scrollTo({ top: target, behavior: 'smooth' })
    },
    [rows],
  )

  // Detail pane WIDTH drag (horizontal, GK4b): the grip sits on the pane's
  // LEFT edge, so dragging it LEFT grows the (right-anchored) detail pane.
  // Clamped to [DETAIL_W_MIN, DETAIL_W_MAX].
  const startDetailResize = (e: ReactMouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startW = detailW
    const onMove = (ev: MouseEvent) => {
      const next = Math.min(DETAIL_W_MAX, Math.max(DETAIL_W_MIN, startW - (ev.clientX - startX)))
      setDetailW(next)
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

  // Sidebar width drag (horizontal): dragging the handle RIGHT grows the
  // sidebar. Clamped to [SIDEBAR_MIN, SIDEBAR_MAX].
  const startSidebarResize = (e: ReactMouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startW = sidebarW
    const onMove = (ev: MouseEvent) => {
      const next = Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, startW + (ev.clientX - startX)))
      setSidebarW(next)
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

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      <ConflictBanner />
      <GraphBreadcrumb />

      {graphMode === 'rail' && (
        <div className="flex min-h-0 flex-1">
          {/* LEFT sidebar (GK4b ref-tree only — the GK3 Changes accordion
              was removed to avoid duplicating the sidebar Source Control
              panel): a scrollable vertical stack of AccordionSections
              (LOCAL/REMOTE/TAGS), each flex-filling when open and collapsing
              to just its header when closed. */}
          <div
            style={{ width: sidebarW }}
            className="flex min-h-0 flex-none flex-col overflow-y-auto border-r border-koma-border"
          >
            <GraphRefTree scrollToSha={scrollToSha} />
          </div>
          <div
            onMouseDown={startSidebarResize}
            className="w-[5px] flex-none cursor-ew-resize border-r border-koma-border hover:bg-koma-grip"
          />

          {/* CENTER: toolbar + virtualized graph list (unchanged behavior,
              now its own column instead of sharing one with the detail
              pane below it — `min-w-0` lets it actually shrink instead of
              shoving the right-hand detail pane off-screen). */}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
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
                            draggedBranch={draggedBranch}
                            dropHoverId={dropHoverId}
                            onBranchDragStart={handleBranchDragStart}
                            onBranchDragEnd={handleBranchDragEnd}
                            onDropHover={handleDropHover}
                            onRebaseDrop={handleRebaseDrop}
                          />
                        </div>
                      )
                    })}
                  </div>
                  {/* Explicit "See more" pagination (below the spacer, in normal flow) —
                      the primary load-more trigger; the near-bottom auto-load in
                      `onScroll` is just a gentle assist on top of this. Uses
                      `aria-disabled` + pointer-events (NOT the `disabled` attribute) so a
                      focused click never forces a blur — disabling a focused control
                      makes the browser revert focus to <body>, which drags the nearest
                      scrollable ancestor's scrollTop back toward 0 as a side effect
                      (the old "teleports to top" bug). */}
                  {hasMore && (
                    <div className="flex justify-center py-3">
                      <button
                        type="button"
                        onClick={handleLoadMore}
                        aria-disabled={loading}
                        className={`flex items-center gap-1.5 rounded bg-koma-accent px-3.5 py-1.5 text-[12px] font-semibold text-koma-bg transition-opacity hover:opacity-90 ${
                          loading ? 'pointer-events-none opacity-60' : ''
                        }`}
                      >
                        {loading && <Loader2 size={12} className="animate-spin" />}
                        See more
                      </button>
                    </div>
                  )}
                </>
              )}
            </div>
          </div>

          {/* RIGHT detail pane (GK4b) — a horizontal split instead of the
              old bottom split, mirroring GitKraken's own commit-detail
              placement. `min-w-0` (on top of the fixed inline width) keeps
              it a well-behaved flex child; its own `overflow-y-auto` scrolls
              independently of the graph column. Only rendered while a commit
              is selected — closable via the header's X button, which calls
              clearSelection; the center column's own flex-1 then reflows to
              fill the freed width automatically. */}
          {selectedSha && (
            <>
              {/* resize grip */}
              <div
                onMouseDown={startDetailResize}
                className="w-[5px] flex-none cursor-ew-resize border-l border-koma-border hover:bg-koma-grip"
              />
              <div
                style={{ width: detailW }}
                className="flex min-h-0 min-w-0 flex-none flex-col overflow-y-auto border-l border-koma-border bg-koma-panel2"
              >
                <div className="flex flex-none items-center justify-between border-b border-koma-border px-3 py-1.5">
                  <span className="text-[11px] font-medium uppercase tracking-wide text-koma-dim">Commit Detail</span>
                  <button
                    type="button"
                    onClick={clearSelection}
                    title="Close detail"
                    aria-label="Close detail"
                    className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
                  >
                    <X size={13} />
                  </button>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto">
                  <GraphDetail />
                </div>
              </div>
            </>
          )}
        </div>
      )}

      {graphMode === 'bubble' && (
        <div className="flex min-h-0 flex-1">
          <GraphBubble />
        </div>
      )}

      {ctxMenu && (
        <GraphContextMenu x={ctxMenu.x} y={ctxMenu.y} target={ctxMenu.target} onClose={closeCtxMenu} />
      )}

      {rebaseConfirm && (
        <RebaseDropConfirm
          x={rebaseConfirm.x}
          y={rebaseConfirm.y}
          branch={rebaseConfirm.branch}
          targetLabel={rebaseConfirm.targetLabel}
          onConfirm={() => {
            gitRebase(rebaseConfirm.target, rebaseConfirm.branch)
            setRebaseConfirm(null)
          }}
          onCancel={() => setRebaseConfirm(null)}
        />
      )}
    </div>
  )
}

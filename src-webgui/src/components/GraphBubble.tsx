import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ChartScatter, Filter, Loader2, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { ActivityCommit } from '../store/koma'
import { aggregateAuthors, buildTimeTicks, linearScale, radiusScale } from '../lib/bubbleScales'

// ---- Render geometry --------------------------------------------------
const LEFT_MARGIN = 132 // author-label gutter
const RIGHT_MARGIN = 20
const LANE_H = 30 // one author lane
const TOP_PAD = 14
const BAR_STRIP_MIN = 90 // add(up)/del(down) timeline strip — floor when many authors/short container
const AXIS_H = 22 // x-axis date tick labels
const BOTTOM_PAD = 6
const MIN_R = 3
const MAX_R = 15
const RADIUS_K = 0.85
const X_TICK_COUNT = 6
const DEFAULT_WIDTH = 800 // never let width math see a 0 before ResizeObserver settles

// 1-2 letter fallback truncation for a lane's author-name label (the gutter is
// narrow — a full name would overflow into the chart). Not the same helper as
// AuthorAvatar's initials (that's a badge, this is a text label).
function truncateName(name: string, email: string): string {
  const src = name.trim() || email.trim() || 'unknown'
  return src.length > 18 ? `${src.slice(0, 17)}…` : src
}

function formatDate(iso: string): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  return new Date(t).toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

type HoverState = { x: number; y: number; commit: ActivityCommit }

// GK5b: the commit-graph tab's Bubble-mode activity chart (GitLens-style
// visual history) — authors on the Y axis (one lane each, busiest on top),
// time on the X axis, one bubble per commit sized by lines-changed and
// coloured per author, with an additions/deletions bar timeline underneath.
// Hand-rolled SVG (no charting library in this repo). Only ever mounted while
// `graphMode === 'bubble'` (GraphTab's conditional render), so its own mount
// effect is the right place to (re)fetch the activity series.
export default function GraphBubble() {
  const commits = useKoma((s) => s.activity.commits)
  const loading = useKoma((s) => s.activity.loading)
  const error = useKoma((s) => s.activity.error)
  const activePath = useKoma((s) => s.activity.path)
  const refreshActivity = useKoma((s) => s.refreshActivity)

  // Fetch (or re-fetch) the CURRENT path filter on mount — this component
  // only mounts while bubble mode is showing, so re-opening the tab always
  // reloads. Deliberately reads `activity.path`'s value at mount time rather
  // than reacting to it (a live dependency would loop: refreshActivity sets
  // `path`, which would re-trigger this effect).
  useEffect(() => {
    refreshActivity(activePath)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshActivity])

  // ---- Responsive width (never divide by 0) ----
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [width, setWidth] = useState(DEFAULT_WIDTH)
  // ---- Responsive height (never let the histogram see a 0 before the
  // ResizeObserver settles) — non-zero default, same lesson as GraphTab's
  // viewportH: the container can be measured while still display:none (this
  // component only mounts in bubble mode, but its GraphTab parent may itself
  // be the hidden tab), so a raw ResizeObserver alone can miss the reveal.
  const [viewportH, setViewportH] = useState(() =>
    typeof window !== 'undefined' ? window.innerHeight : 800,
  )
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) setWidth(el.clientWidth)
      if (el.clientHeight > 0) setViewportH(el.clientHeight)
    })
    ro.observe(el)
    if (el.clientWidth > 0) setWidth(el.clientWidth)
    if (el.clientHeight > 0) setViewportH(el.clientHeight)
    // Belt-and-suspenders: the very first synchronous read above can race the
    // initial paint (same lesson as GraphTab's viewportH measurement).
    const raf = requestAnimationFrame(() => {
      if (el.clientWidth > 0) setWidth(el.clientWidth)
      if (el.clientHeight > 0) setViewportH(el.clientHeight)
    })
    return () => {
      ro.disconnect()
      cancelAnimationFrame(raf)
    }
  }, [])

  // Re-measure height when the graph tab actually becomes the visible one —
  // mirrors GraphTab's own isActiveTab re-measure effect (WebKitGTK
  // frequently misses the ResizeObserver firing on a display:none -> visible
  // flip).
  const isActiveTab = useKoma((s) => s.ui.activeTabId === 'graph')
  useEffect(() => {
    if (!isActiveTab) return
    const raf = requestAnimationFrame(() => {
      const el = containerRef.current
      if (el && el.clientHeight > 0) setViewportH(el.clientHeight)
    })
    return () => cancelAnimationFrame(raf)
  }, [isActiveTab])

  // ---- Path-filter control (GK5b Part 3) ----
  const [pathInput, setPathInput] = useState(activePath ?? '')
  const submitPathFilter = () => {
    const trimmed = pathInput.trim()
    refreshActivity(trimmed.length > 0 ? trimmed : null)
  }
  const clearPathFilter = () => {
    setPathInput('')
    refreshActivity(null)
  }

  // ---- Aggregation + scales (recomputed only when the commit series or
  // measured width changes) ----
  const authors = useMemo(() => aggregateAuthors(commits), [commits])
  const laneIndex = useMemo(() => {
    const m = new Map<string, number>()
    authors.forEach((a, i) => m.set(a.key, i))
    return m
  }, [authors])

  const chartW = Math.max(0, width - LEFT_MARGIN - RIGHT_MARGIN)
  const lanesH = authors.length * LANE_H
  // Author lanes stay fixed at the top; the add/del histogram absorbs
  // whatever vertical space the container has beyond the fixed chrome
  // (lanes + axis + padding), down to a minimum floor — so the chart fills
  // the (tall, flex-1) body container instead of leaving dead empty space
  // below a fixed-height strip.
  const contentMinH = TOP_PAD + lanesH + BAR_STRIP_MIN + AXIS_H + BOTTOM_PAD
  const svgH = Math.max(contentMinH, viewportH)
  const barStripH = svgH - TOP_PAD - lanesH - AXIS_H - BOTTOM_PAD

  const timestamps = useMemo(() => commits.map((c) => Date.parse(c.date)).filter((t) => !Number.isNaN(t)), [commits])
  const minTs = timestamps.length > 0 ? Math.min(...timestamps) : 0
  const maxTs = timestamps.length > 0 ? Math.max(...timestamps) : 0
  const xScale = useMemo(
    () => linearScale(minTs, maxTs, LEFT_MARGIN + 4, LEFT_MARGIN + chartW - 4),
    [minTs, maxTs, chartW],
  )
  const ticks = useMemo(() => buildTimeTicks(minTs, maxTs, X_TICK_COUNT), [minTs, maxTs])

  const maxAbsLines = useMemo(
    () => Math.max(1, ...commits.map((c) => Math.max(c.added, c.deleted))),
    [commits],
  )

  const barStripTop = TOP_PAD + lanesH
  const barBaseline = barStripTop + barStripH / 2

  const [hover, setHover] = useState<HoverState | null>(null)
  const showTooltip = (e: ReactMouseEvent, commit: ActivityCommit) =>
    setHover({ x: e.clientX, y: e.clientY, commit })
  const hideTooltip = () => setHover(null)

  const empty = !loading && (error != null || commits.length === 0)

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      {/* Header: legend + path-narrowing filter (GK5b Part 3) */}
      <div className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[11px]">
        <div className="flex min-w-0 flex-1 items-center gap-3 overflow-x-auto">
          {authors.map((a) => (
            <span key={a.key} className="flex flex-none items-center gap-1 whitespace-nowrap" title={a.email}>
              <span
                className="h-2.5 w-2.5 flex-none rounded-full"
                style={{ backgroundColor: a.color }}
              />
              <span className="text-koma-fg opacity-80">{a.name || a.email || 'unknown'}</span>
              <span className="font-mono opacity-70">
                <span className="text-koma-success">+{a.totalAdded}</span>{' '}
                <span className="text-koma-error">-{a.totalDeleted}</span>
              </span>
            </span>
          ))}
          {authors.length === 0 && !loading && (
            <span className="text-koma-dim opacity-60">No authors yet</span>
          )}
        </div>
        <div className="flex flex-none items-center gap-1">
          <Filter size={12} className="flex-none text-koma-dim opacity-60" />
          <input
            type="text"
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitPathFilter()
            }}
            placeholder="filter by path…"
            className="w-40 rounded border border-koma-border bg-koma-bg px-1.5 py-0.5 text-[11px] text-koma-fg outline-none focus:border-koma-accent"
          />
          {activePath && (
            <button
              type="button"
              onClick={clearPathFilter}
              title="Clear path filter (show whole branch)"
              aria-label="Clear path filter"
              className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
            >
              <X size={12} />
            </button>
          )}
          {loading && <Loader2 size={12} className="flex-none animate-spin text-koma-dim opacity-70" />}
        </div>
      </div>

      {/* Body */}
      <div ref={containerRef} className="relative min-h-0 flex-1 overflow-auto">
        {empty ? (
          <div className="flex h-full w-full flex-col items-center justify-center gap-2 px-6 text-center text-koma-dim">
            <ChartScatter size={28} className="opacity-50" />
            <span className="text-[13px] font-medium opacity-80">
              {error ?? 'No activity to display.'}
            </span>
            {activePath && !error && (
              <span className="text-[11px] opacity-50">Filtered to “{activePath}” — try clearing the filter.</span>
            )}
          </div>
        ) : loading && commits.length === 0 ? (
          <div className="flex h-full w-full items-center justify-center">
            <Loader2 size={20} className="animate-spin text-koma-dim opacity-70" />
          </div>
        ) : (
          <svg width={Math.max(width, LEFT_MARGIN + RIGHT_MARGIN + 1)} height={svgH} className="block">
            {/* Author lanes: label + faint separator line */}
            {authors.map((a, i) => {
              const cy = TOP_PAD + i * LANE_H + LANE_H / 2
              return (
                <g key={a.key}>
                  <line
                    x1={0}
                    y1={TOP_PAD + i * LANE_H + LANE_H}
                    x2={width}
                    y2={TOP_PAD + i * LANE_H + LANE_H}
                    stroke="var(--color-koma-border)"
                    strokeWidth={1}
                    opacity={0.4}
                  />
                  <circle cx={10} cy={cy} r={3.5} fill={a.color} />
                  <text
                    x={20}
                    y={cy}
                    dominantBaseline="middle"
                    className="fill-koma-fg"
                    style={{ fontSize: 11, opacity: 0.85 }}
                  >
                    {truncateName(a.name, a.email)}
                  </text>
                </g>
              )
            })}

            {/* Bubbles: one per commit, cx = time, cy = author lane, r = lines changed */}
            {commits.map((c) => {
              const lane = laneIndex.get(c.email.trim() || c.author.trim() || '?')
              if (lane === undefined) return null
              const ts = Date.parse(c.date)
              if (Number.isNaN(ts)) return null
              const cx = xScale(ts)
              const cy = TOP_PAD + lane * LANE_H + LANE_H / 2
              const r = radiusScale(c.added + c.deleted, RADIUS_K, MIN_R, MAX_R)
              const fill = authors[lane]?.color ?? '#888'
              return (
                <circle
                  key={c.sha}
                  cx={cx}
                  cy={cy}
                  r={r}
                  fill={fill}
                  fillOpacity={0.55}
                  stroke={fill}
                  strokeOpacity={0.9}
                  strokeWidth={1}
                  className="cursor-pointer"
                  onMouseEnter={(e) => showTooltip(e, c)}
                  onMouseMove={(e) => showTooltip(e, c)}
                  onMouseLeave={hideTooltip}
                >
                  <title>
                    {c.sha.slice(0, 7)} · {c.author} · +{c.added} -{c.deleted} · {formatDate(c.date)}
                  </title>
                </circle>
              )
            })}

            {/* Add/del bar timeline strip: added drawn up (success), deleted down (error) */}
            <line
              x1={0}
              y1={barBaseline}
              x2={width}
              y2={barBaseline}
              stroke="var(--color-koma-border)"
              strokeWidth={1}
              opacity={0.5}
            />
            {commits.map((c) => {
              const ts = Date.parse(c.date)
              if (Number.isNaN(ts)) return null
              const cx = xScale(ts)
              const halfH = barStripH / 2 - 2
              const addH = (c.added / maxAbsLines) * halfH
              const delH = (c.deleted / maxAbsLines) * halfH
              return (
                <g key={`bar:${c.sha}`}>
                  {addH > 0 && (
                    <line
                      x1={cx}
                      y1={barBaseline}
                      x2={cx}
                      y2={barBaseline - addH}
                      className="stroke-koma-success"
                      strokeWidth={1.5}
                      opacity={0.85}
                    />
                  )}
                  {delH > 0 && (
                    <line
                      x1={cx}
                      y1={barBaseline}
                      x2={cx}
                      y2={barBaseline + delH}
                      className="stroke-koma-error"
                      strokeWidth={1.5}
                      opacity={0.85}
                    />
                  )}
                </g>
              )
            })}

            {/* X-axis date ticks */}
            {ticks.map((t, i) => {
              const cx = xScale(t.ts)
              const y = barStripTop + barStripH + AXIS_H / 2 + 4
              return (
                <text
                  key={i}
                  x={cx}
                  y={y}
                  textAnchor={i === 0 ? 'start' : i === ticks.length - 1 ? 'end' : 'middle'}
                  className="fill-koma-dim"
                  style={{ fontSize: 10, opacity: 0.7 }}
                >
                  {t.label}
                </text>
              )
            })}
          </svg>
        )}

        {hover && (
          <div
            style={{ position: 'fixed', left: hover.x + 14, top: hover.y + 14, zIndex: 50 }}
            className="pointer-events-none rounded border border-koma-border bg-koma-panel px-2 py-1 text-[11px] text-koma-fg shadow-lg"
          >
            <div className="font-mono text-koma-dim">{hover.commit.sha.slice(0, 7)}</div>
            <div className="max-w-[220px] truncate">{hover.commit.author}</div>
            <div>
              <span className="text-koma-success">+{hover.commit.added}</span>{' '}
              <span className="text-koma-error">-{hover.commit.deleted}</span>
            </div>
            <div className="text-koma-dim opacity-70">{formatDate(hover.commit.date)}</div>
          </div>
        )}
      </div>
    </div>
  )
}

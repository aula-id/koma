import { memo } from 'react'
import { motion } from 'framer-motion'
import type { GraphRow as GraphRowData, GraphSegment } from '../lib/gitGraphLayout'
import type { GitRef } from '../store/koma'

// ---- Render geometry (shared with GraphTab's virtualization math) -----------
export const ROW_H = 28
export const LANE_W = 15
const NODE_R = 4.5
// Cap the drawn gutter width so a pathologically wide graph can't shove the text
// column off-screen; lanes past the cap are clamped to the last column (rare in
// practice) — the svg is overflow-visible, so an unclamped x would otherwise
// bleed into the subject/author column instead of stacking at the gutter edge.
const MAX_GUTTER_LANES = 14

export function gutterWidth(laneCount: number): number {
  return Math.min(Math.max(laneCount, 1), MAX_GUTTER_LANES) * LANE_W
}

function laneX(lane: number): number {
  const clamped = Math.min(lane, MAX_GUTTER_LANES - 1)
  return clamped * LANE_W + LANE_W / 2
}

// Compact relative time from an ISO date (git `%aI`). Absolute date lives in the
// detail pane; the list shows this terse form.
export function relTime(iso: string): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ''
  const s = Math.max(0, (Date.now() - t) / 1000)
  if (s < 60) return 'just now'
  const m = s / 60
  if (m < 60) return `${Math.floor(m)}m ago`
  const h = m / 60
  if (h < 24) return `${Math.floor(h)}h ago`
  const d = h / 24
  if (d < 30) return `${Math.floor(d)}d ago`
  const mo = d / 30
  if (mo < 12) return `${Math.floor(mo)}mo ago`
  return `${Math.floor(d / 365)}y ago`
}

// One SVG path for a segment: straight vertical when it stays in its lane, a
// smooth S-curve (bezier easing horizontally at the band mid-line) for a
// branch/merge that changes lane.
function segPath(s: GraphSegment): string {
  const x1 = laneX(s.x1)
  const x2 = laneX(s.x2)
  const y1 = s.y1 * ROW_H
  const y2 = s.y2 * ROW_H
  if (s.x1 === s.x2) return `M${x1} ${y1} L${x2} ${y2}`
  const my = (y1 + y2) / 2
  return `M${x1} ${y1} C ${x1} ${my} ${x2} ${my} ${x2} ${y2}`
}

// Ref/tag chip tone per kind — these ARE graph data (a meaningful distinct hue
// per kind, like the lane palette): HEAD = accent, local = info(blue), remote =
// warn(amber), tag = success(green). Falls back to a neutral border.
const REF_TONE: Record<GitRef['kind'], string> = {
  head: 'border-koma-accent/70 text-koma-accent',
  local: 'border-koma-info/60 text-koma-info',
  remote: 'border-koma-warn/60 text-koma-warn',
  tag: 'border-koma-success/60 text-koma-success',
}

function RefChip({ r }: { r: GitRef }) {
  const tone = REF_TONE[r.kind] ?? 'border-koma-border text-koma-dim'
  return (
    <span
      title={`${r.kind}: ${r.name}`}
      className={`flex-none truncate rounded-sm border bg-koma-bg/40 px-1 text-[10px] leading-[15px] ${tone}`}
    >
      {r.name}
    </span>
  )
}

type Props = {
  row: GraphRowData
  laneCount: number
  isHead: boolean
  selected: boolean
  // `true` when ANOTHER row is hovered — this row's gutter dims to raise the
  // hovered one (the hovered row is never dimmed).
  dim: boolean
  // First-appearance flag: fade+slide in only the first time a row is seen
  // (initial load / load-more / scroll-in), never re-animate on every scroll.
  animate: boolean
  // Index within the visible window — drives the initial-load stagger.
  staggerIndex: number
  onSelect: (sha: string) => void
  onHover: (sha: string | null) => void
}

// One virtualized commit row: the SVG lane gutter (edges + node, HEAD ringed) +
// ref chips + subject + right-aligned author/relative-date/short-sha. Memoized —
// the parent passes stable sha-keyed callbacks, so a row only re-renders when its
// own selected/dim/animate state changes.
export const GraphRow = memo(function GraphRow({
  row,
  laneCount,
  isHead,
  selected,
  dim,
  animate,
  staggerIndex,
  onSelect,
  onHover,
}: Props) {
  const gw = gutterWidth(laneCount)
  const cx = laneX(row.lane)
  const cy = ROW_H / 2

  return (
    <motion.div
      onClick={() => onSelect(row.sha)}
      onHoverStart={() => onHover(row.sha)}
      onHoverEnd={() => onHover(null)}
      initial={animate ? { opacity: 0, x: -8 } : false}
      animate={{ opacity: 1, x: 0 }}
      transition={{
        duration: 0.16,
        ease: 'easeOut',
        delay: animate ? Math.min(staggerIndex * 0.01, 0.14) : 0,
      }}
      style={{ height: ROW_H }}
      className={`group flex cursor-pointer items-center ${
        selected ? 'bg-koma-hover' : 'hover:bg-koma-hover'
      }`}
    >
      <svg
        width={gw}
        height={ROW_H}
        className="flex-none overflow-visible"
        style={{ opacity: dim ? 0.35 : 1 }}
      >
        {row.segments.map((s, i) => (
          <path
            key={i}
            d={segPath(s)}
            fill="none"
            stroke={s.color}
            strokeWidth={selected ? 2 : 1.5}
            strokeLinecap="round"
            opacity={0.9}
          />
        ))}
        {/* HEAD gets an outer ring for emphasis. */}
        {isHead && (
          <circle cx={cx} cy={cy} r={NODE_R + 2.5} fill="none" stroke={row.color} strokeWidth={1.5} opacity={0.9} />
        )}
        <circle cx={cx} cy={cy} r={NODE_R} fill={row.color} stroke="var(--color-koma-bg)" strokeWidth={1.5} />
      </svg>
      <div className="flex min-w-0 flex-1 items-center gap-1.5 pr-3">
        {row.refs.map((r, i) => (
          <RefChip key={`${r.kind}:${r.name}:${i}`} r={r} />
        ))}
        <span className="min-w-0 flex-1 truncate text-[12px] text-koma-fg">{row.commit.subject}</span>
        <span className="hidden max-w-[130px] flex-none truncate text-[11px] text-koma-dim md:inline">
          {row.commit.author}
        </span>
        <span className="flex-none text-[11px] text-koma-dim opacity-70">{relTime(row.commit.date)}</span>
        <span className="flex-none font-mono text-[11px] text-koma-dim opacity-55">{row.sha.slice(0, 7)}</span>
      </div>
    </motion.div>
  )
})

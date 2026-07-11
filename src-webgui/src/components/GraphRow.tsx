import { memo } from 'react'
import type { MouseEvent as ReactMouseEvent, DragEvent as ReactDragEvent } from 'react'
import { motion } from 'framer-motion'
import type { GraphRow as GraphRowData, GraphSegment } from '../lib/gitGraphLayout'
import type { GitRef } from '../store/koma'
import { AuthorAvatar } from './AuthorAvatar'

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
  head: 'border-koma-accent/70 bg-koma-accent/15 text-koma-accent',
  local: 'border-koma-info/60 bg-koma-info/20 text-koma-fg',
  remote: 'border-koma-warn/60 bg-koma-warn/20 text-koma-fg',
  tag: 'border-koma-success/60 bg-koma-success/20 text-koma-fg',
}

function RefChip({
  r,
  onContextMenu,
  draggedBranch,
  dropHighlight,
  onBranchDragStart,
  onBranchDragEnd,
  onDropHover,
  onRefDrop,
}: {
  r: GitRef
  // Right-click on a BRANCH chip (local/remote — never a tag, which isn't
  // switchable) opens the graph context menu's "Checkout <branch>"/"Copy
  // branch name" mode. `undefined` for a tag chip, so it falls through to the
  // row's own commit context menu instead. `kind` is threaded through so the
  // menu knows whether the chip is a remote ref (needs the DWIM tracking-branch
  // short-name strip before checkout) vs local/head (checked out as-is).
  onContextMenu?: (e: ReactMouseEvent, name: string, kind: GitRef['kind']) => void
  // ---- GitKraken-style drag-to-rebase (G6) ----
  // The branch name currently being dragged ANYWHERE in the graph (lifted to
  // GraphTab so every chip/row can react to it), or null when nothing is
  // dragging. Gates whether THIS chip shows drop-hover affordance at all.
  draggedBranch: string | null
  // True while this exact chip is the drop target currently under the drag —
  // drives the highlight ring (the main drop affordance).
  dropHighlight: boolean
  onBranchDragStart: (name: string) => void
  onBranchDragEnd: () => void
  onDropHover: (id: string | null) => void
  onRefDrop: (e: ReactDragEvent, name: string) => void
}) {
  const tone = REF_TONE[r.kind] ?? 'border-koma-border bg-koma-head text-koma-dim'
  // Only a local/HEAD ref can be the DRAG SOURCE of a rebase — you can't
  // rebase a remote-tracking ref (it only ever moves via fetch); every kind of
  // chip (incl. remote/tag) is still a valid DROP TARGET (rebasing onto
  // `origin/main` or a tag is a legitimate distinct target — see the module's
  // "never remote-strip a rebase target" reasoning, matches G5c's git_merge/
  // git_rebase callers).
  const draggableChip = r.kind === 'local' || r.kind === 'head'
  return (
    <span
      title={`${r.kind}: ${r.name}`}
      draggable={draggableChip}
      onDragStart={
        draggableChip
          ? (e) => {
              e.stopPropagation()
              e.dataTransfer.setData('text/koma-branch', r.name)
              e.dataTransfer.effectAllowed = 'move'
              onBranchDragStart(r.name)
            }
          : undefined
      }
      // dragend ALWAYS fires (successful drop or cancelled drag) — the
      // authoritative point that clears the host-wide drag state.
      onDragEnd={draggableChip ? () => onBranchDragEnd() : undefined}
      onDragOver={
        draggedBranch
          ? (e) => {
              e.preventDefault()
              e.stopPropagation()
              onDropHover(`ref:${r.name}`)
            }
          : undefined
      }
      onDragLeave={
        draggedBranch
          ? (e) => {
              e.stopPropagation()
              onDropHover(null)
            }
          : undefined
      }
      onDrop={
        draggedBranch
          ? (e) => {
              e.preventDefault()
              e.stopPropagation()
              onRefDrop(e, r.name)
            }
          : undefined
      }
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault()
              e.stopPropagation()
              onContextMenu(e, r.name, r.kind)
            }
          : undefined
      }
      className={`flex-none truncate rounded-sm border px-1 text-[10px] leading-[15px] transition-shadow ${tone} ${
        draggableChip ? 'cursor-grab active:cursor-grabbing' : ''
      } ${dropHighlight ? 'ring-1 ring-koma-accent bg-koma-accent/10' : ''}`}
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
  // Right-click on the row (not a ref chip) opens the graph context menu's
  // commit mode ("Checkout commit"/"Create branch here…"/"Copy SHA").
  onContextMenu?: (e: ReactMouseEvent, sha: string) => void
  // Right-click on a branch ref chip opens the context menu's ref mode
  // instead — see `RefChip`'s own `onContextMenu` doc.
  onRefContextMenu?: (e: ReactMouseEvent, name: string, kind: GitRef['kind']) => void
  // ---- GitKraken-style drag-to-rebase (G6) ----
  // The branch name currently being dragged anywhere in the graph, lifted to
  // GraphTab (a single host-wide drag can span many virtualized rows) — null
  // when nothing is dragging.
  draggedBranch: string | null
  // The drop-target id (`commit:<sha>` for this row, `ref:<name>` for one of
  // its chips) currently hovered by the drag, or null. Compared against this
  // row's own ids to decide which highlight (if any) to show.
  dropHoverId: string | null
  onBranchDragStart: (name: string) => void
  onBranchDragEnd: () => void
  onDropHover: (id: string | null) => void
  // A valid (non-no-op) drop landed: `branch` was dragged onto `target` (a sha
  // or a ref name, unstripped) labelled `targetLabel`, at client point (x, y).
  // GraphTab turns this into the confirm popover — see the module doc for the
  // no-op rules (dropped on the branch's own tip / onto itself).
  onRebaseDrop: (branch: string, target: string, targetLabel: string, x: number, y: number) => void
}

// One virtualized commit row: the SVG lane gutter (edges + node, HEAD ringed) +
// ref chips + subject + right-aligned author/relative-date/short-sha. Memoized —
// the parent passes stable sha-keyed callbacks, so a row only re-renders when its
// own selected/dim/animate/drag-related state changes.
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
  onContextMenu,
  onRefContextMenu,
  draggedBranch,
  dropHoverId,
  onBranchDragStart,
  onBranchDragEnd,
  onDropHover,
  onRebaseDrop,
}: Props) {
  const gw = gutterWidth(laneCount)
  const cx = laneX(row.lane)
  const cy = ROW_H / 2
  const rowDropId = `commit:${row.sha}`
  const rowDropHighlight = dropHoverId === rowDropId

  // Drop on the commit row itself: target = this commit's sha. A no-op when
  // the dragged branch's own tip is ALREADY this commit (dropping a branch
  // onto its current position) — silently ignored, no confirm raised.
  const handleRowDrop = (e: ReactDragEvent) => {
    e.preventDefault()
    e.stopPropagation()
    const branch = draggedBranch
    onDropHover(null)
    if (!branch) return
    const isOwnTip = row.refs.some((r) => (r.kind === 'local' || r.kind === 'head') && r.name === branch)
    if (!isOwnTip) {
      onRebaseDrop(branch, row.sha, `commit ${row.sha.slice(0, 7)}`, e.clientX, e.clientY)
    }
    onBranchDragEnd()
  }

  // Drop on a ref chip: target = that ref's name AS-IS (never remote-stripped
  // — rebasing onto `origin/main` is a distinct valid target, per the G5c
  // fix). A no-op when the target IS the dragged branch itself.
  const handleRefDrop = (e: ReactDragEvent, name: string) => {
    const branch = draggedBranch
    onDropHover(null)
    if (!branch) return
    if (name !== branch) {
      onRebaseDrop(branch, name, name, e.clientX, e.clientY)
    }
    onBranchDragEnd()
  }

  return (
    <motion.div
      onClick={() => {
        // A drag-drop landing on the row must never also fire row select —
        // guarded by the drag state rather than relying on click suppression.
        if (!draggedBranch) onSelect(row.sha)
      }}
      onHoverStart={() => onHover(row.sha)}
      onHoverEnd={() => onHover(null)}
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault()
              onContextMenu(e, row.sha)
            }
          : undefined
      }
      onDragOver={
        draggedBranch
          ? (e) => {
              e.preventDefault()
              onDropHover(rowDropId)
            }
          : undefined
      }
      onDragLeave={draggedBranch ? () => onDropHover(null) : undefined}
      onDrop={draggedBranch ? handleRowDrop : undefined}
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
      } ${rowDropHighlight ? 'ring-1 ring-inset ring-koma-accent' : ''}`}
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
          <RefChip
            key={`${r.kind}:${r.name}:${i}`}
            r={r}
            // A tag isn't switchable — its chip falls through to the row's
            // own commit context menu instead of opening a ref-mode menu.
            onContextMenu={r.kind === 'tag' ? undefined : onRefContextMenu}
            draggedBranch={draggedBranch}
            dropHighlight={dropHoverId === `ref:${r.name}`}
            onBranchDragStart={onBranchDragStart}
            onBranchDragEnd={onBranchDragEnd}
            onDropHover={onDropHover}
            onRefDrop={handleRefDrop}
          />
        ))}
        <span className="min-w-0 flex-1 truncate text-[12px] text-koma-fg">{row.commit.subject}</span>
        <span className="hidden max-w-[140px] flex-none items-center gap-1 md:flex">
          <AuthorAvatar name={row.commit.author} email={row.commit.email} />
          <span className="min-w-0 truncate text-[11px] text-koma-dim">{row.commit.author}</span>
        </span>
        <span className="flex-none text-[11px] text-koma-dim opacity-70">{relTime(row.commit.date)}</span>
        <span className="flex-none font-mono text-[11px] text-koma-dim opacity-55">{row.sha.slice(0, 7)}</span>
      </div>
    </motion.div>
  )
})

import { useEffect, useMemo } from 'react'
import { Loader2 } from 'lucide-react'
import { useKoma } from '../store/koma'
import { computeGitGraph } from '../lib/gitGraphLayout'
import type { GraphRow as GraphRowData, GraphSegment } from '../lib/gitGraphLayout'

// Compact geometry — deliberately NOT GraphRow's ROW_H/LANE_W (those are sized
// for the full detailed tab); this is a narrow sidepanel section, so both are
// shrunk and the gutter caps at fewer lanes before clamping.
const ROW_H = 20
const LANE_W = 10
const NODE_R = 3
const MAX_GUTTER_LANES = 6
// Only the first page's leading rows — this is a "standard" glance view, not
// the virtualized full graph; no windowing needed at this size.
const MINI_MAX = 40

function laneX(lane: number): number {
  const clamped = Math.min(lane, MAX_GUTTER_LANES - 1)
  return clamped * LANE_W + LANE_W / 2
}

function segPath(s: GraphSegment): string {
  const x1 = laneX(s.x1)
  const x2 = laneX(s.x2)
  const y1 = s.y1 * ROW_H
  const y2 = s.y2 * ROW_H
  if (s.x1 === s.x2) return `M${x1} ${y1} L${x2} ${y2}`
  const my = (y1 + y2) / 2
  return `M${x1} ${y1} C ${x1} ${my} ${x2} ${my} ${x2} ${y2}`
}

// Local lane width for JUST the rows actually shown (not the whole loaded
// page) — keeps the gutter tight when the visible slice happens to be
// narrower than the full loaded graph, still clamped to MAX_GUTTER_LANES.
function miniLaneCount(rows: GraphRowData[]): number {
  let max = 0
  for (const r of rows) {
    max = Math.max(max, r.lane)
    for (const s of r.segments) max = Math.max(max, s.x1, s.x2)
  }
  return Math.min(Math.max(max + 1, 1), MAX_GUTTER_LANES)
}

type RowProps = {
  row: GraphRowData
  gutterW: number
  isHead: boolean
  selected: boolean
  onSelect: (sha: string) => void
}

function MiniRow({ row, gutterW, isHead, selected, onSelect }: RowProps) {
  const cx = laneX(row.lane)
  const cy = ROW_H / 2
  const headRef = row.refs.find((r) => r.kind === 'head')
  const otherRef = row.refs.find((r) => r.kind !== 'head')

  return (
    <div
      onClick={() => onSelect(row.sha)}
      style={{ height: ROW_H }}
      className={`group flex cursor-pointer items-center gap-1 px-1.5 ${
        selected ? 'bg-koma-hover' : 'hover:bg-koma-hover'
      }`}
    >
      <svg width={gutterW} height={ROW_H} className="flex-none overflow-visible">
        {row.segments.map((s, i) => (
          <path
            key={i}
            d={segPath(s)}
            fill="none"
            stroke={s.color}
            strokeWidth={selected ? 1.75 : 1.25}
            strokeLinecap="round"
            opacity={0.9}
          />
        ))}
        {isHead && (
          <circle cx={cx} cy={cy} r={NODE_R + 2} fill="none" stroke={row.color} strokeWidth={1.25} opacity={0.9} />
        )}
        <circle cx={cx} cy={cy} r={NODE_R} fill={row.color} stroke="var(--color-koma-bg)" strokeWidth={1.25} />
      </svg>
      <div className="flex min-w-0 flex-1 items-center gap-1">
        {headRef && (
          <span
            title={headRef.name}
            className="flex-none truncate rounded-sm border border-koma-accent/70 bg-koma-accent/15 px-1 text-[9px] leading-[14px] text-koma-accent"
          >
            {headRef.name}
          </span>
        )}
        {otherRef && (
          <span
            title={otherRef.name}
            className="hidden flex-none truncate rounded-sm border border-koma-border bg-koma-head px-1 text-[9px] leading-[14px] text-koma-dim sm:inline-block"
          >
            {otherRef.name}
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-[11px] text-koma-fg">{row.commit.subject}</span>
        <span className="flex-none font-mono text-[10px] text-koma-dim opacity-55">{row.sha.slice(0, 7)}</span>
      </div>
    </div>
  )
}

// Compact commit graph for the Source Control sidepanel (G3) — reuses the SAME
// `graph` store slice + `computeGitGraph` layout engine as the full GraphTab
// (no second fetch path, no duplicated lane logic). Shows the leading
// MINI_MAX rows only; "Explore" (GitPanel's header/section action) jumps to
// the full, detailed, virtualized tab for anything deeper. Lazily fetches:
// only fires `refreshGraph()` if the slice is still empty AND not already
// loading, so it never double-fetches against a graph the tab already
// populated, and — since this only mounts while its AccordionSection is
// expanded — never fetches until the user actually opens the section.
export function GitGraphMini() {
  const commits = useKoma((s) => s.graph.commits)
  const head = useKoma((s) => s.graph.head)
  const loading = useKoma((s) => s.graph.loading)
  const selectedSha = useKoma((s) => s.graph.selectedSha)
  const refreshGraph = useKoma((s) => s.refreshGraph)
  const openGraphTab = useKoma((s) => s.openGraphTab)
  const selectCommit = useKoma((s) => s.selectCommit)
  const sessionId = useKoma((s) => s.session.id)

  // Keyed on `sessionId`: this mounts while the "Commit Graph" accordion is
  // open and stays mounted across a session switch, so a plain "only fetch
  // when empty" guard would never refetch (the OLD session's commits are
  // still sitting in the slice). `refreshGraph` already serializes via its
  // own `pendingRefresh` flag, so re-firing on every session change is safe.
  useEffect(() => {
    refreshGraph()
  }, [refreshGraph, sessionId])

  const rows = useMemo(() => computeGitGraph(commits).rows.slice(0, MINI_MAX), [commits])
  const gutterW = useMemo(() => miniLaneCount(rows) * LANE_W, [rows])

  const onSelect = (sha: string) => {
    openGraphTab()
    selectCommit(sha)
  }

  if (rows.length === 0) {
    return (
      <div className="flex h-16 w-full items-center justify-center text-[11px] text-koma-dim opacity-60">
        {loading ? <Loader2 size={14} className="animate-spin opacity-70" /> : 'No commits to display.'}
      </div>
    )
  }

  return (
    <div className="flex flex-col py-0.5">
      {rows.map((row) => (
        <MiniRow
          key={row.sha}
          row={row}
          gutterW={gutterW}
          isHead={row.sha === head}
          selected={row.sha === selectedSha}
          onSelect={onSelect}
        />
      ))}
    </div>
  )
}

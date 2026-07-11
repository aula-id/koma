// Pure, React-free GitKraken-style commit-graph lane layout (G2). Turns the
// host's newest-first `GitCommitNode[]` (already `--date-order --parents`) into
// a per-row structure the SVG gutter draws directly. No side effects, no imports
// beyond the wire types — trivially testable (see `__selftest`).

import type { GitCommitNode, GitRef } from '../store/koma'

// Lane colour palette — the ONE place a hardcoded colour list is legitimate:
// this is graph DATA (which branch owns which column), not app chrome. Ten
// distinct hues at a mid saturation/lightness (Tailwind-500-ish) chosen to read
// on BOTH the light and dark koma themes. Colour is stable PER LANE INDEX
// (`laneColor(i) = LANE_COLORS[i % n]`), so a branch keeps its hue for its whole
// life in a column.
export const LANE_COLORS = [
  '#4f9dff', // blue
  '#f59e0b', // amber
  '#22c55e', // green
  '#c084fc', // purple
  '#ef4444', // red
  '#14b8a6', // teal
  '#ec4899', // pink
  '#84cc16', // lime
  '#f97316', // orange
  '#38bdf8', // sky
] as const

export function laneColor(lane: number): string {
  const n = LANE_COLORS.length
  return LANE_COLORS[((lane % n) + n) % n]
}

// One drawable segment of a lane through a single row's BAND, in a normalized
// coordinate system the renderer scales to pixels:
//   x = lane INDEX      (renderer: laneX(x) = x * LANE_W + LANE_W / 2)
//   y = vertical position within the row: 0 = top edge, 0.5 = node-centre line,
//       1 = bottom edge (renderer: y * ROW_H)
// x1 === x2 is a straight vertical; x1 !== x2 is a branch/merge curve the
// renderer draws as a bezier. `color` is the segment's own lane hue.
export type GraphSegment = {
  x1: number
  y1: number
  x2: number
  y2: number
  color: string
}

// One laid-out commit row. `lane`/`color` place + tint the node; `segments` are
// this row's SELF-CONTAINED band (upper-half incoming verticals + lower-half
// pass-through continuations + this commit's out-edges to its parents), so
// adjacent rows align edge-to-edge with no shared/global SVG — ideal for
// virtualization (each visible row draws its own height-ROW_H SVG). `refs`/
// `commit` pass through for the renderer's chips + metadata.
export type GraphRow = {
  sha: string
  lane: number
  color: string
  refs: GitRef[]
  commit: GitCommitNode
  segments: GraphSegment[]
}

export type GitGraphLayout = {
  rows: GraphRow[]
  // The widest the graph ever gets (max lane index + 1) — the gutter must be
  // sized for this so every row's SVG shares one coordinate system.
  laneCount: number
}

// Leftmost free (null) lane, or the append index when every lane is occupied.
function firstFreeLane(lanes: (string | null)[]): number {
  const i = lanes.indexOf(null)
  return i === -1 ? lanes.length : i
}

// Compute the lane layout for `commits` (newest-first). Pure + STATELESS over the
// FULL array — the store re-runs this on the whole (concatenated) list after a
// load-more append, so a dangling edge (a parent not yet loaded) reconnects
// automatically once that parent's commit arrives.
export function computeGitGraph(commits: GitCommitNode[]): GitGraphLayout {
  // lanes[i] = the sha lane `i` is currently WAITING for (its next expected
  // commit, walking downward), or null when the lane is free. Invariant: no sha
  // ever appears twice — we always CONVERGE onto an existing waiting lane before
  // allocating a new one, so a branch-point duplicate never forms in the first
  // place (this is the "keep one, free the duplicates" rule, applied at assign
  // time rather than as a cleanup pass).
  const lanes: (string | null)[] = []
  const rows: GraphRow[] = []
  let maxLane = 0

  for (const c of commits) {
    // Snapshot the lane state ENTERING this row (== the previous row's exit
    // state) for the upper-half incoming verticals + the pass-through test.
    const laneIn = lanes.slice()

    // 1. This commit's lane: the lane already waiting for it (an in-view child
    //    fed it), else the leftmost free lane (a tip with no loaded child).
    let myLane = lanes.indexOf(c.sha)
    if (myLane === -1) myLane = firstFreeLane(lanes)
    while (lanes.length <= myLane) lanes.push(null)
    // The commit CONSUMES its lane; its first parent may re-take it just below.
    lanes[myLane] = null

    // 2. Route each parent to a lane. A parent SOME lane already waits for
    //    CONVERGES onto that lane (a branch/merge reconnecting — no duplicate).
    //    Otherwise the FIRST parent continues straight in `myLane`; an ADDITIONAL
    //    parent (a merge — incl. octopus, >2 parents) opens the leftmost free
    //    lane. A root commit (no parents) leaves `myLane` freed to null, so the
    //    lane ends here and is reusable by a later branch (keeps the graph tight).
    const outTargets: number[] = []
    c.parents.forEach((p, k) => {
      let j = lanes.indexOf(p)
      if (j === -1) {
        j = k === 0 ? myLane : firstFreeLane(lanes)
        while (lanes.length <= j) lanes.push(null)
        lanes[j] = p
      }
      outTargets.push(j)
    })

    // 3. Emit this row's band segments (local y: 0 top, 0.5 node centre, 1 bottom).
    const segments: GraphSegment[] = []
    // 3a. Upper half — a straight vertical (top edge -> node-centre line) for
    //     every lane active ENTERING the row: the incoming edge into the node
    //     (at `myLane`, when a child fed it) plus every pass-through lane. A tip's
    //     `myLane` is null in `laneIn`, so a tip correctly draws no incoming edge.
    for (let j = 0; j < laneIn.length; j++) {
      if (laneIn[j] != null) segments.push({ x1: j, y1: 0, x2: j, y2: 0.5, color: laneColor(j) })
    }
    // 3b. Lower half — pass-through continuations: a lane that existed above and
    //     continues below UNCHANGED (same sha, and not the commit's own lane)
    //     runs straight from the node-centre line to the bottom edge. A lane that
    //     is BOTH a pass-through AND a merge target gets this straight PLUS the
    //     merge curve from 3c — that's how both children's lines meet at a shared
    //     parent (e.g. the far side of a diamond).
    for (let j = 0; j < lanes.length; j++) {
      if (lanes[j] != null && laneIn[j] === lanes[j] && j !== myLane) {
        segments.push({ x1: j, y1: 0.5, x2: j, y2: 1, color: laneColor(j) })
      }
    }
    // 3c. Lower half — this commit's out-edges: from the node centre to each
    //     parent's lane at the bottom edge (a straight line when the parent kept
    //     `myLane`, a curve when it branched/merged to a different lane). Coloured
    //     by the TARGET (parent) lane so a branch keeps its hue past the fork.
    for (const j of outTargets) {
      segments.push({ x1: myLane, y1: 0.5, x2: j, y2: 1, color: laneColor(j) })
    }

    for (const s of segments) maxLane = Math.max(maxLane, s.x1, s.x2)
    maxLane = Math.max(maxLane, myLane)

    rows.push({
      sha: c.sha,
      lane: myLane,
      color: laneColor(myLane),
      refs: c.refs,
      commit: c,
      segments,
    })

    // Compact: drop trailing free lanes so the live width tracks the graph's
    // ACTUAL current breadth (freed tip/merge lanes get reused, never leak
    // unbounded as history flows past a wide merge).
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop()
  }

  return { rows, laneCount: maxLane + 1 }
}

// ---- Self-test (no test runner is wired in this package — see the G2 task
// note; do NOT add one). Call from a scratch/dev context and inspect the result
// array; every `ok` must be true. Kept as a plain exported function, zero deps.
export function __selftest(): { name: string; ok: boolean; msg?: string }[] {
  const mk = (sha: string, parents: string[]): GitCommitNode => ({
    sha,
    parents,
    refs: [],
    author: 'a',
    email: 'e',
    date: '2026-01-01T00:00:00Z',
    subject: sha,
  })
  const results: { name: string; ok: boolean; msg?: string }[] = []
  const check = (name: string, ok: boolean, msg?: string) => results.push({ name, ok, msg })

  // 1. Linear history A -> B -> C: one lane, three nodes all on lane 0, no curves.
  {
    const { rows, laneCount } = computeGitGraph([mk('A', ['B']), mk('B', ['C']), mk('C', [])])
    check('linear: 3 rows', rows.length === 3)
    check('linear: single lane', laneCount === 1, `laneCount=${laneCount}`)
    check('linear: all nodes on lane 0', rows.every((r) => r.lane === 0))
    check('linear: no curves', rows.every((r) => r.segments.every((s) => s.x1 === s.x2)))
  }

  // 2. Branch + merge (diamond): A merges B (first parent) and C; B and C share
  //    parent D. Expect A=lane0, B=lane0, C=lane1, D=lane0; A branches to lane1,
  //    C merges back to lane0.
  {
    const { rows, laneCount } = computeGitGraph([
      mk('A', ['B', 'C']),
      mk('B', ['D']),
      mk('C', ['D']),
      mk('D', []),
    ])
    const by: Record<string, GraphRow> = Object.fromEntries(
      rows.map((r): [string, GraphRow] => [r.sha, r]),
    )
    check('diamond: 4 rows', rows.length === 4)
    check('diamond: two lanes', laneCount === 2, `laneCount=${laneCount}`)
    check('diamond: C on lane 1', by['C']?.lane === 1, `C lane=${by['C']?.lane}`)
    check('diamond: D back on lane 0', by['D']?.lane === 0, `D lane=${by['D']?.lane}`)
    check(
      'diamond: A branches (curve lane0 -> lane1)',
      !!by['A']?.segments.some((s) => s.x1 === 0 && s.x2 === 1 && s.y1 === 0.5 && s.y2 === 1),
    )
    check(
      'diamond: C merges (curve lane1 -> lane0)',
      !!by['C']?.segments.some((s) => s.x1 === 1 && s.x2 === 0 && s.y2 === 1),
    )
  }

  // 3. Dangling parent: A's parent X isn't in the loaded page — A's out-edge
  //    still runs to the bottom edge (reconnects when X loads later).
  {
    const { rows, laneCount } = computeGitGraph([mk('A', ['X'])])
    check('dangling: 1 row', rows.length === 1)
    check('dangling: single lane', laneCount === 1)
    check('dangling: edge exits bottom', !!rows[0]?.segments.some((s) => s.y2 === 1 && s.x1 === 0))
  }

  // 4. Octopus merge (>2 parents): M with parents P1,P2,P3 fans out to 3 lanes.
  {
    const { rows, laneCount } = computeGitGraph([
      mk('M', ['P1', 'P2', 'P3']),
      mk('P1', []),
      mk('P2', []),
      mk('P3', []),
    ])
    const m = rows.find((r) => r.sha === 'M')
    const outs = m ? m.segments.filter((s) => s.y1 === 0.5 && s.y2 === 1 && s.x1 === m.lane) : []
    check('octopus: 3 parent edges', outs.length === 3, `outs=${outs.length}`)
    check('octopus: lanes >= 3', laneCount >= 3, `laneCount=${laneCount}`)
  }

  return results
}

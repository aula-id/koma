// VSCode-style split view (editor groups) for the main tab column.
//
// The model is deliberately FLAT: `groups` is an ordered list of group ids laid
// out along ONE axis (`splitDir`), not VSCode's recursive grid. At most TWO
// panes (side-by-side or stacked) — orientation flips in place via setSplitDir.
// A flat list keeps every invariant checkable in a single pass.
//
// Membership lives in `tabGroup` (tab id -> group id) rather than on the Tab
// objects themselves, so the ~20 `open*Tab` actions keep appending to `ui.tabs`
// without knowing groups exist: group actions and rendering selectors call
// `normalizeGroups`, which stamps anything unassigned into the focused group.
// That normalizer is the ONLY place membership and focus are repaired — which
// also means the ad-hoc tab-pruning paths (agent deleted, extension uninstalled,
// file deleted, session detached) need no group bookkeeping of their own.
//
// Nothing here imports the store: the store owns the state, this module owns the
// rules, and editorGroups.test.ts exercises them on plain objects.

export type EditorGroupId = string

/** Layout axis: 'row' = groups side by side, 'col' = groups stacked. */
export type SplitDir = 'row' | 'col'

/** The group every tab starts in — the "no split" state is `groups: [DEFAULT_GROUP]`. */
export const DEFAULT_GROUP: EditorGroupId = 'g0'

// Two panes is the product ceiling: one split, one axis, one toggle. Nested
// grids and three-way layouts are out of scope (and the drop overlay refuses
// edge zones once a split already exists).
export const MAX_GROUPS = 2

/** Width of the draggable divider between two groups, in px. */
export const GRIP_PX = 5

// No group may be dragged below this share of the axis, so a pane can never be
// resized into an unclickable sliver.
const MIN_FRACTION = 0.12

/** The slice of `ui` this module owns. `tabs` is read-only here — only order and ids matter. */
export type GroupLayout = {
  readonly tabs: readonly { readonly id: string }[]
  activeTabId: string
  groups: EditorGroupId[]
  tabGroup: Record<string, EditorGroupId>
  groupActive: Record<EditorGroupId, string>
  activeGroupId: EditorGroupId
  splitDir: SplitDir
  groupSizes: Record<EditorGroupId, number>
}

/** Ordered tab ids in one group — tab-strip paint order, taken from `tabs`. */
export function groupTabIds(ui: GroupLayout, groupId: EditorGroupId): string[] {
  const out: string[] = []
  for (const t of ui.tabs) {
    if ((ui.tabGroup[t.id] ?? ui.activeGroupId) === groupId) out.push(t.id)
  }
  return out
}

/** The group a tab lives in (the focused group, for a tab not yet stamped). */
export function groupOf(ui: GroupLayout, tabId: string): EditorGroupId {
  return ui.tabGroup[tabId] ?? ui.activeGroupId
}

/**
 * Whether a tab's content is on screen — i.e. it's the active tab OF ITS OWN
 * group, which with a split is no longer the same thing as `activeTabId`.
 */
export function isTabVisible(ui: GroupLayout, tabId: string): boolean {
  return ui.groupActive[groupOf(ui, tabId)] === tabId
}

/** Mint an id no live group is using. Ids are opaque; reuse after a collapse is fine. */
export function nextGroupId(groups: readonly EditorGroupId[]): EditorGroupId {
  let max = -1
  for (const g of groups) {
    const n = /^g(\d+)$/.exec(g)
    if (n) max = Math.max(max, Number(n[1]))
  }
  return `g${max + 1}`
}

/**
 * Re-establish every group invariant. Cheap and identity-stable: returns the
 * SAME object when nothing needed fixing, so it can run on every commit.
 *
 * In order: drop duplicate/empty groups, resolve each tab's membership (an
 * unassigned tab joins the focused group), give every group a live active tab,
 * and finally let `activeTabId` drive which group is focused — that last rule is
 * what makes plain `activateTab`/`open*Tab` calls move focus to the right pane
 * without knowing anything about groups.
 */
export function normalizeGroups<S extends GroupLayout>(ui: S): S {
  const groups = ui.groups.filter((g, i) => ui.groups.indexOf(g) === i).slice(0, MAX_GROUPS)
  if (groups.length === 0) groups.push(DEFAULT_GROUP)
  const known = new Set(groups)
  // Tabs are stamped into whichever group is focused; an activeGroupId that no
  // longer exists falls back to the first group.
  const fallback = known.has(ui.activeGroupId) ? ui.activeGroupId : groups[0]

  const tabGroup: Record<string, EditorGroupId> = {}
  const counts = new Map<EditorGroupId, number>()
  for (const t of ui.tabs) {
    const prev = ui.tabGroup[t.id]
    const g = prev !== undefined && known.has(prev) ? prev : fallback
    tabGroup[t.id] = g
    counts.set(g, (counts.get(g) ?? 0) + 1)
  }

  // A group with no tabs left (its last one was closed or dragged away) stops
  // existing — that's how a split collapses back to a single pane.
  const live = groups.filter((g) => (counts.get(g) ?? 0) > 0)
  if (live.length === 0) live.push(fallback)

  const activeGroupId = live.includes(fallback) ? fallback : live[0]

  const groupActive: Record<EditorGroupId, string> = {}
  for (const g of live) {
    const ids = ui.tabs.filter((t) => tabGroup[t.id] === g).map((t) => t.id)
    const held = ui.groupActive[g]
    groupActive[g] = held !== undefined && ids.includes(held) ? held : ids[ids.length - 1]
  }

  // `activeTabId` is the driver: clicking a tab, or opening one, focuses the
  // group that owns it. Only when it points at a tab that's gone does the
  // focused group's own active tab win instead.
  let nextActiveGroup = activeGroupId
  let activeTabId = ui.activeTabId
  if (tabGroup[activeTabId] !== undefined) {
    nextActiveGroup = tabGroup[activeTabId]
    groupActive[nextActiveGroup] = activeTabId
  } else {
    activeTabId = groupActive[activeGroupId] ?? activeTabId
  }

  // Single-pane collapse: drop the second group's size weight and reset the
  // survivor to 1. Leaving a post-resize weight (e.g. 0.3fr or 1.7fr) is
  // theoretically fine for one track, but WebKit/Edge have painted leftover
  // empty regions after 2→1 collapse when stale multi-track geometry lingered
  // alongside non-unit fr weights. Unit weight + explicit single-track layout
  // (see gridLayout) is the reliable unsplit state.
  const groupSizes: Record<EditorGroupId, number> = {}
  if (live.length === 1) {
    groupSizes[live[0]] = 1
  } else {
    for (const g of live) groupSizes[g] = ui.groupSizes[g] ?? 1
  }

  // Orientation only matters with ≥2 panes; pin back to the default axis so the
  // next split starts from a known row layout rather than a leftover 'col'.
  const splitDir: SplitDir = live.length < 2 ? 'row' : ui.splitDir

  const same =
    sameList(live, ui.groups) &&
    sameMap(tabGroup, ui.tabGroup) &&
    sameMap(groupActive, ui.groupActive) &&
    sameMap(groupSizes, ui.groupSizes) &&
    nextActiveGroup === ui.activeGroupId &&
    activeTabId === ui.activeTabId &&
    splitDir === ui.splitDir
  if (same) return ui

  return {
    ...ui,
    groups: live,
    tabGroup,
    groupActive,
    groupSizes,
    splitDir,
    activeGroupId: nextActiveGroup,
    activeTabId,
  }
}

function sameList(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i])
}

function sameMap<V>(a: Record<string, V>, b: Record<string, V>): boolean {
  const ka = Object.keys(a)
  if (ka.length !== Object.keys(b).length) return false
  return ka.every((k) => a[k] === b[k])
}

/**
 * Which tab to focus after `closingId` goes away: its left neighbour WITHIN THE
 * SAME group, else the right one, so closing a tab in one pane never yanks focus
 * into another. `null` when the group is about to empty out (and therefore
 * collapse) — the caller picks a global fallback.
 */
export function neighbourInGroup(ui: GroupLayout, closingId: string): string | null {
  const ids = groupTabIds(ui, groupOf(ui, closingId))
  const i = ids.indexOf(closingId)
  if (i < 0) return null
  return ids[i - 1] ?? ids[i + 1] ?? null
}

/**
 * Move `tabId` into `groupId`, positioned before `beforeId` (or last when null).
 * Returns the reordered flat tab list — the tab strips read their order from it.
 *
 * tabs[0] stays put no matter what: several call sites treat it as the permanent
 * chat tab, so it is never reordered and nothing is ever inserted ahead of it.
 */
export function reorderTab<T extends { id: string }>(
  tabs: readonly T[],
  tabId: string,
  beforeId: string | null,
): T[] {
  const from = tabs.findIndex((t) => t.id === tabId)
  if (from <= 0 || tabId === beforeId) return tabs.slice()
  const rest = tabs.filter((t) => t.id !== tabId)
  const at = beforeId === null ? rest.length : rest.findIndex((t) => t.id === beforeId)
  const clamped = at < 1 ? (beforeId === null ? rest.length : 1) : at
  rest.splice(clamped, 0, tabs[from])
  return rest
}

/**
 * Insert a new group next to `targetId` and return it. Only valid from a single
 * pane (MAX_GROUPS = 2). `dir` sets the global axis for that two-pane layout.
 */
export function insertGroup(
  ui: GroupLayout,
  targetId: EditorGroupId,
  side: 'before' | 'after',
  dir: SplitDir,
): { groups: EditorGroupId[]; groupSizes: Record<EditorGroupId, number>; splitDir: SplitDir; id: EditorGroupId } | null {
  if (ui.groups.length >= MAX_GROUPS) return null
  const at = ui.groups.indexOf(targetId)
  if (at < 0) return null
  const id = nextGroupId(ui.groups)
  const groups = ui.groups.slice()
  groups.splice(side === 'before' ? at : at + 1, 0, id)
  // The new pane opens at the average of the existing shares, i.e. an even
  // split of a fresh layout.
  const shares = ui.groups.map((g) => ui.groupSizes[g] ?? 1)
  const avg = shares.reduce((a, b) => a + b, 0) / (shares.length || 1)
  return { groups, groupSizes: { ...ui.groupSizes, [id]: avg }, splitDir: dir, id }
}

/**
 * Flip or set the layout axis while two panes are open. No-op when unsplit —
 * orientation only matters once a second group exists. Group order and sizes
 * stay put; only the CSS axis changes.
 */
export function setSplitDir(
  ui: GroupLayout,
  dir: SplitDir,
): Pick<GroupLayout, 'splitDir'> | null {
  if (ui.groups.length < 2 || ui.splitDir === dir) return null
  return { splitDir: dir }
}

/** Toggle row ↔ col when already split. */
export function toggleSplitDir(ui: GroupLayout): Pick<GroupLayout, 'splitDir'> | null {
  if (ui.groups.length < 2) return null
  return { splitDir: ui.splitDir === 'row' ? 'col' : 'row' }
}

/** Where a drag would land inside a group's content box. */
export type DropZone = 'center' | 'left' | 'right' | 'top' | 'bottom'

// Matches VSCode's feel: the outer fifth of each side splits, the middle just
// moves the tab into that group. Callers pass `allowEdges: false` once a split
// already exists so the overlay never promises a third pane / nested grid.
const EDGE_RATIO = 0.2

/** Classify a drop point (offset within a `w`x`h` box) into a drop zone. */
export function dropZoneFor(
  x: number,
  y: number,
  w: number,
  h: number,
  opts?: { allowEdges?: boolean },
): DropZone {
  if (w <= 0 || h <= 0) return 'center'
  if (opts?.allowEdges === false) return 'center'
  const fx = x / w
  const fy = y / h
  const near = Math.min(fx, 1 - fx, fy, 1 - fy)
  if (near >= EDGE_RATIO) return 'center'
  if (near === fx) return 'left'
  if (near === 1 - fx) return 'right'
  if (near === fy) return 'top'
  return 'bottom'
}

/**
 * Drag the divider that sits between `groups[index]` and `groups[index + 1]`:
 * trade `deltaPx` of the axis between exactly those two, leaving every other
 * pane untouched and neither below `MIN_FRACTION`.
 */
export function resizeGroups(
  groups: readonly EditorGroupId[],
  sizes: Record<EditorGroupId, number>,
  index: number,
  deltaPx: number,
  totalPx: number,
): Record<EditorGroupId, number> {
  const a = groups[index]
  const b = groups[index + 1]
  if (!a || !b || totalPx <= 0) return sizes
  const total = groups.reduce((sum, g) => sum + (sizes[g] ?? 1), 0)
  const min = MIN_FRACTION * total
  const pair = (sizes[a] ?? 1) + (sizes[b] ?? 1)
  // totalPx is the full grid axis (includes the 5px grip). Map delta against
  // the pane-only span so a drag matches the visible content columns/rows.
  const panePx = Math.max(1, totalPx - GRIP_PX)
  const wanted = (sizes[a] ?? 1) + (deltaPx / panePx) * total
  const next = Math.min(Math.max(wanted, min), pair - min)
  const other = pair - next
  // Identity-stable at the clamp edge so mousemove spam does not replace
  // groupSizes (and re-render both TabBars + every pane) every pixel.
  if (sizes[a] === next && sizes[b] === other) return sizes
  return { ...sizes, [a]: next, [b]: other }
}

/** A grid-placement pair, spread straight onto a style prop. */
export type GridCell = { gridColumn: string; gridRow: string }

export type GroupCells = {
  id: EditorGroupId
  /** The group's tab strip. */
  bar: GridCell
  /** Every tab content box belonging to this group (they stack in one cell). */
  content: GridCell
  /** Divider against the next group; null for the last one. */
  grip: GridCell | null
}

/**
 * Place all groups in ONE css grid.
 *
 * The point of a single grid — rather than nested flex containers — is that a
 * tab's content box changes only its `gridColumn`/`gridRow` when it moves
 * between panes. Its DOM parent never changes, so React never unmounts it and
 * the "chat stays mounted so its scroll/stream state survives" rule keeps
 * holding across splits, as does every terminal's scrollback and every Monaco
 * editor's view state.
 */
export function gridLayout(
  groups: readonly EditorGroupId[],
  sizes: Record<EditorGroupId, number>,
  dir: SplitDir,
): { gridTemplateColumns: string; gridTemplateRows: string; cells: GroupCells[] } {
  const fr = (g: EditorGroupId) => `minmax(0, ${sizes[g] ?? 1}fr)`
  const last = groups.length - 1

  // Unsplit: one explicit track pair — never emit a grip track or multi-fr
  // template left over from a prior split. WebKitGTK / WebView2 have kept a
  // phantom empty column/row after 2→1 when templates only changed fr weights.
  if (groups.length <= 1) {
    const id = groups[0] ?? DEFAULT_GROUP
    return {
      gridTemplateColumns: 'minmax(0, 1fr)',
      gridTemplateRows: 'auto minmax(0, 1fr)',
      cells: [
        {
          id,
          bar: { gridColumn: '1', gridRow: '1' },
          content: { gridColumn: '1', gridRow: '2' },
          grip: null,
        },
      ],
    }
  }

  if (dir === 'row') {
    // Columns alternate pane/grip; the two rows are the strip and the content.
    return {
      gridTemplateColumns: groups.map(fr).join(` ${GRIP_PX}px `),
      gridTemplateRows: 'auto minmax(0, 1fr)',
      cells: groups.map((id, i) => ({
        id,
        bar: { gridColumn: `${2 * i + 1}`, gridRow: '1' },
        content: { gridColumn: `${2 * i + 1}`, gridRow: '2' },
        grip: i === last ? null : { gridColumn: `${2 * i + 2}`, gridRow: '1 / 3' },
      })),
    }
  }

  // Stacked: each pane owns a strip row + a content row, with grip rows between.
  const rows: string[] = []
  for (const [i, g] of groups.entries()) {
    rows.push('auto', fr(g))
    if (i !== last) rows.push(`${GRIP_PX}px`)
  }
  return {
    gridTemplateColumns: 'minmax(0, 1fr)',
    gridTemplateRows: rows.join(' '),
    cells: groups.map((id, i) => ({
      id,
      bar: { gridColumn: '1', gridRow: `${3 * i + 1}` },
      content: { gridColumn: '1', gridRow: `${3 * i + 2}` },
      grip: i === last ? null : { gridColumn: '1', gridRow: `${3 * i + 3}` },
    })),
  }
}

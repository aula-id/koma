// VSCode-style split view (editor groups) for the main tab column.
//
// The membership model is still flat (`tabGroup`, `groupActive`, `groups` as
// live leaf ids). The *geometry* is a recursive binary tree (`splitTree`): any
// leaf can become |A|B| or atop/bbot, nested arbitrarily. `normalizeGroups`
// is the only place membership, focus, and the tree are repaired — open*Tab
// paths stay unaware of splits.
//
// Panes stay siblings (absolute boxes) so React never remounts chat / Monaco /
// xterm. Each split owns its own aSize/bSize — sibling stacks do not share
// tracks (tmux: LA[top|bot]|RB[top|bot] resizes independently).

export type EditorGroupId = string
export type SplitNodeId = string

/** Layout axis: 'row' = children side by side, 'col' = children stacked. */
export type SplitDir = 'row' | 'col'

export type EditorLayoutNode =
  | { type: 'leaf'; id: EditorGroupId }
  | {
      type: 'split'
      id: SplitNodeId
      dir: SplitDir
      aSize: number
      bSize: number
      a: EditorLayoutNode
      b: EditorLayoutNode
    }

/** The group every tab starts in — the "no split" state is a single leaf. */
export const DEFAULT_GROUP: EditorGroupId = 'g0'

export const DEFAULT_TREE: EditorLayoutNode = { type: 'leaf', id: DEFAULT_GROUP }

/** Safety ceiling on live panes. Edge drops stop offering a new leaf past this. */
export const MAX_GROUPS = 8

/** Width of the draggable divider between two children, in px. */
export const GRIP_PX = 5

/** Tab strip height. Matches `TabBar` `h-8` so content boxes sit under it. */
export const TAB_BAR_PX = 32

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
  splitTree?: EditorLayoutNode
  /** Parent split axis per leaf. Absent on the sole unsplit pane. */
  groupSplitDir?: Record<EditorGroupId, SplitDir>
}

/** Host snapshots / HMR / partial UI objects can omit children. Never throw. */
export function asLayoutNode(node: unknown): EditorLayoutNode {
  if (node == null || typeof node !== 'object') return { type: 'leaf', id: DEFAULT_GROUP }
  const n = node as Partial<EditorLayoutNode> & { id?: string; type?: string }
  if (n.type === 'split') {
    const a = n.a != null ? asLayoutNode(n.a) : null
    const b = n.b != null ? asLayoutNode(n.b) : null
    if (!a) return b ?? { type: 'leaf', id: DEFAULT_GROUP }
    if (!b) return a
    const dir: SplitDir = n.dir === 'col' ? 'col' : 'row'
    const aSize = typeof n.aSize === 'number' && Number.isFinite(n.aSize) && n.aSize > 0 ? n.aSize : 1
    const bSize = typeof n.bSize === 'number' && Number.isFinite(n.bSize) && n.bSize > 0 ? n.bSize : 1
    const id = typeof n.id === 'string' && n.id ? n.id : 's0'
    if (a === n.a && b === n.b && dir === n.dir && aSize === n.aSize && bSize === n.bSize && id === n.id) return n as EditorLayoutNode
    return { type: 'split', id, dir, aSize, bSize, a, b }
  }
  const id = typeof n.id === 'string' && n.id ? n.id : DEFAULT_GROUP
  if (n.type === 'leaf' && n.id === id) return n as EditorLayoutNode
  return { type: 'leaf', id }
}

export function groupTabIds(ui: GroupLayout, groupId: EditorGroupId): string[] {
  const out: string[] = []
  for (const t of ui.tabs ?? []) {
    if ((ui.tabGroup?.[t.id] ?? ui.activeGroupId) === groupId) out.push(t.id)
  }
  return out
}

export function groupOf(ui: GroupLayout, tabId: string): EditorGroupId {
  return ui.tabGroup?.[tabId] ?? ui.activeGroupId ?? DEFAULT_GROUP
}

export function isTabVisible(ui: GroupLayout, tabId: string): boolean {
  return ui.groupActive?.[groupOf(ui, tabId)] === tabId
}

export function leafIds(node: EditorLayoutNode | null | undefined): EditorGroupId[] {
  const n = asLayoutNode(node)
  if (n.type === 'leaf') return [n.id]
  return [...leafIds(n.a), ...leafIds(n.b)]
}

export function parentSplitOf(node: EditorLayoutNode | null | undefined, leafId: EditorGroupId): Extract<EditorLayoutNode, { type: 'split' }> | null {
  const n = asLayoutNode(node)
  if (n.type === 'leaf') return null
  if (n.a.type === 'leaf' && n.a.id === leafId) return n
  if (n.b.type === 'leaf' && n.b.id === leafId) return n
  return parentSplitOf(n.a, leafId) ?? parentSplitOf(n.b, leafId)
}

export function findSplit(node: EditorLayoutNode | null | undefined, splitId: SplitNodeId): Extract<EditorLayoutNode, { type: 'split' }> | null {
  const n = asLayoutNode(node)
  if (n.type === 'leaf') return null
  if (n.id === splitId) return n
  return findSplit(n.a, splitId) ?? findSplit(n.b, splitId)
}

export function nextGroupId(groups: readonly EditorGroupId[] | null | undefined): EditorGroupId {
  let max = -1
  for (const g of groups ?? []) {
    const n = /^g(\d+)$/.exec(g)
    if (n) max = Math.max(max, Number(n[1]))
  }
  return `g${max + 1}`
}

export function nextSplitId(node: EditorLayoutNode | null | undefined): SplitNodeId {
  let max = -1
  const walk = (n: EditorLayoutNode) => {
    if (n.type === 'leaf') return
    const m = /^s(\d+)$/.exec(n.id)
    if (m) max = Math.max(max, Number(m[1]))
    walk(n.a)
    walk(n.b)
  }
  walk(asLayoutNode(node))
  return `s${max + 1}`
}

export function migrateTree(ui: GroupLayout): EditorLayoutNode {
  if (ui.splitTree != null) return asLayoutNode(ui.splitTree)
  const groups = (ui.groups ?? []).filter((g, i) => (ui.groups ?? []).indexOf(g) === i)
  const sizes = ui.groupSizes ?? {}
  const dir: SplitDir = ui.splitDir === 'col' ? 'col' : 'row'
  if (groups.length <= 1) return { type: 'leaf', id: groups[0] ?? DEFAULT_GROUP }
  if (groups.length === 2) {
    return {
      type: 'split',
      id: 's0',
      dir,
      aSize: sizes[groups[0]] ?? 1,
      bSize: sizes[groups[1]] ?? 1,
      a: { type: 'leaf', id: groups[0] },
      b: { type: 'leaf', id: groups[1] },
    }
  }
  return groups.slice(1).reduce<EditorLayoutNode>(
    (acc, id, i) => ({
      type: 'split',
      id: `s${i}`,
      dir,
      aSize: 1,
      bSize: 1,
      a: acc,
      b: { type: 'leaf', id },
    }),
    { type: 'leaf', id: groups[0] },
  )
}

function pruneEmpty(node: EditorLayoutNode, counts: Map<EditorGroupId, number>): EditorLayoutNode | null {
  if (node.type === 'leaf') return (counts.get(node.id) ?? 0) > 0 ? node : null
  const a = pruneEmpty(node.a, counts)
  const b = pruneEmpty(node.b, counts)
  if (a && b) {
    if (a === node.a && b === node.b) return node
    return { ...node, a, b }
  }
  return a ?? b
}

function groupSplitDirs(node: EditorLayoutNode, parent: SplitDir | null, out: Record<EditorGroupId, SplitDir>) {
  if (node.type === 'leaf') {
    if (parent) out[node.id] = parent
    return
  }
  groupSplitDirs(node.a, node.dir, out)
  groupSplitDirs(node.b, node.dir, out)
}

function sameTree(a: EditorLayoutNode, b: EditorLayoutNode): boolean {
  if (a === b) return true
  if (a.type !== b.type) return false
  if (a.type === 'leaf' && b.type === 'leaf') return a.id === b.id
  if (a.type === 'split' && b.type === 'split') {
    return (
      a.id === b.id &&
      a.dir === b.dir &&
      a.aSize === b.aSize &&
      a.bSize === b.bSize &&
      sameTree(a.a, b.a) &&
      sameTree(a.b, b.b)
    )
  }
  return false
}

function replaceNode(
  node: EditorLayoutNode,
  id: string,
  next: EditorLayoutNode,
): EditorLayoutNode {
  if (node.type === 'leaf') return node.id === id ? next : node
  if (node.id === id) return next
  const a = replaceNode(node.a, id, next)
  const b = replaceNode(node.b, id, next)
  if (a === node.a && b === node.b) return node
  return { ...node, a, b }
}

function mapSplit(
  node: EditorLayoutNode,
  splitId: SplitNodeId,
  fn: (split: Extract<EditorLayoutNode, { type: 'split' }>) => EditorLayoutNode,
): EditorLayoutNode {
  if (node.type === 'leaf') return node
  if (node.id === splitId) return fn(node)
  const a = mapSplit(node.a, splitId, fn)
  const b = mapSplit(node.b, splitId, fn)
  if (a === node.a && b === node.b) return node
  return { ...node, a, b }
}

/**
 * Re-establish every group invariant. Cheap and identity-stable: returns the
 * SAME object when nothing needed fixing, so it can run on every commit.
 */
export function normalizeGroups<S extends GroupLayout>(ui: S): S {
  if (ui == null || typeof ui !== 'object') {
    return ui
  }
  const tabs = ui.tabs ?? []
  const tabGroupIn = ui.tabGroup ?? {}
  const groupActiveIn = ui.groupActive ?? {}
  const groupSizesIn = ui.groupSizes ?? {}
  const rawTree = migrateTree({ ...ui, tabs, tabGroup: tabGroupIn, groupActive: groupActiveIn, groupSizes: groupSizesIn, groups: ui.groups ?? [] })
  const rawLeaves = leafIds(rawTree)
  const known = new Set(rawLeaves)
  const fallback = known.has(ui.activeGroupId) ? ui.activeGroupId : (rawLeaves[0] ?? DEFAULT_GROUP)

  const tabGroup: Record<string, EditorGroupId> = {}
  const counts = new Map<EditorGroupId, number>()
  for (const t of tabs) {
    if (!t?.id) continue
    const prev = tabGroupIn[t.id]
    const g = prev !== undefined && known.has(prev) ? prev : fallback
    tabGroup[t.id] = g
    counts.set(g, (counts.get(g) ?? 0) + 1)
  }

  const pruned = pruneEmpty(rawTree, counts)
  const splitTree: EditorLayoutNode = pruned ?? { type: 'leaf', id: fallback }
  const live = leafIds(splitTree)
  const liveSet = new Set(live)
  const activeGroupId = liveSet.has(fallback) ? fallback : (live[0] ?? DEFAULT_GROUP)

  const groupActive: Record<EditorGroupId, string> = {}
  for (const g of live) {
    const ids = tabs.filter((t) => t?.id && tabGroup[t.id] === g).map((t) => t.id)
    const held = groupActiveIn[g]
    groupActive[g] = held !== undefined && ids.includes(held) ? held : (ids[ids.length - 1] ?? '')
  }

  let nextActiveGroup = activeGroupId
  let activeTabId = ui.activeTabId
  if (tabGroup[activeTabId] !== undefined && liveSet.has(tabGroup[activeTabId])) {
    nextActiveGroup = tabGroup[activeTabId]
    groupActive[nextActiveGroup] = activeTabId
  } else {
    activeTabId = groupActive[activeGroupId] ?? activeTabId
  }

  const groupSizes: Record<EditorGroupId, number> = {}
  if (live.length === 1) {
    groupSizes[live[0]] = 1
  } else {
    for (const g of live) groupSizes[g] = groupSizesIn[g] ?? 1
  }

  const groupSplitDir: Record<EditorGroupId, SplitDir> = {}
  groupSplitDirs(splitTree, null, groupSplitDir)
  const splitDir: SplitDir = splitTree.type === 'split' ? splitTree.dir : 'row'

  const same =
    sameList(live, ui.groups) &&
    sameMap(tabGroup, ui.tabGroup) &&
    sameMap(groupActive, ui.groupActive) &&
    sameMap(groupSizes, ui.groupSizes) &&
    sameMap(groupSplitDir, ui.groupSplitDir ?? {}) &&
    nextActiveGroup === ui.activeGroupId &&
    activeTabId === ui.activeTabId &&
    splitDir === ui.splitDir &&
    ui.splitTree != null &&
    sameTree(splitTree, ui.splitTree)
  if (same) return ui

  return {
    ...ui,
    groups: live,
    tabGroup,
    groupActive,
    groupSizes,
    groupSplitDir,
    splitTree,
    splitDir,
    activeGroupId: nextActiveGroup,
    activeTabId,
  }
}

function sameList(a: readonly string[] | null | undefined, b: readonly string[] | null | undefined): boolean {
  if (!a || !b) return a === b
  return a.length === b.length && a.every((v, i) => v === b[i])
}

function sameMap<V>(a: Record<string, V> | null | undefined, b: Record<string, V> | null | undefined): boolean {
  if (!a || !b) return a === b
  const ka = Object.keys(a)
  if (ka.length !== Object.keys(b).length) return false
  return ka.every((k) => a[k] === b[k])
}

export function neighbourInGroup(ui: GroupLayout, closingId: string): string | null {
  const ids = groupTabIds(ui, groupOf(ui, closingId))
  const i = ids.indexOf(closingId)
  if (i < 0) return null
  return ids[i - 1] ?? ids[i + 1] ?? null
}

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
 * Split `targetId` into a nested pair and return the new leaf id.
 * Refused at MAX_GROUPS live leaves.
 */
export function insertGroup(
  ui: GroupLayout,
  targetId: EditorGroupId,
  side: 'before' | 'after',
  dir: SplitDir,
): { splitTree: EditorLayoutNode; groups: EditorGroupId[]; id: EditorGroupId } | null {
  const tree = migrateTree(ui)
  const leaves = leafIds(tree)
  if (leaves.length >= MAX_GROUPS || !leaves.includes(targetId)) return null
  const id = nextGroupId(leaves)
  const created: EditorLayoutNode = { type: 'leaf', id }
  const splitTree = replaceNode(tree, targetId, {
    type: 'split',
    id: nextSplitId(tree),
    dir,
    aSize: 1,
    bSize: 1,
    a: side === 'before' ? created : { type: 'leaf', id: targetId },
    b: side === 'before' ? { type: 'leaf', id: targetId } : created,
  })
  return { splitTree, groups: leafIds(splitTree), id }
}

/** Flip the parent split of `groupId` (focused leaf by default). */
export function toggleSplitDir(
  ui: GroupLayout,
  groupId: EditorGroupId = ui.activeGroupId,
): { splitTree: EditorLayoutNode } | null {
  const tree = migrateTree(ui)
  const parent = parentSplitOf(tree, groupId)
  if (!parent) return null
  const dir: SplitDir = parent.dir === 'row' ? 'col' : 'row'
  return { splitTree: mapSplit(tree, parent.id, (s) => ({ ...s, dir })) }
}

export function setSplitDir(
  ui: GroupLayout,
  dir: SplitDir,
  groupId: EditorGroupId = ui.activeGroupId,
): { splitTree: EditorLayoutNode } | null {
  const tree = migrateTree(ui)
  const parent = parentSplitOf(tree, groupId)
  if (!parent || parent.dir === dir) return null
  return { splitTree: mapSplit(tree, parent.id, (s) => ({ ...s, dir })) }
}

export type DropZone = 'center' | 'left' | 'right' | 'top' | 'bottom'

const EDGE_RATIO = 0.2

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

/** Trade weight between the two children of `splitId`. Identity-stable at clamp. */
export function resizeSplit(
  tree: EditorLayoutNode | null | undefined,
  splitId: SplitNodeId,
  deltaPx: number,
  totalPx: number,
): EditorLayoutNode {
  tree = asLayoutNode(tree)
  const split = findSplit(tree, splitId)
  if (!split || totalPx <= 0) return tree
  const pair = split.aSize + split.bSize
  const min = MIN_FRACTION * pair
  const panePx = Math.max(1, totalPx - GRIP_PX)
  const wanted = split.aSize + (deltaPx / panePx) * pair
  const next = Math.min(Math.max(wanted, min), pair - min)
  const other = pair - next
  if (split.aSize === next && split.bSize === other) return tree
  return mapSplit(tree, splitId, (s) => ({ ...s, aSize: next, bSize: other }))
}

/** @deprecated Use resizeSplit. Kept for the old two-pane index API in tests. */
export function resizeGroups(
  groups: readonly EditorGroupId[],
  sizes: Record<EditorGroupId, number>,
  index: number,
  deltaPx: number,
  totalPx: number,
): Record<EditorGroupId, number> {
  const a = groups?.[index]
  const b = groups?.[index + 1]
  if (!a || !b || totalPx <= 0) return sizes ?? {}
  const tree = resizeSplit(
    {
      type: 'split',
      id: 's0',
      dir: 'row',
      aSize: sizes[a] ?? 1,
      bSize: sizes[b] ?? 1,
      a: { type: 'leaf', id: a },
      b: { type: 'leaf', id: b },
    },
    's0',
    deltaPx,
    totalPx,
  )
  if (tree.type !== 'split') return sizes
  if (tree.aSize === (sizes[a] ?? 1) && tree.bSize === (sizes[b] ?? 1)) return sizes
  return { ...sizes, [a]: tree.aSize, [b]: tree.bSize }
}

export type FracRect = { x: number; y: number; w: number; h: number }

export type PaneBox = {
  position: 'absolute'
  left: string
  top: string
  width: string
  height: string
}

export type GridCell = PaneBox

export type GroupCells = {
  id: EditorGroupId
  bar: PaneBox
  content: PaneBox
}

export type GripCell = {
  id: SplitNodeId
  dir: SplitDir
  cell: PaneBox
}

function pct(n: number): string {
  const value = Number.isFinite(n) ? Math.max(0, n) * 100 : 0
  return `${+value.toFixed(4)}%`
}

function paneBox(x: number, y: number, w: number, h: number): PaneBox {
  return { position: 'absolute', left: pct(x), top: pct(y), width: pct(w), height: pct(h) }
}

function leafBoxes(rect: FracRect): { bar: PaneBox; content: PaneBox } {
  return {
    bar: { position: 'absolute', left: pct(rect.x), top: pct(rect.y), width: pct(rect.w), height: `${TAB_BAR_PX}px` },
    content: {
      position: 'absolute',
      left: pct(rect.x),
      top: `calc(${pct(rect.y)} + ${TAB_BAR_PX}px)`,
      width: pct(rect.w),
      height: `calc(${pct(rect.h)} - ${TAB_BAR_PX}px)`,
    },
  }
}

function splitChildren(node: Extract<EditorLayoutNode, { type: 'split' }>, rect: FracRect): { a: FracRect; b: FracRect; cut: number } {
  const pair = node.aSize + node.bSize
  const t = pair > 0 && Number.isFinite(pair) ? node.aSize / pair : 0.5
  if (node.dir === 'row') {
    return {
      a: { x: rect.x, y: rect.y, w: rect.w * t, h: rect.h },
      b: { x: rect.x + rect.w * t, y: rect.y, w: rect.w * (1 - t), h: rect.h },
      cut: t,
    }
  }
  return {
    a: { x: rect.x, y: rect.y, w: rect.w, h: rect.h * t },
    b: { x: rect.x, y: rect.y + rect.h * t, w: rect.w, h: rect.h * (1 - t) },
    cut: t,
  }
}

function gripBox(dir: SplitDir, rect: FracRect, cut: number): PaneBox {
  if (dir === 'row') {
    return {
      position: 'absolute',
      left: `calc(${pct(rect.x + rect.w * cut)} - ${GRIP_PX / 2}px)`,
      top: pct(rect.y),
      width: `${GRIP_PX}px`,
      height: pct(rect.h),
    }
  }
  return {
    position: 'absolute',
    left: pct(rect.x),
    top: `calc(${pct(rect.y + rect.h * cut)} - ${GRIP_PX / 2}px)`,
    width: pct(rect.w),
    height: `${GRIP_PX}px`,
  }
}

/** Fraction of the editor box owned by a leaf or split. Isolated from siblings. */
export function nodeFracRect(tree: EditorLayoutNode | null | undefined, id: string): FracRect | null {
  const walk = (node: EditorLayoutNode, rect: FracRect): FracRect | null => {
    if (node.type === 'leaf') return node.id === id ? rect : null
    if (node.id === id) return rect
    const kids = splitChildren(node, rect)
    return walk(node.a, kids.a) ?? walk(node.b, kids.b)
  }
  return walk(asLayoutNode(tree), { x: 0, y: 0, w: 1, h: 1 })
}

export function gridLayoutFromTree(tree: EditorLayoutNode | null | undefined): {
  cells: GroupCells[]
  grips: GripCell[]
} {
  tree = asLayoutNode(tree)
  const cells: GroupCells[] = []
  const grips: GripCell[] = []
  const walk = (node: EditorLayoutNode, rect: FracRect) => {
    if (node.type === 'leaf') {
      const boxes = leafBoxes(rect)
      cells.push({ id: node.id, bar: boxes.bar, content: boxes.content })
      return
    }
    const kids = splitChildren(node, rect)
    grips.push({ id: node.id, dir: node.dir, cell: gripBox(node.dir, rect, kids.cut) })
    walk(node.a, kids.a)
    walk(node.b, kids.b)
  }
  walk(tree, { x: 0, y: 0, w: 1, h: 1 })
  return { cells, grips }
}

/** Flat two-pane helper used by older tests. Prefer gridLayoutFromTree. */
export function gridLayout(
  groups: readonly EditorGroupId[],
  sizes: Record<EditorGroupId, number>,
  dir: SplitDir,
): { cells: Array<GroupCells & { grip: PaneBox | null }> } {
  if (groups.length <= 1) {
    const laid = gridLayoutFromTree({ type: 'leaf', id: groups[0] ?? DEFAULT_GROUP })
    return { cells: laid.cells.map((cell) => ({ ...cell, grip: null })) }
  }
  const laid = gridLayoutFromTree({
    type: 'split',
    id: 's0',
    dir,
    aSize: sizes[groups[0]] ?? 1,
    bSize: sizes[groups[1]] ?? 1,
    a: { type: 'leaf', id: groups[0] },
    b: { type: 'leaf', id: groups[1] },
  })
  const grip = laid.grips[0]?.cell ?? null
  return {
    cells: laid.cells.map((cell, i) => ({
      ...cell,
      grip: i === 0 ? grip : null,
    })),
  }
}

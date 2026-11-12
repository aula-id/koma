import type { Goal, GoalNode, Report, ShelfRow } from "./room-link.ts"

/** One line of a browse list. A folder opens and closes; a leaf opens its detail. */
export type TreeRow = {
  /** Stable across refreshes, so the highlight and the collapsed folders survive new data. */
  id: string
  depth: number
  kind: "folder" | "leaf"
  label: string
  detail: string
  /** A folder is open when its rows are shown. */
  open: boolean
  /** What a leaf stands for: a shelf key, a goal node, a report. */
  ref: { type: "item"; key: string } | { type: "node"; goal: string; node: string } | { type: "report"; key: string } | null
  /** Short mark for the row: a count, a severity, a status. */
  mark: string
  /** Branch lines in front of a tree node, such as `│  ├─`. */
  guide?: string
  /** A trajectory node's place in the tree, such as `1.1.2`. */
  number?: string
  /** Where else this node is linked from, as numbers. */
  refs?: string[]
}

export type Collapsed = ReadonlySet<string>

/** The shelf as folders. `claims/web/ssrf` sits in `claims/` then `web/`. */
export function shelfRows(rows: ShelfRow[], collapsed: Collapsed): TreeRow[] {
  type Folder = { path: string; folders: Map<string, Folder>; leaves: ShelfRow[]; count: number }
  const root: Folder = { path: "", folders: new Map(), leaves: [], count: 0 }
  for (const row of [...rows].sort((a, b) => a.key.localeCompare(b.key))) {
    const parts = row.key.split("/")
    let at = root
    at.count += 1
    for (const part of parts.slice(0, -1)) {
      let next = at.folders.get(part)
      if (!next) {
        next = { path: `${at.path}${part}/`, folders: new Map(), leaves: [], count: 0 }
        at.folders.set(part, next)
      }
      next.count += 1
      at = next
    }
    at.leaves.push(row)
  }
  const out: TreeRow[] = []
  const walk = (folder: Folder, depth: number) => {
    for (const [name, child] of folder.folders) {
      const open = !collapsed.has(`dir:${child.path}`)
      out.push({ id: `dir:${child.path}`, depth, kind: "folder", label: `${name}/`, detail: "", open, ref: null, mark: `${child.count}` })
      if (open) walk(child, depth + 1)
    }
    for (const leaf of folder.leaves) {
      const name = leaf.key.split("/").at(-1) ?? leaf.key
      out.push({ id: `item:${leaf.key}`, depth, kind: "leaf", label: name, detail: leaf.title === name ? "" : leaf.title, open: false, ref: { type: "item", key: leaf.key }, mark: leaf.status })
    }
  }
  walk(root, 0)
  return out
}

/** One node of a goal's tree: where it sits and where else it is linked from. */
export type TNode = {
  node: GoalNode
  /** `1.1.2`: the position among its parents' children, all the way down. */
  number: string
  depth: number
  parent: string | null
  children: string[]
  /** Whether it is the last child, for the branch glyph. */
  last: boolean
  /** The numbers of the other nodes that link here. */
  alsoFrom: string[]
}

/**
 * A goal's links as a tree. A node sits once, under the first node that reaches it, and any other
 * node linking to it is listed on it as a number, so nothing is drawn twice.
 */
export function trajectoryTree(goal: Goal): { order: TNode[]; byId: Map<string, TNode> } {
  const nodes = new Map(goal.nodes.map((n) => [n.id, n]))
  const out = new Map<string, string[]>()
  const incoming = new Set<string>()
  for (const e of goal.edges) {
    if (e.from === e.to || !nodes.has(e.from) || !nodes.has(e.to)) continue
    const list = out.get(e.from) ?? []
    if (!list.includes(e.to)) list.push(e.to)
    out.set(e.from, list)
    incoming.add(e.to)
  }
  const rank = (id: string) => nodes.get(id)!
  const roots = goal.nodes.filter((n) => !incoming.has(n.id)).sort((a, b) => Number(b.kind === "start") - Number(a.kind === "start") || a.rank - b.rank || a.row - b.row).map((n) => n.id)
  const byId = new Map<string, TNode>()
  const order: TNode[] = []
  const place = (id: string, parent: TNode | null, index: number, depth: number) => {
    if (byId.has(id) || depth > 200) return
    const t: TNode = { node: rank(id), number: parent ? `${parent.number}.${index}` : `${index}`, depth, parent: parent?.node.id ?? null, children: [], last: false, alsoFrom: [] }
    byId.set(id, t)
    order.push(t)
    if (parent) parent.children.push(id)
    let n = 0
    for (const child of out.get(id) ?? []) {
      if (byId.has(child)) continue
      place(child, t, ++n, depth + 1)
    }
  }
  let top = 0
  for (const id of roots) place(id, null, ++top, 0)
  // A loop with no way in still gets drawn: start at its first node.
  for (const n of [...goal.nodes].sort((a, b) => a.rank - b.rank || a.row - b.row)) if (!byId.has(n.id)) place(n.id, null, ++top, 0)
  for (const t of order) {
    const siblings = t.parent ? byId.get(t.parent)!.children : roots.filter((r) => byId.get(r)!.parent === null)
    t.last = siblings.at(-1) === t.node.id
  }
  for (const [from, list] of out) {
    for (const to of list) {
      const target = byId.get(to)
      const source = byId.get(from)
      if (target && source && target.parent !== from && !target.alsoFrom.includes(source.number)) target.alsoFrom.push(source.number)
    }
  }
  return { order, byId }
}

/** The branch glyphs in front of a node: `│  ` for each ancestor with more to come, then `├─` or `└─`. */
function guideOf(t: TNode, byId: Map<string, TNode>): string {
  let prefix = ""
  let at = t.parent ? byId.get(t.parent) : undefined
  const stack: string[] = []
  while (at) {
    stack.push(at.last ? "   " : "│  ")
    at = at.parent ? byId.get(at.parent) : undefined
  }
  prefix = stack.reverse().join("")
  return `${prefix}${t.last ? "└─" : "├─"}`
}

/** Goals as folders holding their tree. Every node is open unless it was folded. */
export function goalRows(goals: Goal[], collapsed: Collapsed): TreeRow[] {
  const out: TreeRow[] = []
  for (const goal of goals) {
    const id = `goal:${goal.goal_key}`
    const open = !collapsed.has(id)
    out.push({ id, depth: 0, kind: "folder", label: goal.goal_key, detail: `${goal.open} open · ${goal.verified} verified · ${goal.returned} returned`, open, ref: null, mark: `${goal.nodes.length}` })
    if (!open) continue
    const { order, byId } = trajectoryTree(goal)
    for (const t of order) {
      const rowId = `node:${goal.goal_key}:${t.node.id}`
      // Hidden while a node above it is folded.
      let hidden = false
      for (let up = t.parent ? byId.get(t.parent) : undefined; up; up = up.parent ? byId.get(up.parent) : undefined) {
        if (collapsed.has(`node:${goal.goal_key}:${up.node.id}`)) { hidden = true; break }
      }
      if (hidden) continue
      const folds = t.children.length > 0
      out.push({
        id: rowId, depth: t.depth + 1, kind: folds ? "folder" : "leaf", label: t.node.label || t.node.id, detail: t.node.kind,
        open: folds && !collapsed.has(rowId), ref: { type: "node", goal: goal.goal_key, node: t.node.id }, mark: t.node.severity || t.node.status,
        guide: guideOf(t, byId), number: t.number, refs: t.alsoFrom,
      })
    }
  }
  return out
}

export function reportRows(reports: Report[]): TreeRow[] {
  return reports.map((r) => ({
    id: `report:${r.key}`, depth: 0, kind: "leaf" as const, label: r.title || r.key, detail: [r.audience, r.severity_summary].filter(Boolean).join(" · "),
    open: false, ref: { type: "report" as const, key: r.key }, mark: r.go_no_go,
  }))
}

/** Where the highlight lands after a move, kept inside the list. */
export function move(rows: TreeRow[], selected: string, direction: -1 | 1 | "first" | "last"): string {
  if (!rows.length) return ""
  const at = Math.max(0, rows.findIndex((r) => r.id === selected))
  const next = direction === "first" ? 0 : direction === "last" ? rows.length - 1 : Math.max(0, Math.min(rows.length - 1, at + direction))
  return rows[next]!.id
}

/** The folder a row belongs to, so Left on a leaf climbs out of it. */
export function parentOf(rows: TreeRow[], id: string): string | null {
  const at = rows.findIndex((r) => r.id === id)
  if (at <= 0) return null
  const depth = rows[at]!.depth
  for (let i = at - 1; i >= 0; i--) if (rows[i]!.kind === "folder" && rows[i]!.depth < depth) return rows[i]!.id
  return null
}

export function toggled(collapsed: Collapsed, id: string): Set<string> {
  const next = new Set(collapsed)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  return next
}

/** What one key does in a list: move, open or close a folder, or open a leaf. */
export type Step = { selected: string; collapsed: Set<string>; open: TreeRow | null }

export function press(rows: TreeRow[], selected: string, collapsed: Collapsed, key: string): Step {
  const row = rows.find((r) => r.id === selected) ?? rows[0]
  const keep = { selected: row?.id ?? "", collapsed: new Set(collapsed), open: null as TreeRow | null }
  if (!row) return keep
  switch (key) {
    case "up": case "k": return { ...keep, selected: move(rows, row.id, -1) }
    case "down": case "j": return { ...keep, selected: move(rows, row.id, 1) }
    case "home": return { ...keep, selected: move(rows, row.id, "first") }
    case "end": return { ...keep, selected: move(rows, row.id, "last") }
    case "right":
      if (row.kind === "folder" && !row.open) return { ...keep, collapsed: toggled(collapsed, row.id) }
      if (row.kind === "folder") return { ...keep, selected: move(rows, row.id, 1) }
      return row.ref ? { ...keep, open: row } : keep
    case "left":
      if (row.kind === "folder" && row.open) return { ...keep, collapsed: toggled(collapsed, row.id) }
      return { ...keep, selected: parentOf(rows, row.id) ?? row.id }
    case "space":
      if (row.kind === "folder") return { ...keep, collapsed: toggled(collapsed, row.id) }
      return { ...keep, open: row }
    case "return": case "enter":
      if (row.ref) return { ...keep, open: row }
      if (row.kind === "folder") return { ...keep, collapsed: toggled(collapsed, row.id) }
      return keep
    default: return keep
  }
}

/** The rows to draw: a window that keeps the highlight in view. */
export function windowOf(rows: TreeRow[], selected: string, start: number, height: number): { from: number; rows: TreeRow[] } {
  const at = Math.max(0, rows.findIndex((r) => r.id === selected))
  const max = Math.max(0, rows.length - height)
  let from = Math.max(0, Math.min(start, max))
  if (at < from) from = at
  else if (at >= from + height) from = at - height + 1
  from = Math.max(0, Math.min(from, max))
  return { from, rows: rows.slice(from, from + height) }
}

/** What a node is connected to, for its detail screen. */
export type Around = {
  number: string
  /** Every node that links here, the first being the one it hangs under. */
  from: { node: GoalNode; number: string; rel: string }[]
  /** Every node it links to. */
  to: { node: GoalNode; number: string; rel: string }[]
  /** Nodes that share a parent with it. */
  related: { node: GoalNode; number: string }[]
}

export function describe(goal: Goal, nodeId: string): Around | null {
  const { byId } = trajectoryTree(goal)
  const me = byId.get(nodeId)
  if (!me) return null
  const numberOf = (id: string) => byId.get(id)?.number ?? ""
  const from = goal.edges.filter((e) => e.to === nodeId && byId.has(e.from)).map((e) => ({ node: byId.get(e.from)!.node, number: numberOf(e.from), rel: e.rel }))
  const to = goal.edges.filter((e) => e.from === nodeId && byId.has(e.to)).map((e) => ({ node: byId.get(e.to)!.node, number: numberOf(e.to), rel: e.rel }))
  const siblings = me.parent ? byId.get(me.parent)!.children.filter((id) => id !== nodeId) : []
  return { number: me.number, from, to, related: siblings.map((id) => ({ node: byId.get(id)!.node, number: numberOf(id) })) }
}

/** A record as `name  value` lines, in a readable order. Long text stays whole. */
export function fields(content: string): { name: string; value: string }[] | null {
  let parsed: unknown
  try { parsed = JSON.parse(content) } catch { return null }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null
  const record = parsed as Record<string, unknown>
  const first = ["kind", "name", "title", "severity", "status", "verdict", "host", "vulnclass", "summary", "claim_key", "verdict_key"]
  const names = [...first.filter((n) => n in record), ...Object.keys(record).filter((n) => !first.includes(n) && n !== "updated")]
  return names.map((name) => {
    const value = record[name]
    const text = typeof value === "string" ? value : Array.isArray(value) && value.every((v) => typeof v === "string") ? value.join(", ") : JSON.stringify(value, null, 2)
    return { name, value: text ?? "" }
  }).filter((f) => f.value !== "" && f.value !== "[]" && f.value !== "null")
}

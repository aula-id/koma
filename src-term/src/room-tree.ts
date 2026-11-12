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

/** Goals as folders, each holding its nodes in the order of the links. */
export function goalRows(goals: Goal[], collapsed: Collapsed): TreeRow[] {
  const out: TreeRow[] = []
  for (const goal of goals) {
    const id = `goal:${goal.goal_key}`
    const open = !collapsed.has(id)
    out.push({ id, depth: 0, kind: "folder", label: goal.goal_key, detail: `${goal.open} open · ${goal.verified} verified · ${goal.returned} returned`, open, ref: null, mark: `${goal.nodes.length}` })
    if (!open) continue
    for (const node of goal.nodes) {
      out.push({ id: `node:${goal.goal_key}:${node.id}`, depth: 1, kind: "leaf", label: node.label || node.id, detail: node.kind, open: false, ref: { type: "node", goal: goal.goal_key, node: node.id }, mark: node.severity || node.status })
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
      return { ...keep, open: row }
    case "left":
      if (row.kind === "folder" && row.open) return { ...keep, collapsed: toggled(collapsed, row.id) }
      return { ...keep, selected: parentOf(rows, row.id) ?? row.id }
    case "return": case "enter": case "space":
      if (row.kind === "folder") return { ...keep, collapsed: toggled(collapsed, row.id) }
      return { ...keep, open: row }
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

/** The edges around one node, for its detail screen. */
export function around(goal: Goal, nodeId: string): { from: { node: GoalNode; rel: string }[]; to: { node: GoalNode; rel: string }[] } {
  const byId = new Map(goal.nodes.map((n) => [n.id, n]))
  const from = goal.edges.filter((e) => e.to === nodeId).flatMap((e) => byId.get(e.from) ? [{ node: byId.get(e.from)!, rel: e.rel }] : [])
  const to = goal.edges.filter((e) => e.from === nodeId).flatMap((e) => byId.get(e.to) ? [{ node: byId.get(e.to)!, rel: e.rel }] : [])
  return { from, to }
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

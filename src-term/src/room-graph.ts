import type { Goal, GoalNode } from "./room-link.ts"
import { trajectoryTree } from "./room-tree.ts"

/** A node's box, in terminal cells: three lines of text inside a border. */
export const BOX_W = 22
export const BOX_H = 5
/** Cells between one column's right edge and the next column's left edge, for the lines and the arrow. */
const GAP = 5
const STEP = BOX_W + GAP
/** Rows from one box's top to the next in a column: the box and one spare row for lines that pass. */
const PITCH = BOX_H + 1

export type Box = { id: string; x: number; y: number; w: number; h: number; node: GoalNode; number: string; backLinks: number }
export type Tone = "edge" | "box" | "select" | "success" | "warn" | "error" | "info" | "dim"
export type Cell = { ch: string; tone: Tone }
export type GraphLayout = {
  boxes: Box[]
  width: number
  height: number
  /** Lines between boxes: for each cell, which sides a line leaves through. */
  bits: Map<number, number>
  /** Lines of links that cross another link: drawn dotted, so each can be followed. */
  dots: Map<number, number>
  /** Arrow heads, drawn over the lines. */
  marks: Map<number, string>
  key: (x: number, y: number) => number
}

const U = 1, D = 2, L = 4, R = 8
const JOIN: Record<number, string> = { 1: "│", 2: "│", 3: "│", 4: "─", 8: "─", 12: "─", 9: "└", 5: "┘", 10: "┌", 6: "┐", 11: "├", 7: "┤", 14: "┬", 13: "┴", 15: "┼" }

/**
 * A left-to-right diagram. Depth runs across the screen, one column per depth, and the boxes of
 * one depth stack down their column. Links leave a box on its right and arrive on the left of the
 * next column. A link that skips a column runs along the spare row above its target, so it never
 * crosses a box. A link back or sideways is not drawn; the box says how many there are (`↺2`), and
 * the detail screen lists them. A long column runs off the bottom and a long chain off the right;
 * the screen scrolls both ways.
 */
export function layoutGraph(goal: Goal): GraphLayout {
  const { byId } = trajectoryTree(goal)
  const ranks = [...new Set(goal.nodes.map((n) => n.rank))].sort((a, b) => a - b)
  const columnOf = new Map(ranks.map((r, i) => [r, i]))
  const laneOf = new Map<string, number>()
  for (const rank of ranks) {
    goal.nodes.filter((n) => n.rank === rank).sort((a, b) => a.row - b.row || a.id.localeCompare(b.id)).forEach((n, lane) => laneOf.set(n.id, lane))
  }
  const boxes: Box[] = goal.nodes.map((n) => ({
    id: n.id, x: 2 + columnOf.get(n.rank)! * STEP, y: 1 + laneOf.get(n.id)! * PITCH, w: BOX_W, h: BOX_H, node: n, number: byId.get(n.id)?.number ?? "", backLinks: 0,
  }))
  const at = new Map(boxes.map((b) => [b.id, b]))
  const width = Math.max(...boxes.map((b) => b.x + b.w), 0) + 4
  const height = Math.max(...boxes.map((b) => b.y + b.h), 0) + 2
  const key = (x: number, y: number) => y * (width + 8) + x
  const marks = new Map<number, string>()
  // Each link is routed on its own first, so crossings between links can be found afterwards.
  type Route = { from: string; to: string; cells: Map<number, number> }
  const routes: Route[] = []
  let route: Route
  const add = (x: number, y: number, bit: number) => { if (x >= 0 && y >= 0) route.cells.set(key(x, y), (route.cells.get(key(x, y)) ?? 0) | bit) }
  const horizontal = (y: number, x1: number, x2: number) => {
    const [a, b] = x1 <= x2 ? [x1, x2] : [x2, x1]
    for (let x = a; x <= b; x++) {
      if (x < b) add(x, y, R)
      if (x > a) add(x, y, L)
    }
  }
  const vertical = (x: number, y1: number, y2: number) => {
    const [a, b] = y1 <= y2 ? [y1, y2] : [y2, y1]
    for (let y = a; y <= b; y++) {
      if (y < b) add(x, y, D)
      if (y > a) add(x, y, U)
    }
  }
  const mid = (b: Box) => b.y + (BOX_H >> 1)
  const seen = new Set<string>()
  for (const e of goal.edges) {
    const from = at.get(e.from), to = at.get(e.to)
    const id = `${e.from}>${e.to}`
    if (!from || !to || from === to || seen.has(id)) continue
    seen.add(id)
    const fc = columnOf.get(from.node.rank)!, tc = columnOf.get(to.node.rank)!
    if (tc <= fc) { to.backLinks += 1; continue }
    route = { from: from.id, to: to.id, cells: new Map() }
    routes.push(route)
    const ay = mid(from), by = mid(to)
    const bus = from.x + BOX_W + 1
    // Out of the right side of the box, to a bus just beyond it.
    horizontal(ay, from.x + BOX_W, bus)
    if (tc === fc + 1) {
      vertical(bus, ay, by)
      horizontal(by, bus, to.x - 2)
    } else {
      const row = to.y - 1
      const gx = to.x - 3
      vertical(bus, ay, row)
      horizontal(row, bus, gx)
      vertical(gx, row, by)
      horizontal(by, gx, to.x - 2)
    }
    marks.set(key(to.x - 1, by), "►")
  }
  // Links that share a start or an end merge cleanly. Two that share cells but neither end cross,
  // and a reader cannot tell who goes where: the longer one of the pair is drawn dotted.
  const owners = new Map<number, number[]>()
  routes.forEach((r, i) => { for (const cell of r.cells.keys()) owners.set(cell, [...(owners.get(cell) ?? []), i]) })
  const dotted = new Set<number>()
  for (const list of owners.values()) {
    for (let a = 0; a < list.length; a++) {
      for (let b = a + 1; b < list.length; b++) {
        const x = routes[list[a]!]!, y = routes[list[b]!]!
        if (x.from === y.from || x.to === y.to) continue
        dotted.add(x.cells.size > y.cells.size ? list[a]! : list[b]!)
      }
    }
  }
  const bits = new Map<number, number>()
  const dots = new Map<number, number>()
  routes.forEach((r, i) => {
    const into = dotted.has(i) ? dots : bits
    for (const [cell, bit] of r.cells) into.set(cell, (into.get(cell) ?? 0) | bit)
  })
  return { boxes, width, height, bits, dots, marks, key }
}

function toneOf(node: GoalNode): Tone {
  const mark = (node.severity || node.status).toLowerCase()
  if (["critical", "high", "failed", "blocked"].includes(mark)) return "error"
  if (["medium", "open"].includes(mark)) return "warn"
  if (["verified", "confirmed", "done", "met", "low"].includes(mark)) return "success"
  if (["info"].includes(mark)) return "info"
  return "box"
}

const clip = (text: string, width: number) => ([...text].length <= width ? text : `${[...text].slice(0, Math.max(0, width - 1)).join("")}…`)

/** A label over two lines, breaking at a space when there is one; the second line ends in … if it is cut. */
export function wrap2(text: string, width: number): [string, string] {
  const chars = [...text]
  if (chars.length <= width) return [text, ""]
  let cut = chars.slice(0, width + 1).lastIndexOf(" ")
  if (cut < width / 2) cut = width
  const first = chars.slice(0, cut).join("").trimEnd()
  return [first, clip(chars.slice(cut).join("").trim(), width)]
}

/** The whole picture as cells. The screen shows a window of it. */
export function drawGraph(layout: GraphLayout, selected: string | null): Cell[][] {
  const grid: Cell[][] = Array.from({ length: layout.height }, () => Array.from({ length: layout.width }, () => ({ ch: " ", tone: "edge" as Tone })))
  const put = (x: number, y: number, ch: string, tone: Tone) => { if (grid[y]?.[x]) grid[y]![x] = { ch, tone } }
  for (const [k, bit] of layout.bits) {
    const stride = layout.width + 8
    const y = Math.floor(k / stride), x = k % stride
    put(x, y, JOIN[bit] ?? "┼", "edge")
  }
  // A dotted link shows as dots where it runs alone, and as the line it crosses where they meet.
  for (const [k, bit] of layout.dots) {
    const stride = layout.width + 8
    const y = Math.floor(k / stride), x = k % stride
    if (layout.bits.has(k)) { put(x, y, JOIN[(layout.bits.get(k) ?? 0) | bit] ?? "┼", "edge"); continue }
    put(x, y, bit === L || bit === R || bit === (L | R) ? "┄" : bit === U || bit === D || bit === (U | D) ? "┆" : JOIN[bit] ?? "┼", "edge")
  }
  for (const [k, ch] of layout.marks) {
    const stride = layout.width + 8
    put(k % stride, Math.floor(k / stride), ch, "edge")
  }
  for (const box of layout.boxes) {
    const tone = box.id === selected ? "select" : toneOf(box.node)
    const inner = box.w - 2
    const [first, second] = wrap2(`${box.number ? `(${box.number}) ` : ""}${box.node.label || box.node.id}`, inner - 2)
    const third = [box.node.kind, box.node.severity || box.node.status, box.backLinks ? `↺${box.backLinks}` : ""].filter(Boolean).join(" · ")
    const row = (text: string) => `│ ${clip(text, inner - 2).padEnd(inner - 2)} │`
    const lines = [`╭${"─".repeat(inner)}╮`, row(first), row(second), row(third), `╰${"─".repeat(inner)}╯`]
    lines.forEach((line, r) => [...line].forEach((ch, col) => put(box.x + col, box.y + r, ch, tone)))
    // Where a line joins the box: in on the left, out on the right.
    const my = box.y + (BOX_H >> 1)
    if (layout.bits.has(layout.key(box.x - 1, my)) || layout.dots.has(layout.key(box.x - 1, my))) put(box.x, my, "┤", tone)
    if (layout.bits.has(layout.key(box.x + BOX_W, my)) || layout.dots.has(layout.key(box.x + BOX_W, my))) put(box.x + BOX_W - 1, my, "├", tone)
  }
  return grid
}

/** Which box a cell belongs to, if any. */
export function boxAt(layout: GraphLayout, x: number, y: number): Box | null {
  return layout.boxes.find((b) => x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h) ?? null
}

/** The window's top-left after a pan, kept inside the picture. */
export function pan(offset: { x: number; y: number }, dx: number, dy: number, layout: GraphLayout, view: { width: number; height: number }): { x: number; y: number } {
  const maxX = Math.max(0, layout.width - view.width)
  const maxY = Math.max(0, layout.height - view.height)
  return { x: Math.max(0, Math.min(maxX, offset.x + dx)), y: Math.max(0, Math.min(maxY, offset.y + dy)) }
}

/** The window's top-left that brings a box into view, moving as little as it can. */
export function reveal(offset: { x: number; y: number }, box: Box, layout: GraphLayout, view: { width: number; height: number }): { x: number; y: number } {
  let { x, y } = offset
  if (box.x < x + 2) x = box.x - 2
  else if (box.x + box.w > x + view.width - 2) x = box.x + box.w - view.width + 2
  if (box.y - 3 < y) y = box.y - 3
  else if (box.y + box.h + 1 > y + view.height) y = box.y + box.h + 1 - view.height
  return pan({ x: 0, y: 0 }, x, y, layout, view)
}

/** The boxes in reading order, for Tab. */
export function readingOrder(layout: GraphLayout): Box[] {
  return [...layout.boxes].sort((a, b) => a.x - b.x || a.y - b.y)
}

/** One window of the picture as lines of same-coloured runs. */
export function windowLines(grid: Cell[][], offset: { x: number; y: number }, view: { width: number; height: number }): { tone: Tone; text: string }[][] {
  const out: { tone: Tone; text: string }[][] = []
  for (let y = offset.y; y < Math.min(grid.length, offset.y + view.height); y++) {
    const runs: { tone: Tone; text: string }[] = []
    for (let x = offset.x; x < Math.min(grid[y]!.length, offset.x + view.width); x++) {
      const cell = grid[y]![x]!
      const last = runs.at(-1)
      if (last && last.tone === cell.tone) last.text += cell.ch
      else runs.push({ tone: cell.tone, text: cell.ch })
    }
    out.push(runs)
  }
  return out
}

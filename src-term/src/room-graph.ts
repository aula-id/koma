import type { Goal, GoalNode } from "./room-link.ts"
import { trajectoryTree } from "./room-tree.ts"

/** The size of a node's box and the space around it, in terminal cells. */
export const BOX_W = 24
export const BOX_H = 3
const SLOT = BOX_W + 4
const RANK = BOX_H + 3

export type Box = { id: string; x: number; y: number; w: number; h: number; node: GoalNode; number: string; backLinks: number }
export type Tone = "edge" | "box" | "select" | "success" | "warn" | "error" | "info" | "dim"
export type Cell = { ch: string; tone: Tone }
export type GraphLayout = {
  boxes: Box[]
  width: number
  height: number
  /** Lines between boxes: for each cell, which sides a line leaves through. */
  bits: Map<number, number>
  /** Arrow heads and relation names, drawn over the lines. */
  marks: Map<number, string>
  key: (x: number, y: number) => number
}

const U = 1, D = 2, L = 4, R = 8
const JOIN: Record<number, string> = { 1: "│", 2: "│", 3: "│", 4: "─", 8: "─", 12: "─", 9: "└", 5: "┘", 10: "┌", 6: "┐", 11: "├", 7: "┤", 14: "┬", 13: "┴", 15: "┼" }

/**
 * Where each node's box goes and how the links run between them. Depth is the row of boxes
 * going down; the lane inside a depth is the column. A link to a node more than one depth down
 * goes through the gutter beside it, so it never crosses a box. A link back up or across is not
 * drawn; the box says how many there are, and the detail screen lists them.
 */
export function layoutGraph(goal: Goal): GraphLayout {
  const { byId } = trajectoryTree(goal)
  const index = new Map(goal.nodes.map((n) => [n.id, n]))
  const boxes: Box[] = goal.nodes.map((n) => ({
    id: n.id, x: 2 + n.row * SLOT, y: 1 + n.rank * RANK, w: BOX_W, h: BOX_H, node: n, number: byId.get(n.id)?.number ?? "", backLinks: 0,
  }))
  const at = new Map(boxes.map((b) => [b.id, b]))
  const width = Math.max(...boxes.map((b) => b.x + b.w), 0) + 4
  const height = Math.max(...boxes.map((b) => b.y + b.h), 0) + 2
  const key = (x: number, y: number) => y * (width + 8) + x
  const bits = new Map<number, number>()
  const marks = new Map<number, string>()
  const add = (x: number, y: number, bit: number) => { if (x >= 0 && y >= 0) bits.set(key(x, y), (bits.get(key(x, y)) ?? 0) | bit) }
  const vertical = (x: number, y1: number, y2: number) => {
    for (let y = y1; y <= y2; y++) {
      if (y < y2) add(x, y, D)
      if (y > y1) add(x, y, U)
    }
  }
  const horizontal = (y: number, x1: number, x2: number) => {
    const [a, b] = x1 <= x2 ? [x1, x2] : [x2, x1]
    for (let x = a; x <= b; x++) {
      if (x < b) add(x, y, R)
      if (x > a) add(x, y, L)
    }
  }
  const seen = new Set<string>()
  for (const e of goal.edges) {
    const from = at.get(e.from), to = at.get(e.to)
    const id = `${e.from}>${e.to}`
    if (!from || !to || from === to || !index.has(e.from) || seen.has(id)) continue
    seen.add(id)
    if (to.node.rank <= from.node.rank) { to.backLinks += 1; continue }
    const ax = from.x + (BOX_W >> 1), bx = to.x + (BOX_W >> 1)
    const busA = from.y + BOX_H + 1, busB = to.y - 2
    vertical(ax, from.y + BOX_H, busA)
    if (to.node.rank === from.node.rank + 1) {
      horizontal(busA, ax, bx)
      vertical(bx, busA, to.y - 1)
    } else {
      const gx = to.x - 2
      horizontal(busA, ax, gx)
      vertical(gx, busA, busB)
      horizontal(busB, gx, bx)
      vertical(bx, busB, to.y - 1)
    }
    marks.set(key(bx, to.y - 1), "▼")
  }
  return { boxes, width, height, bits, marks, key }
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

/** The whole picture as cells. The screen shows a window of it. */
export function drawGraph(layout: GraphLayout, selected: string | null): Cell[][] {
  const grid: Cell[][] = Array.from({ length: layout.height }, () => Array.from({ length: layout.width }, () => ({ ch: " ", tone: "edge" as Tone })))
  const put = (x: number, y: number, ch: string, tone: Tone) => { if (grid[y]?.[x]) grid[y]![x] = { ch, tone } }
  for (const [k, bit] of layout.bits) {
    const stride = layout.width + 8
    const y = Math.floor(k / stride), x = k % stride
    put(x, y, JOIN[bit] ?? "┼", "edge")
  }
  for (const [k, ch] of layout.marks) {
    const stride = layout.width + 8
    put(k % stride, Math.floor(k / stride), ch, "edge")
  }
  for (const box of layout.boxes) {
    const tone = box.id === selected ? "select" : toneOf(box.node)
    const inner = box.w - 2
    const label = `${box.number ? `(${box.number}) ` : ""}${box.node.label || box.node.id}${box.backLinks ? ` ↺${box.backLinks}` : ""}`
    const lines = [`╭${"─".repeat(inner)}╮`, `│${clip(label, inner - 2).padEnd(inner - 2)}  │`.slice(0, box.w), `╰${"─".repeat(inner)}╯`]
    lines.forEach((line, row) => [...line].forEach((ch, col) => put(box.x + col, box.y + row, ch, tone)))
    // Where a line joins the box.
    if (layout.bits.has(layout.key(box.x + (BOX_W >> 1), box.y - 1))) put(box.x + (BOX_W >> 1), box.y, "┴", tone)
    if (layout.bits.has(layout.key(box.x + (BOX_W >> 1), box.y + BOX_H))) put(box.x + (BOX_W >> 1), box.y + BOX_H - 1, "┬", tone)
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
  return [...layout.boxes].sort((a, b) => a.node.rank - b.node.rank || a.node.row - b.node.row)
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

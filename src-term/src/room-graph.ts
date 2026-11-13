import type { Goal, GoalNode } from "./room-link.ts"
import { trajectoryTree } from "./room-tree.ts"

/** A box is three cells tall. Its width follows its label, up to this many cells. */
export const BOX_W = 46
export const BOX_H = 3
const MIN_W = 14
/** How far a child sits to the right of its parent. */
const INDENT = 5

export type Box = { id: string; x: number; y: number; w: number; h: number; node: GoalNode; number: string; backLinks: number }
export type Tone = "edge" | "box" | "select" | "success" | "warn" | "error" | "info" | "dim"
export type Cell = { ch: string; tone: Tone }
export type GraphLayout = {
  boxes: Box[]
  width: number
  height: number
  /** Lines between boxes: for each cell, which sides a line leaves through. */
  bits: Map<number, number>
  /** Arrow heads, drawn over the lines. */
  marks: Map<number, string>
  key: (x: number, y: number) => number
}

const U = 1, D = 2, L = 4, R = 8
const JOIN: Record<number, string> = { 1: "│", 2: "│", 3: "│", 4: "─", 8: "─", 12: "─", 9: "└", 5: "┘", 10: "┌", 6: "┐", 11: "├", 7: "┤", 14: "┬", 13: "┴", 15: "┼" }

const labelOf = (node: GoalNode, number: string, extra: number) =>
  `${number ? `(${number}) ` : ""}${node.label || node.id}${extra ? ` ↺${extra}` : ""}`

/**
 * The goal as a tree that grows down the screen, one box under the other, each child a step to
 * the right of its parent: the compact way to see a long chain. A trunk runs down beside a
 * parent's boxes and a short arrow leads into each child. A node sits once, under the first
 * node that reaches it; any other link into it is not drawn, and its box says how many there are
 * (`↺2`). The detail screen lists them.
 */
export function layoutGraph(goal: Goal): GraphLayout {
  const { order } = trajectoryTree(goal)
  const boxes: Box[] = order.map((t, i) => {
    const label = labelOf(t.node, t.number, t.alsoFrom.length)
    const w = Math.min(BOX_W, Math.max(MIN_W, [...label].length + 4))
    return { id: t.node.id, x: 2 + t.depth * INDENT, y: 1 + i * BOX_H, w, h: BOX_H, node: t.node, number: t.number, backLinks: t.alsoFrom.length }
  })
  const at = new Map(boxes.map((b) => [b.id, b]))
  const width = Math.max(...boxes.map((b) => b.x + b.w), 0) + 4
  const height = Math.max(...boxes.map((b) => b.y + b.h), 0) + 2
  const key = (x: number, y: number) => y * (width + 8) + x
  const bits = new Map<number, number>()
  const marks = new Map<number, string>()
  const add = (x: number, y: number, bit: number) => { if (x >= 0 && y >= 0) bits.set(key(x, y), (bits.get(key(x, y)) ?? 0) | bit) }
  // The trunk of each parent starts under it and ends level with its last child; a branch leads into every child.
  for (const t of order) {
    const me = at.get(t.node.id)!
    if (!t.children.length) continue
    const trunk = me.x + 2
    const mids = t.children.map((c) => at.get(c)!.y + 1)
    const end = Math.max(...mids)
    for (let y = me.y + BOX_H; y <= end; y++) {
      if (y < end) add(trunk, y, D)
      if (y > me.y + BOX_H) add(trunk, y, U)
    }
    for (const c of t.children) {
      const child = at.get(c)!
      const mid = child.y + 1
      for (let x = trunk; x <= child.x - 2; x++) {
        if (x < child.x - 2) add(x, mid, R)
        if (x > trunk) add(x, mid, L)
      }
      marks.set(key(child.x - 1, mid), "►")
    }
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
    const label = labelOf(box.node, box.number, box.backLinks)
    const lines = [`╭${"─".repeat(inner)}╮`, `│ ${clip(label, inner - 2).padEnd(inner - 2)} │`, `╰${"─".repeat(inner)}╯`]
    lines.forEach((line, row) => [...line].forEach((ch, col) => put(box.x + col, box.y + row, ch, tone)))
    // Where the trunk leaves the box.
    if (layout.bits.has(layout.key(box.x + 2, box.y + BOX_H))) put(box.x + 2, box.y + BOX_H - 1, "┬", tone)
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
  return [...layout.boxes].sort((a, b) => a.y - b.y || a.x - b.x)
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

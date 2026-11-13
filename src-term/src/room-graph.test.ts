import { expect, test } from "bun:test"
import { BOX_H, BOX_W, boxAt, drawGraph, layoutGraph, pan, readingOrder, reveal, windowLines } from "./room-graph.ts"
import type { Goal } from "./room-link.ts"

const n = (id: string, label: string, rank: number, row: number, kind = "fact", severity = "", status = "") => ({ id, label, kind, status, severity, note: "", resource_key: "", rank, row })
const goal: Goal = {
  goal_key: "g", open: 0, verified: 0, returned: 0,
  nodes: [n("s", "start", 0, 0, "start"), n("a", "asset", 1, 0), n("b", "asset two", 1, 1), n("c", "claim", 2, 0, "claim", "high"), n("f", "finding", 3, 1, "finding", "", "verified")],
  edges: [{ from: "s", to: "a", rel: "maps" }, { from: "s", to: "b", rel: "maps" }, { from: "a", to: "c", rel: "hosts" }, { from: "b", to: "c", rel: "hosts" }, { from: "c", to: "f", rel: "proves" }, { from: "f", to: "s", rel: "back" }],
}

const text = (grid: ReturnType<typeof drawGraph>) => grid.map((row) => row.map((c) => c.ch).join("")).join("\n")

test("every node gets a box at its depth and lane, and none overlap", () => {
  const layout = layoutGraph(goal)
  expect(layout.boxes.length).toBe(5)
  const spot = (id: string) => layout.boxes.find((b) => b.id === id)!
  expect(spot("a").y).toBeGreaterThan(spot("s").y)
  expect(spot("b").x).toBeGreaterThan(spot("a").x)
  expect(spot("c").y).toBeGreaterThan(spot("a").y)
  for (const a of layout.boxes) for (const b of layout.boxes) {
    if (a !== b) expect(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h).toBe(false)
  }
})

test("boxes carry their number and label, lines join them, and arrows point into the child", () => {
  const picture = text(drawGraph(layoutGraph(goal), null))
  expect(picture).toContain("(1) start")
  expect(picture).toContain("(1.1) asset")
  expect(picture).toContain("(1.1.1) claim")
  expect(picture).toContain("╭")
  expect(picture).toContain("▼")
  expect(picture).toMatch(/[┌┐├┬]/)
  expect(picture).toContain("┬")
})

test("a link back up is not drawn but the box says it has one", () => {
  const layout = layoutGraph(goal)
  expect(layout.boxes.find((b) => b.id === "s")!.backLinks).toBe(1)
  expect(text(drawGraph(layout, null))).toContain("↺1")
})

test("a link that skips a depth runs down the gutter and never through a box", () => {
  const skip: Goal = { ...goal, nodes: [...goal.nodes, n("z", "far", 3, 0)], edges: [...goal.edges, { from: "s", to: "z", rel: "long" }] }
  const layout = layoutGraph(skip)
  const grid = drawGraph(layout, null)
  for (const box of layout.boxes) {
    for (let y = box.y + 1; y < box.y + BOX_H - 1; y++) {
      for (let x = box.x + 1; x < box.x + BOX_W - 1; x++) expect("│─┌┐└┘├┤┬┴┼▼".includes(grid[y]![x]!.ch) && grid[y]![x]!.ch !== "│").toBe(false)
    }
  }
})

test("the selected box is drawn in the selection tone, and a click finds its box", () => {
  const layout = layoutGraph(goal)
  const grid = drawGraph(layout, "c")
  const box = layout.boxes.find((b) => b.id === "c")!
  expect(grid[box.y]![box.x]!.tone).toBe("select")
  expect(boxAt(layout, box.x + 3, box.y + 1)?.id).toBe("c")
  expect(boxAt(layout, 0, 0)).toBeNull()
  expect(grid[layout.boxes.find((b) => b.id === "f")!.y]![layout.boxes.find((b) => b.id === "f")!.x]!.tone).toBe("success")
})

test("panning stays inside the picture, and revealing a box brings it into the window", () => {
  const layout = layoutGraph(goal)
  const view = { width: 40, height: 10 }
  expect(pan({ x: 0, y: 0 }, -5, -5, layout, view)).toEqual({ x: 0, y: 0 })
  const far = pan({ x: 0, y: 0 }, 9999, 9999, layout, view)
  expect(far.x).toBe(layout.width - view.width)
  expect(far.y).toBe(layout.height - view.height)
  const box = layout.boxes.find((b) => b.id === "f")!
  const shown = reveal({ x: 0, y: 0 }, box, layout, view)
  expect(box.y >= shown.y && box.y + box.h <= shown.y + view.height).toBe(true)
  expect(box.x >= shown.x && box.x + box.w <= shown.x + view.width).toBe(true)
  expect(readingOrder(layout).map((b) => b.id)).toEqual(["s", "a", "b", "c", "f"])
})

test("a window is the picture cut to size, in runs of one tone", () => {
  const layout = layoutGraph(goal)
  const lines = windowLines(drawGraph(layout, "a"), { x: 0, y: 0 }, { width: 30, height: 8 })
  expect(lines.length).toBe(8)
  expect(lines.every((runs) => runs.reduce((n, r) => n + [...r.text].length, 0) <= 30)).toBe(true)
  expect(lines.flat().some((r) => r.tone === "select")).toBe(true)
})

test("a layer wider than the screen wraps onto another row, so the picture stays narrow", () => {
  const wide: Goal = {
    goal_key: "w", open: 0, verified: 0, returned: 0,
    nodes: [n("s", "start", 0, 0, "start"), ...Array.from({ length: 8 }, (_, i) => n(`a${i}`, `asset ${i}`, 1, i))],
    edges: Array.from({ length: 8 }, (_, i) => ({ from: "s", to: `a${i}`, rel: "" })),
  }
  const roomy = layoutGraph(wide, 240), tight = layoutGraph(wide, 80)
  expect(roomy.width).toBeGreaterThan(160)
  expect(tight.width).toBeLessThanOrEqual(80)
  expect(tight.height).toBeGreaterThan(roomy.height)
  const tops = new Set(tight.boxes.map((b) => b.y))
  expect(tops.size).toBeGreaterThan(2)
  for (const a of tight.boxes) for (const b of tight.boxes) {
    if (a !== b) expect(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h).toBe(false)
  }
})

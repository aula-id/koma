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
  // Depth runs across; the boxes of one depth stack down their column.
  expect(spot("a").x).toBeGreaterThan(spot("s").x)
  expect(spot("b").x).toBe(spot("a").x)
  expect(spot("b").y).toBeGreaterThan(spot("a").y)
  expect(spot("c").x).toBeGreaterThan(spot("a").x)
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
  expect(picture).toContain("►")
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
  const lines = windowLines(drawGraph(layout, "a"), { x: 0, y: 0 }, { width: 30, height: 14 })
  expect(lines.length).toBe(14)
  expect(lines.every((runs) => runs.reduce((n, r) => n + [...r.text].length, 0) <= 30)).toBe(true)
  expect(lines.flat().some((r) => r.tone === "select")).toBe(true)
})

test("depth runs across and one depth stacks down its column", () => {
  const wide: Goal = {
    goal_key: "w", open: 0, verified: 0, returned: 0,
    nodes: [n("s", "start", 0, 0, "start"), ...Array.from({ length: 6 }, (_, i) => n(`a${i}`, `asset ${i}`, 1, i))],
    edges: Array.from({ length: 6 }, (_, i) => ({ from: "s", to: `a${i}`, rel: "" })),
  }
  const layout = layoutGraph(wide)
  const s = layout.boxes.find((b) => b.id === "s")!
  const column = layout.boxes.filter((b) => b.id !== "s")
  expect(new Set(column.map((b) => b.x)).size).toBe(1)
  expect(column[0]!.x).toBeGreaterThan(s.x)
  expect(new Set(column.map((b) => b.y)).size).toBe(6)
  expect(layout.height).toBeGreaterThan(layout.width / 2)
})

test("two goals with the same name get distinct names, so a row is selected and opened alone", async () => {
  const { uniqueGoals } = await import("./room-link.ts")
  const { goalRows } = await import("./room-tree.ts")
  const one = { ...goal, goal_key: "audit" }
  const goals = uniqueGoals([one, { ...one }, { ...one }, { ...goal, goal_key: "other" }])
  expect(goals.map((g) => g.goal_key)).toEqual(["audit", "audit (2)", "audit (3)", "other"])
  const ids = goalRows(goals, new Set()).map((r) => r.id)
  expect(new Set(ids).size).toBe(ids.length)
})

test("a link that crosses another is dotted, so each one can be followed", () => {
  const cross: Goal = {
    goal_key: "x", open: 0, verified: 0, returned: 0,
    nodes: [n("a", "A", 0, 0), n("b", "B", 0, 1), n("c", "C", 0, 2), n("x", "X", 1, 0), n("y", "Y", 1, 1), n("z", "Z", 1, 2)],
    edges: [{ from: "a", to: "x", rel: "" }, { from: "b", to: "y", rel: "" }, { from: "c", to: "z", rel: "" }, { from: "a", to: "z", rel: "" }],
  }
  const layout = layoutGraph(cross)
  expect(layout.dots.size).toBeGreaterThan(0)
  const picture = text(drawGraph(layout, null))
  expect(picture).toContain("┆")
  // Links that only share an end merge, and stay solid.
  expect(layoutGraph(goal).dots.size).toBe(0)
})

import { expect, test } from "bun:test"
import { around, fields, goalRows, move, parentOf, press, reportRows, shelfRows, toggled, windowOf } from "./room-tree.ts"
import type { Goal, ShelfRow } from "./room-link.ts"

const row = (key: string, title = "", status = ""): ShelfRow => ({ key, revision: 1, updated: 0, kind: "", title, status })
const shelf = [row("claims/web/ssrf", "SSRF in fetch", "open"), row("claims/web/xss", "XSS"), row("claims/idor", "IDOR"), row("assets/app", "App"), row("notes/plan")]

test("the shelf reads as folders, folders first, with counts", () => {
  const rows = shelfRows(shelf, new Set())
  expect(rows.map((r) => `${"  ".repeat(r.depth)}${r.label}`)).toEqual([
    "assets/", "  app", "claims/", "  web/", "    ssrf", "    xss", "  idor", "notes/", "  plan",
  ])
  expect(rows.find((r) => r.id === "dir:claims/")!.mark).toBe("3")
  expect(rows.find((r) => r.id === "item:claims/web/ssrf")!.detail).toBe("SSRF in fetch")
})

test("a collapsed folder hides what is inside it, and the others stay open", () => {
  const rows = shelfRows(shelf, new Set(["dir:claims/"]))
  expect(rows.map((r) => r.label)).toEqual(["assets/", "app", "claims/", "notes/", "plan"])
  expect(rows.find((r) => r.id === "dir:claims/")!.open).toBe(false)
})

const goals: Goal[] = [
  { goal_key: "g1", open: 1, verified: 0, returned: 0, nodes: [{ id: "a", label: "Map", kind: "fact", status: "", severity: "", note: "", resource_key: "", rank: 0, row: 0 }, { id: "b", label: "SSRF", kind: "claim", status: "open", severity: "high", note: "n", resource_key: "claims/x", rank: 1, row: 0 }], edges: [{ from: "a", to: "b", rel: "leads_to" }] },
  { goal_key: "g2", open: 0, verified: 1, returned: 0, nodes: [], edges: [] },
]

test("each goal is a folder of its nodes and can be collapsed on its own", () => {
  expect(goalRows(goals, new Set()).map((r) => r.id)).toEqual(["goal:g1", "node:g1:a", "node:g1:b", "goal:g2"])
  expect(goalRows(goals, new Set(["goal:g1"])).map((r) => r.id)).toEqual(["goal:g1", "goal:g2"])
  expect(goalRows(goals, new Set())[0]!.detail).toBe("1 open · 0 verified · 0 returned")
})

test("keys move, open and close, and Enter opens a leaf", () => {
  const rows = goalRows(goals, new Set())
  expect(press(rows, "goal:g1", new Set(), "down").selected).toBe("node:g1:a")
  expect(press(rows, "goal:g1", new Set(), "enter").collapsed.has("goal:g1")).toBe(true)
  const closed = goalRows(goals, new Set(["goal:g1"]))
  expect(press(closed, "goal:g1", new Set(["goal:g1"]), "right").collapsed.has("goal:g1")).toBe(false)
  expect(press(rows, "goal:g1", new Set(), "right").selected).toBe("node:g1:a")
  const leaf = press(rows, "node:g1:b", new Set(), "enter")
  expect(leaf.open?.ref).toEqual({ type: "node", goal: "g1", node: "b" })
  expect(press(rows, "node:g1:b", new Set(), "left").selected).toBe("goal:g1")
  expect(move(rows, "goal:g2", 1)).toBe("goal:g2")
  expect(parentOf(rows, "node:g1:a")).toBe("goal:g1")
  expect(toggled(new Set(["x"]), "x").size).toBe(0)
})

test("the window follows the highlight", () => {
  const rows = shelfRows(shelf, new Set())
  expect(windowOf(rows, rows[0]!.id, 0, 3).rows.length).toBe(3)
  const low = windowOf(rows, rows.at(-1)!.id, 0, 3)
  expect(low.rows.at(-1)!.id).toBe(rows.at(-1)!.id)
  expect(low.from).toBe(rows.length - 3)
})

test("a node knows what leads to it and what it leads to", () => {
  const near = around(goals[0]!, "b")
  expect(near.from.map((e) => e.node.id)).toEqual(["a"])
  expect(near.to).toEqual([])
})

test("a record reads as fields, with the important ones first and empties left out", () => {
  const out = fields(JSON.stringify({ updated: 1, evidence_keys: [], poc: "curl x", name: "SSRF", severity: "high", cve_ids: ["CVE-1", "CVE-2"] }))!
  expect(out.map((f) => f.name)).toEqual(["name", "severity", "poc", "cve_ids"])
  expect(out.at(-1)!.value).toBe("CVE-1, CVE-2")
  expect(fields("just text")).toBeNull()
  expect(reportRows([{ key: "reports/a", revision: 1, updated: 0, title: "", audience: "manager", severity_summary: "1 high", go_no_go: "no-go", finding_keys: [], markdown: "" }])[0]!.mark).toBe("no-go")
})

import { expect, test } from "bun:test"
import { describe, fields, trajectoryTree, goalRows, move, parentOf, press, reportRows, shelfRows, toggled, windowOf } from "./room-tree.ts"
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

const n = (id: string, label: string, kind = "fact", rank = 0, row = 0, resource_key = "", severity = "", status = "") => ({ id, label, kind, status, severity, note: "", resource_key, rank, row })
const goals: Goal[] = [
  { goal_key: "g1", open: 1, verified: 0, returned: 0,
    nodes: [n("s", "start", "start"), n("sc", "scope", "scope", 1, 0), n("a1", "asset one", "asset", 1, 1), n("a2", "asset two", "asset", 1, 2), n("c", "SSRF claim", "claim", 2, 0, "claims/x", "high"), n("e", "output", "evidence", 3, 0), n("v", "verdict", "verdict", 3, 1)],
    edges: [{ from: "s", to: "sc", rel: "scope" }, { from: "s", to: "a1", rel: "maps" }, { from: "s", to: "a2", rel: "maps" }, { from: "a1", to: "c", rel: "hosts" }, { from: "a2", to: "c", rel: "hosts" }, { from: "c", to: "e", rel: "supports" }, { from: "c", to: "v", rel: "checks" }] },
  { goal_key: "g2", open: 0, verified: 1, returned: 0, nodes: [], edges: [] },
]

test("a goal reads as a numbered tree of what links to what", () => {
  const { order } = trajectoryTree(goals[0]!)
  expect(order.map((t) => `${t.number} ${t.node.id}`)).toEqual(["1 s", "1.1 sc", "1.2 a1", "1.2.1 c", "1.2.1.1 e", "1.2.1.2 v", "1.3 a2"])
})

test("a node with two parents sits once and lists the other parent as a reference", () => {
  const rows = goalRows(goals, new Set())
  const claim = rows.find((r) => r.id === "node:g1:c")!
  expect(claim.number).toBe("1.2.1")
  expect(claim.refs).toEqual(["1.3"])
  expect(rows.filter((r) => r.label === "SSRF claim").length).toBe(1)
  expect(rows.find((r) => r.id === "node:g1:a2")!.refs).toEqual([])
})

test("the rows carry branch glyphs that follow the tree", () => {
  const rows = goalRows(goals, new Set())
  const guide = (id: string) => rows.find((r) => r.id === `node:g1:${id}`)!.guide
  expect(guide("s")).toBe("└─")
  expect(guide("sc")).toBe("   ├─")
  expect(guide("a1")).toBe("   ├─")
  expect(guide("c")).toBe("   │  └─")
  expect(guide("e")).toBe("   │     ├─")
  expect(guide("a2")).toBe("   └─")
})

test("everything is open by default, a node folds its subtree, and a goal folds itself", () => {
  expect(goalRows(goals, new Set()).length).toBe(1 + 7 + 1)
  const folded = goalRows(goals, new Set(["node:g1:a1"]))
  expect(folded.map((r) => r.id)).not.toContain("node:g1:c")
  expect(folded.find((r) => r.id === "node:g1:a1")!.open).toBe(false)
  expect(goalRows(goals, new Set(["goal:g1"])).map((r) => r.id)).toEqual(["goal:g1", "goal:g2"])
})

test("a node with children opens its detail on enter and folds on space or left", () => {
  const rows = goalRows(goals, new Set())
  expect(press(rows, "node:g1:a1", new Set(), "enter").open?.ref).toEqual({ type: "node", goal: "g1", node: "a1" })
  expect(press(rows, "node:g1:a1", new Set(), "space").collapsed.has("node:g1:a1")).toBe(true)
  expect(press(rows, "node:g1:a1", new Set(), "left").collapsed.has("node:g1:a1")).toBe(true)
  expect(press(rows, "node:g1:c", new Set(), "left").selected).toBe("node:g1:c")
})

test("a loop with no way in is still drawn, and a link back is a reference", () => {
  const loop: Goal = { goal_key: "l", open: 0, verified: 0, returned: 0, nodes: [n("a", "A"), n("b", "B", "fact", 1)], edges: [{ from: "a", to: "b", rel: "" }, { from: "b", to: "a", rel: "" }] }
  const { order } = trajectoryTree(loop)
  expect(order.map((t) => t.number)).toEqual(["1", "1.1"])
  expect(order[0]!.alsoFrom).toEqual(["1.1"])
})

test("a node's detail names where it is referenced from, what it references and its relatives", () => {
  const near = describe(goals[0]!, "c")!
  expect(near.number).toBe("1.2.1")
  expect(near.from.map((e) => `${e.number} ${e.rel}`)).toEqual(["1.2 hosts", "1.3 hosts"])
  expect(near.to.map((e) => e.node.id)).toEqual(["e", "v"])
  expect(near.related).toEqual([])
  expect(describe(goals[0]!, "e")!.related.map((r) => r.node.id)).toEqual(["v"])
  expect(describe(goals[0]!, "nope")).toBeNull()
})

test("keys move, open and close, and Enter opens a leaf", () => {
  const rows = goalRows(goals, new Set())
  expect(press(rows, "goal:g1", new Set(), "down").selected).toBe("node:g1:s")
  expect(press(rows, "goal:g1", new Set(), "enter").collapsed.has("goal:g1")).toBe(true)
  const closed = goalRows(goals, new Set(["goal:g1"]))
  expect(press(closed, "goal:g1", new Set(["goal:g1"]), "right").collapsed.has("goal:g1")).toBe(false)
  const leaf = press(rows, "node:g1:e", new Set(), "enter")
  expect(leaf.open?.ref).toEqual({ type: "node", goal: "g1", node: "e" })
  expect(press(rows, "node:g1:e", new Set(), "left").selected).toBe("node:g1:c")
  expect(move(rows, "goal:g2", 1)).toBe("goal:g2")
  expect(parentOf(rows, "node:g1:sc")).toBe("node:g1:s")
  expect(toggled(new Set(["x"]), "x").size).toBe(0)
})

test("the window follows the highlight", () => {
  const rows = shelfRows(shelf, new Set())
  expect(windowOf(rows, rows[0]!.id, 0, 3).rows.length).toBe(3)
  const low = windowOf(rows, rows.at(-1)!.id, 0, 3)
  expect(low.rows.at(-1)!.id).toBe(rows.at(-1)!.id)
  expect(low.from).toBe(rows.length - 3)
})

test("a record reads as fields, with the important ones first and empties left out", () => {
  const out = fields(JSON.stringify({ updated: 1, evidence_keys: [], poc: "curl x", name: "SSRF", severity: "high", cve_ids: ["CVE-1", "CVE-2"] }))!
  expect(out.map((f) => f.name)).toEqual(["name", "severity", "poc", "cve_ids"])
  expect(out.at(-1)!.value).toBe("CVE-1, CVE-2")
  expect(fields("just text")).toBeNull()
  expect(reportRows([{ key: "reports/a", revision: 1, updated: 0, title: "", audience: "manager", severity_summary: "1 high", go_no_go: "no-go", finding_keys: [], markdown: "" }])[0]!.mark).toBe("no-go")
})

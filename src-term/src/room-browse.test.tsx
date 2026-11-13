import { expect, test } from "bun:test"
import { testRender } from "@opentui/react/test-utils"
import { act, useMemo, type ComponentProps } from "react"
import { useRenderer } from "@opentui/react"
import { SyntaxStyle } from "@opentui/core"
import { BrowseScreen, type BrowseData, type BrowseView, type OpenItem } from "./room-browse.tsx"
import { themeFor } from "./theme.ts"

const theme = themeFor("dark")
const data: BrowseData = {
  shelf: [
    { key: "claims/web/ssrf", revision: 3, updated: 0, kind: "claim", title: "SSRF in fetch", status: "open" },
    { key: "claims/idor", revision: 1, updated: 0, kind: "claim", title: "IDOR on orders", status: "" },
    { key: "assets/app", revision: 1, updated: 0, kind: "asset", title: "App server", status: "" },
  ],
  goals: [
    { goal_key: "api", open: 1, verified: 0, returned: 0, edges: [{ from: "a", to: "b", rel: "leads_to" }], nodes: [
      { id: "a", label: "Map the api", kind: "fact", status: "", severity: "", note: "", resource_key: "", rank: 0, row: 0 },
      { id: "b", label: "Try SSRF", kind: "claim", status: "open", severity: "high", note: "fetch takes a url", resource_key: "claims/web/ssrf", rank: 1, row: 0 }] },
    { goal_key: "web", open: 0, verified: 1, returned: 0, edges: [], nodes: [{ id: "w", label: "Crawl", kind: "fact", status: "", severity: "", note: "", resource_key: "", rank: 0, row: 0 }] },
  ],
  reports: [{ key: "reports/final", revision: 2, updated: 0, title: "Final report", audience: "manager", severity_summary: "1 high", go_no_go: "no-go", finding_keys: ["findings/ssrf"], markdown: "# Summary\n\nOne high finding." }],
}
const ssrf: OpenItem = { key: "claims/web/ssrf", revision: 3, updated: 0, content: JSON.stringify({ kind: "claim", name: "SSRF in fetch", severity: "high", summary: "fetch follows redirects", evidence_keys: ["evidence/ssrf-1"] }) }

type View = Awaited<ReturnType<typeof testRender>>

/** The screen with a syntax style made from the live renderer, as the room makes it. */
function Host(props: Omit<ComponentProps<typeof BrowseScreen>, "syntaxStyle">) {
  const renderer = useRenderer()
  const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
  return <BrowseScreen {...props} syntaxStyle={style} />
}

async function open(view: BrowseView, extra: { item?: OpenItem | null; opened?: string[] } = {}) {
  const opened = extra.opened ?? []
  let screen!: View
  await act(async () => {
    screen = await testRender(<Host view={view} theme={theme} data={data} item={extra.item ?? null} onOpenItem={(k) => opened.push(k)} onRefresh={() => {}} onBack={() => opened.push("back")} />, { width: 100, height: 30 })
  })
  const settle = async () => { await act(async () => { await new Promise((r) => setTimeout(r, 40)); await screen.renderOnce() }) }
  await settle()
  return { screen, opened, settle, text: () => screen.captureCharFrame() }
}

test("the shelf is a folder tree, and a folder folds on enter", async () => {
  const t = await open("shelf")
  expect(t.text()).toContain("▾ claims/")
  expect(t.text()).toContain("ssrf")
  expect(t.text()).toContain("SSRF in fetch")
  await act(async () => { t.screen.mockInput.pressArrow("down") })
  await act(async () => { t.screen.mockInput.pressArrow("down") })
  await t.settle()
  await act(async () => { await t.screen.mockInput.pressEnter() })
  await t.settle()
  expect(t.text()).toContain("▸ claims/")
  expect(t.text()).not.toContain("IDOR on orders")
  await act(async () => { await t.screen.dispose() })
})

test("enter on a shelf item asks for it and opens its detail screen", async () => {
  const t = await open("shelf", { item: ssrf })
  for (let i = 0; i < 4; i++) { await act(async () => { t.screen.mockInput.pressArrow("down") }); await t.settle() }
  await act(async () => { await t.screen.mockInput.pressEnter() })
  await t.settle()
  expect(t.opened).toEqual(["claims/web/ssrf"])
  const text = t.text()
  expect(text).toContain("claims/web/ssrf")
  expect(text).toContain("fetch follows redirects")
  expect(t.opened).toContain("claims/web/ssrf")
  expect(text).toContain("evidence/ssrf-1")
  expect(text).toContain("links (1)")
  await act(async () => { t.screen.mockInput.pressEscape(); await new Promise((r) => setTimeout(r, 40)) })
  await t.settle()
  expect(t.text()).toContain("▾ claims/")
  expect(t.opened).toEqual(["claims/web/ssrf"])
  await act(async () => { await t.screen.dispose() })
})

test("goals fold one at a time and a node opens with the steps around it", async () => {
  const t = await open("trajectory", { item: ssrf })
  expect(t.text()).toContain("api")
  expect(t.text()).toContain("1 open · 0 verified · 0 returned")
  expect(t.text()).toContain("Map the api")
  expect(t.text()).toContain("Crawl")
  await act(async () => { await t.screen.mockInput.pressEnter() })
  await t.settle()
  expect(t.text()).not.toContain("Map the api")
  expect(t.text()).toContain("Crawl")
  await act(async () => { await t.screen.mockInput.pressEnter() })
  await t.settle()
  for (let i = 0; i < 2; i++) { await act(async () => { t.screen.mockInput.pressArrow("down") }); await t.settle() }
  await act(async () => { await t.screen.mockInput.pressEnter() })
  await t.settle()
  const text = t.text()
  expect(text).toContain("Try SSRF")
  expect(text).toContain("fetch takes a url")
  expect(text).toContain("fetch follows redirects")
  expect(text).toContain("claims/web/ssrf")
  expect(text).toContain("Map the api")
  await act(async () => { await t.screen.dispose() })
})

test("reports list with their verdict and open to the whole report", async () => {
  const t = await open("report")
  expect(t.text()).toContain("Final report")
  expect(t.text()).toContain("no-go")
  expect(t.text()).toContain("manager")
  await act(async () => { await t.screen.mockInput.pressEnter() })
  // Markdown draws a frame after the screen does.
  for (let i = 0; i < 4; i++) { await t.settle(); await act(async () => { await t.screen.waitForVisualIdle() }) }
  const text = t.text()
  expect(text).toContain("One high finding.")
  expect(text).toContain("findings/ssrf")
  await act(async () => { await t.screen.dispose() })
})

test("an empty shelf says so, and escape closes the screen", async () => {
  let screen!: View
  const log: string[] = []
  await act(async () => { screen = await testRender(<Host view="report" theme={theme} data={{ shelf: [], goals: [], reports: [] }} item={null} onOpenItem={() => {}} onRefresh={() => {}} onBack={() => log.push("back")} />, { width: 90, height: 24 }) })
  await act(async () => { await new Promise((r) => setTimeout(r, 40)); await screen.renderOnce() })
  expect(screen.captureCharFrame()).toContain("No report yet")
  await act(async () => { screen.mockInput.pressEscape(); await new Promise((r) => setTimeout(r, 40)) })
  expect(log).toEqual(["back"])
  await act(async () => { await screen.dispose() })
})

const node = (id: string, label: string, rank: number, row: number, kind = "fact", severity = "", resource_key = "") => ({ id, label, kind, status: "", severity, note: "", resource_key, rank, row })
const rich: BrowseData = {
  shelf: [], reports: [],
  goals: [{
    goal_key: "review", open: 1, verified: 0, returned: 0,
    nodes: [node("s", "start in-scope", 0, 0, "start"), node("a1", "asset one", 1, 0, "asset"), node("a2", "asset two", 1, 4, "asset"), node("c", "SSRF claim", 2, 0, "claim", "high", "claims/web/ssrf"), node("e", "output", 3, 0, "evidence")],
    edges: [{ from: "s", to: "a1", rel: "maps" }, { from: "s", to: "a2", rel: "maps" }, { from: "a1", to: "c", rel: "hosts" }, { from: "a2", to: "c", rel: "hosts" }, { from: "c", to: "e", rel: "supports" }],
  }],
}

async function openRich() {
  let screen!: View
  const opened: string[] = []
  await act(async () => { screen = await testRender(<Host view="trajectory" theme={theme} data={rich} item={null} onOpenItem={(k) => opened.push(k)} onRefresh={() => {}} onBack={() => opened.push("back")} />, { width: 110, height: 34 }) })
  const settle = async () => { await act(async () => { await new Promise((r) => setTimeout(r, 40)); await screen.renderOnce() }) }
  const press = async (run: () => unknown) => { await act(async () => { await run() }); await settle() }
  await settle()
  return { screen, opened, settle, press, text: () => screen.captureCharFrame() }
}

test("the trajectory is a numbered tree and a node linked twice shows where else it is linked from", async () => {
  const t = await openRich()
  const text = t.text()
  expect(text).toContain("(1) start in-scope")
  expect(text).toContain("(1.1) asset one")
  expect(text).toContain("(1.1.1) SSRF claim")
  expect(text).toContain("[(1.2)]")
  expect(text).toContain("├─")
  expect(text).toContain("└─")
  expect(text.match(/SSRF claim/g)?.length).toBe(1)
  await act(async () => { await t.screen.dispose() })
})

test("a node's detail lists what it is referenced from, what it references and its relatives", async () => {
  const t = await openRich()
  for (let i = 0; i < 3; i++) await t.press(() => t.screen.mockInput.pressArrow("down"))
  await t.press(() => t.screen.mockInput.pressEnter())
  const text = t.text()
  expect(text).toContain("(1.1.1)")
  expect(text).toContain("reference from · hosts")
  expect(text).toContain("reference to · supports")
  expect(text).toContain("claims/web/ssrf")
  await act(async () => { await t.screen.dispose() })
})

test("g draws the graph in boxes, arrows and scrolls sideways, and t comes back to the tree", async () => {
  const t = await openRich()
  await t.press(() => t.screen.mockInput.pressKey("g"))
  const graph = t.text()
  expect(graph).toContain("trajectory · graph")
  expect(graph).toContain("╭")
  expect(graph).toContain("▼")
  expect(graph).toContain("(1.1.1) SSRF claim")
  expect(graph).toMatch(/x 0\/\d+/)
  await t.press(() => t.screen.mockInput.pressArrow("right"))
  expect(t.text()).toMatch(/x 6\//)
  await t.press(() => t.screen.mockInput.pressKey("home"))
  expect(t.text()).toMatch(/x 0\//)
  await t.press(() => t.screen.mockInput.pressKey("t"))
  expect(t.text()).not.toContain("trajectory · graph")
  expect(t.text()).toContain("(1.1) asset one")
  await act(async () => { await t.screen.dispose() })
})

test("in the graph tab walks the nodes and enter opens the one that is chosen", async () => {
  const t = await openRich()
  await t.press(() => t.screen.mockInput.pressKey("g"))
  for (let i = 0; i < 3; i++) await t.press(() => t.screen.mockInput.pressTab())
  await t.press(() => t.screen.mockInput.pressEnter())
  const text = t.text()
  expect(text).toContain("SSRF claim")
  expect(text).toContain("reference to · supports")
  await act(async () => { t.screen.mockInput.pressEscape(); await new Promise((r) => setTimeout(r, 100)) })
  await t.settle()
  expect(t.text()).toContain("trajectory · graph")
  await act(async () => { await t.screen.dispose() })
})

test("a long detail scrolls with page down and wheel keys and starts at the top", async () => {
  const long: OpenItem = { key: "claims/web/ssrf", revision: 1, updated: 0, content: JSON.stringify({ kind: "claim", name: "SSRF", summary: Array.from({ length: 80 }, (_, i) => `line ${i}`).join("\n") }) }
  const t = await open("shelf", { item: long })
  for (let i = 0; i < 4; i++) { await act(async () => { t.screen.mockInput.pressArrow("down") }); await t.settle() }
  await act(async () => { await t.screen.mockInput.pressEnter() }); await t.settle()
  // The item's detail is open at its first line.
  expect(t.text()).toContain("line 0")
  expect(t.text()).not.toContain("line 70")
  await act(async () => { t.screen.mockInput.pressKey("END") }); await t.settle()
  expect(t.text()).toContain("line 79")
  await act(async () => { t.screen.mockInput.pressKey("HOME") }); await t.settle()
  expect(t.text()).toContain("line 0")
  await act(async () => { await t.screen.dispose() })
})

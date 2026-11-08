import { OAUTH_OPTIONS } from "./data.ts"
import { expect, test } from "bun:test"
import { testRender } from "@opentui/react/test-utils"
import { act, useMemo } from "react"
import { useRenderer } from "@opentui/react"
import { SyntaxStyle } from "@opentui/core"
import { Chat } from "../chat.tsx"
import { controlOf, fold } from "../protocol.ts"
import { themeFor } from "../theme.ts"
import { Fullscreen, Lines, Panel, Split } from "./bits.tsx"
import { ModeSurface } from "./surface.tsx"
import { selectedOf } from "./place.ts"

const theme = themeFor("github dark")
const items = Array.from({ length: 60 }, (_, i) => ({ label: `Item ${String(i).padStart(2, "0")}` }))

test("lists expand after a terminal resize and keep the footer visible", async () => {
  const view = await testRender(<Fullscreen title="test" hint="Esc back" theme={theme}><Panel title="items" hot theme={theme}><Lines rows={items} selected={4} theme={theme} onPress={() => {}} /></Panel></Fullscreen>, { width: 80, height: 18 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    const before = view.captureCharFrame()
    await act(async () => { view.resize(80, 36); await view.waitForVisualIdle() })
    await act(async () => { await view.waitForVisualIdle() })
    const after = view.captureCharFrame()
    expect(after.match(/Item \d+/g)!.length).toBeGreaterThan(before.match(/Item \d+/g)!.length)
    expect(after).toContain("Item 20")
    expect(after).toContain("Esc back")
  } finally { await act(async () => { await view.dispose() }) }
})

test("narrow split retains both navigation and detail", async () => {
  const view = await testRender(<Fullscreen title="test" hint="Esc back" theme={theme}><Split wide={false} theme={theme} list={<Lines rows={items} selected={55} theme={theme} onPress={() => {}} />} detail={<text>Selected item detail</text>} /></Fullscreen>, { width: 50, height: 30 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    await act(async () => { await view.waitForVisualIdle() })
    const frame = view.captureCharFrame()
    expect(frame).toContain("Item 55")
    expect(frame).toContain("Selected item detail")
    expect(frame).toContain("Esc back")
  } finally { await act(async () => { await view.dispose() }) }
})

const fullScreens: [string, Record<string, unknown>][] = [
  ["Settings", { page: "quick", quick_focus: "general", field: 6, palette: "github dark", palette_sel: 9 }],
  ["Settings", { page: "quick", quick_focus: "general", field: 11, editing: true, subagent_max_turns: 321, palette: "dark", palette_sel: 0 }],
  ["Settings", { page: "quick", quick_focus: "appearance", palette_sel: 9, palette: "dark" }],
  ["Settings", { page: "quick", search_editor: { provider: "tavily", key: "••••" }, web_search: { provider: "built_in" } }],
  ["Settings", { page: "quick", picker: { query: "src", matches: items.map((row) => row.label), sel: 30 } }],
  ["Settings", { page: "providers", providers: [{ name: "Example", endpoint: "https://example.test/v1" }], prov_sel: 1 }],
  ["Settings", { page: "provider_form", prov_modal: { name: "Example", endpoint: "https://example.test/v1", api_key: "••••", field: 2 } }],
  ["Settings", { page: "models", model_filter: "all", model_sel: 5, models: [{ name: "Test model", model_id: "model-1", roles: ["main"] }] }],
  ["Settings", { page: "model_form", model_fields: ["name", "provider", "model", "role", "save", "cancel"], model_modal: { name: "Test model", field: 2, query: "model", result_sel: 1 }, model_results: ["model-1", "model-2"] }],
  ["Settings", { page: "oauth", oauth_flow: { kind: "pick", cursor: 7, options: OAUTH_OPTIONS } }],
  ["Settings", { page: "oauth", oauth_flow: { kind: "codex_wait", url: "https://example.test/auth", provider: "codex" } }],
  ["Help", { all: [{ key: "/settings", desc: "Manage settings" }], selected: 0 }],
  ["Agents", { agents: [{ name: "Test agent", description: "Test description" }], list_sel: 0 }],
  ["Mcp", { mode: "create", field: "command", draft_transport: "stdio", draft_command: "example" }],
  ["Extensions", { mode: "browse", rows: [{ name: "Test extension", description: "An extension" }], list_sel: 0 }],
  ["ExtStore", { mode: "browse", rows: [{ name: "Test listing" }], list_sel: 0 }],
  ["ExtScreen", { screen: { title: "Test screen", body: [{ t: "menu", items: [{ id: "one", label: "Option one" }] }] } }],
  ["Security", { status: { running: true, installed: true, tools: [{ name: "Tool one", domain: "filesystem" }] }, selected: 0, catalog: [{ id: "nmap", blurb: "scan", image: "recon", exec: "net", domain: "recon", ready_mark: "remote" }] }],
  ["Remote", { sub: "manage:browse", hosts: [{ name: "Test host", user: "user", host: "example.test", port: 22 }], filtered: [0], selected: 0 }],
  ["Onboard", { cursor: 1 }],
  ["OnboardProvider", { step: "login", oauth_flow: { kind: "pick", cursor: 7, options: OAUTH_OPTIONS } }],
  ["OnboardProvider", { step: "model_select", provider: "codex", results: ["model-1", "model-2"], result_sel: 1 }],
  ["KeyInput", { step: 0, field: 1, endpoint: "https://example.test/v1", api_key: "sample" }],
  ["KeyInput", { step: 1, results: ["model-1"], query: "model", result_sel: 0 }],
  ["SessionPicker", { all: [{ name: "Test session", id: "test" }], selected: 0 }],
  ["SessionHub", { cooking: [{ name: "Test session", kind: "session" }], history: [{ name: "Saved session" }], focus_cooking: true }],
  ["SessionHub", { cooking: [{ name: "Test session", kind: "session" }], history: [{ name: "Saved session" }], focus_cooking: false }],
  ["Usage", { view: "global", metric: "tokens", data: { totals: { calls: 3, cost: 0.02 }, heatmap_buckets: [{ bucket_epoch: Math.floor(Date.now() / 3600000) * 3600, tokens: 42, cost: 0.02 }] } }],
  ["Attachments", { editor: [1, { lines: Array.from({ length: 60 }, (_, i) => `Editor line ${i}`), row: 40, col: 3, scroll: 20 }] }],
]

for (const [width, height] of [[50, 24], [80, 18], [100, 40]]) {
  test(`ported full-screen views retain their content and actions at ${width}×${height}`, async () => {
    for (const [name, body] of fullScreens) {
      const view = await testRender(<box flexGrow={1} paddingLeft={2} paddingRight={2}><ModeSurface mode={{ [name]: body }} theme={theme} columns={width} onMove={() => {}} send={() => {}} /></box>, { width, height })
      try {
        // Layout measurement can resize the dashboard, then its list window.
        for (let pass = 0; pass < 4; pass++) await act(async () => { await view.waitForVisualIdle() })
        const frame = view.captureCharFrame()
        expect(frame, `${name}: ${JSON.stringify(body)}`).not.toContain("Error:")
        expect(frame).not.toContain("nothing to show")
        expect(frame).toMatch(/\(esc\)|\(q\)/i)
        if (name === "Settings" && body.page === "quick" && !body.search_editor && !body.picker && width === 100) {
          if (!body.editing) expect(frame).toContain("Session name")
          expect(frame).toContain("Max out tokens")
        }
        if (body.editing) {
          expect(frame).toContain("Max turns")
          expect(frame).toContain("321")
          expect(frame).toContain("done (enter)")
          expect(frame).toContain("cancel (esc)")
        }
        if (body.quick_focus === "appearance") expect(frame).toContain("github dark")
        if (body.search_editor) {
          expect(frame).toContain("Tavily API key")
          expect(frame).toContain("Save search provider")
          expect(frame).not.toContain("type the key")
        }
        if (body.picker) expect(frame).toContain("Item 30")
        if (name === "Security") {
          expect(frame).toContain("Dashboard")
          expect(frame).toContain("Images")
          expect(frame).toContain("Refused")
          expect(frame).toContain("nmap")
          expect(frame).not.toContain("Deps")
        }
        if (name === "OnboardProvider" && body.step === "login") expect(frame).toContain("Command Code (paste key)")
        if (name === "Usage") expect(frame).toContain("42 tokens")
        if (name === "SessionHub") {
          expect(frame).toContain(body.focus_cooking ? "Test session" : "Saved session")
          expect(frame).not.toContain(body.focus_cooking ? "Saved session" : "Test session")
          expect(frame).toContain("Cooking")
          expect(frame).toContain("History")
        }
        if (name === "Attachments") expect(frame).toContain("Line 41 / 60")
      } finally { await act(async () => { await view.dispose() }) }
    }
  })
}

test("active API-key input accepts paste and visible save submits", async () => {
  const requests: unknown[] = []
  const view = await testRender(<ModeSurface mode={{ Settings: { page: "quick", search_editor: { provider: "tavily", key: "", editing: true, confirmed: true } } }} theme={theme} columns={80} onMove={() => {}} send={(request) => requests.push(request)} />, { width: 80, height: 30 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    await act(async () => { await view.mockInput.pasteBracketedText("pasted-test-key") })
    expect(requests).toContainEqual({ MenuReplace: { field: "search_key", value: "pasted-test-key" } })
    const lines = view.captureCharFrame().split("\n")
    const y = lines.findIndex((line) => line.includes("Save search provider"))
    const x = lines[y]!.indexOf("Save search provider") + 3
    await act(async () => { await view.mockMouse.click(x, y) })
    expect(requests).toContainEqual({ MenuAction: "Save" })
  } finally { await act(async () => { await view.dispose() }) }
})

test("nested settings selection takes precedence over quick dashboard focus", () => {
  expect(selectedOf({ Settings: { page: "quick", field: 6, picker: { sel: 30 } } })).toBe(30)
  expect(selectedOf({ Settings: { page: "quick", field: 6, search_editor: { provider: "tavily" } } })).toBe(2)
  expect(selectedOf({ Settings: { page: "quick", field: 6, list_editing: true, list_sel: 4 } })).toBe(4)
})

test("long extension menus scroll to the selected action", async () => {
  const view = await testRender(<ModeSurface mode={{ ExtScreen: { menu_cursor: 55, screen: { title: "Long menu", body: [{ t: "text", text: "Choose an action" }, { t: "menu", items: items.map((row, i) => ({ id: String(i), label: row.label })) }] } } }} theme={theme} columns={80} onMove={() => {}} send={() => {}} />, { width: 80, height: 24 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    await act(async () => { await view.waitForVisualIdle() })
    const frame = view.captureCharFrame()
    expect(frame).toContain("Item 55")
    expect(frame).toContain("Select (enter)")
  } finally { await act(async () => { await view.dispose() }) }
})

test("chat keeps its composer, command palette and compact model overlay", async () => {
  const dark = themeFor("dark")
  for (const overlay of [false, true]) {
    const snapshot = fold(null, { Snapshot: {
      foreground_id: "chat-review",
      sessions: [{ id: "chat-review", name: "Terminal review", cwd: "/tmp", messages: [] }],
      global: { mode: "Chat", palette: "dark", input: "/settings", command_palette: [{ name: "/settings", desc: "Manage settings" }] },
    } })!
    function Harness() {
      const renderer = useRenderer()
      const style = useMemo(() => SyntaxStyle.fromStyles({ default: { fg: dark.fg } }, renderer.nativeScene), [renderer])
      return <Chat snap={snapshot} theme={dark} syntaxStyle={style} focused={false} send={() => {}} sendTracked={async () => ""} onFlushError={() => {}}
        overlay={overlay ? <ModeSurface mode={{ Model: { sub: "choose", options: ["First model", "Second model"], cursor: 1 } }} theme={dark} columns={80} onMove={() => {}} send={() => {}} /> : null} />
    }
    const view = await testRender(<box flexGrow={1} paddingLeft={2} paddingRight={2}><Harness /></box>, { width: 80, height: 32 })
    try {
      await act(async () => { await view.waitForVisualIdle() })
      await act(async () => { await view.waitForVisualIdle() })
      const frame = view.captureCharFrame()
      expect(frame).toContain("Terminal review")
      expect(frame).toContain("[/]")
      expect(frame).toContain(overlay ? "Second model" : "Manage settings")
      const lines = frame.split("\n")
      const top = lines.findIndex((line) => line.includes(overlay ? "choose" : "commands"))
      expect(lines[top + 9]).toContain("╰")
      if (overlay) expect(frame).toContain("● Second model")
    } finally { await act(async () => { await view.dispose() }) }
  }
})

test("slash suggestions hide /select while clicks, arrows and direct execution still work", async () => {
  const dark = themeFor("dark")
  for (const direct of [false, true]) {
    const requests: unknown[] = []
    const input = direct ? "/select" : "/"
    const commands = (direct ? ["/select"] : ["/settings", "/select", "/help"]).map((name) => ({ name, desc: `Run ${name}` }))
    const snapshot = fold(null, { Snapshot: {
      foreground_id: "slash-review",
      sessions: [{ id: "slash-review", name: "Slash review", cwd: "/tmp", messages: [] }],
      global: { mode: "Chat", palette: "dark", input, cursor: input.length, command_palette: commands },
    } })!
    function Harness() {
      const renderer = useRenderer()
      const style = useMemo(() => SyntaxStyle.fromStyles({ default: { fg: dark.fg } }, renderer.nativeScene), [renderer])
      return <Chat snap={snapshot} theme={dark} syntaxStyle={style} focused send={(r) => requests.push(r)} sendTracked={async () => "ack"} onFlushError={() => {}} />
    }
    const view = await testRender(<Harness />, { width: 80, height: 24 })
    try {
      await act(async () => { await view.waitForVisualIdle() })
      await act(async () => { await view.waitForVisualIdle() })
      if (direct) {
        expect(view.captureCharFrame()).not.toContain("commands")
        await act(async () => { view.mockInput.pressEnter() })
        expect(requests).toEqual([{ SendKey: { code: "Enter", mods: 0 } }])
        expect(controlOf("EnterSelect")).toEqual({ kind: "select" })
      } else {
        expect(view.captureCharFrame()).toContain("Run /help")
        expect(view.captureCharFrame()).not.toContain("/select")
        await act(async () => { view.mockInput.pressArrow("down") })
        expect(requests).toEqual(Array.from({ length: 2 }, () => ({ SendKey: { code: "Down", mods: 0 } })))
        requests.length = 0
        const lines = view.captureCharFrame().split("\n")
        const y = lines.findIndex((line) => line.includes("Run /help"))
        await act(async () => { await view.mockMouse.click(lines[y]!.indexOf("/help") + 2, y) })
        expect(requests).toEqual(Array.from({ length: 2 }, () => ({ SendKey: { code: "Down", mods: 0 } })))
      }
    } finally { await act(async () => { await view.dispose() }) }
  }
})

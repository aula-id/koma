import { expect, test } from "bun:test"
import { act } from "react"
import { testRender } from "@opentui/react/test-utils"
import { rgbToHex } from "@opentui/core"
import { writeFileSync } from "node:fs"
import { ModeSurface } from "./surface.tsx"
import { PALETTE_NAMES, themeFor } from "../theme.ts"
import { FormGuard } from "./drafts.ts"
import { EventDispatcher, PendingOperations } from "../events.ts"
import { Fullscreen, Lines, Panel } from "./bits.tsx"

const escape = (s: string) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll('"', "&quot;")
const previews: string[] = []
const frames: unknown[] = []
for (const [width, height] of [[50, 24], [80, 18], [100, 40], [160, 45]]) {
  for (const name of PALETTE_NAMES) test(`semantic settings colors / ${name} / ${width}×${height}`, async () => {
    const theme = themeFor(name)
    const view = await testRender(<box flexGrow={1} paddingLeft={2} paddingRight={2}><ModeSurface mode={{ Settings: { page: "quick", quick_focus: "general", palette: name, palette_sel: PALETTE_NAMES.indexOf(name), field: 2, awareness_enabled: true, classifier_enabled: false } }} theme={theme} columns={width!} onMove={() => {}} send={() => {}} /></box>, { width, height })
    try {
      await act(async () => { await view.waitForVisualIdle() })
      await act(async () => { await view.waitForVisualIdle() })
      await act(async () => { await view.waitForVisualIdle() })
      const text = view.captureCharFrame()
      expect(text).toContain("Appearance")
      expect(text).toContain("General")
      expect(text).toContain("[━●]")
      expect(text).toContain("[●─]")
      expect(text).not.toContain("Save")
      expect(text).not.toContain("Unsaved")
      expect(text).not.toContain("preview")
      expect(text).not.toContain("■")
      expect(text).not.toContain("Edit / toggle\n")
      expect(text.match(/Edit \/ toggle/g)?.length).toBe(1)
      expect(text).toContain("Back (esc)")
      if (width! < 100) expect(text).toContain("/ 13")
      if (width === 50) expect(text).toContain("/ 10")
      const frame = view.captureSpans()
      const spans = frame.lines.flatMap((line) => line.spans)
      for (const label of ["General"]) {
        const line = frame.lines.find((line) => line.spans.map((s) => s.text).join("").includes(label))!
        const at = line.spans.map((s) => s.text).join("").indexOf(label)
        let offset = 0
        const hit = line.spans.find((span) => { const end=offset+span.text.length; const found=offset<=at && end>at;offset=end;return found })!
        expect(rgbToHex(hit.fg).toLowerCase()).toBe(theme.accent)
      }
      expect(spans.some((span) => span.text.includes("[━●]") && rgbToHex(span.fg).toLowerCase() === theme.success)).toBe(true)
      expect(spans.some((span) => span.text.includes("[●─]") && rgbToHex(span.fg).toLowerCase() === theme.dim)).toBe(true)
      const lines = text.split("\n")
      const switchAfter = (label: string, value: string) => {
        const line = lines.find((line) => line.includes(label))
        expect(line).toBeDefined()
        const labelAt = line!.indexOf(label)
        expect(labelAt).toBeGreaterThanOrEqual(0)
        const switchAt = line!.indexOf(value, labelAt + label.length)
        expect(switchAt).toBeGreaterThan(labelAt)
        return switchAt
      }
      expect(switchAfter("Awareness", "[━●]")).toBe(switchAfter("Harness", "[●─]"))
      const html = frame.lines.map((line) => line.spans.map((span) => `<span style="color:${rgbToHex(span.fg)};background:${rgbToHex(span.bg)}">${escape(span.text)}</span>`).join("")).join("\n")
      frames.push({name,width,height,lines:frame.lines.map((line)=>line.spans.map((span)=>({text:span.text,fg:rgbToHex(span.fg),bg:rgbToHex(span.bg),width:span.width})))})
      previews.push(`<article><h2>${escape(name)} · ${width}×${height}</h2><pre style="background:${theme.bg}">${html}</pre></article>`)
    } finally { await act(async () => { await view.dispose() }) }
  })
}

test("write actual rendered theme previews", () => {
  if (process.env.KOMA_PREVIEW_PATH) writeFileSync(process.env.KOMA_PREVIEW_PATH+".json",JSON.stringify(frames))
  if (process.env.KOMA_PREVIEW_PATH) writeFileSync(process.env.KOMA_PREVIEW_PATH, `<!doctype html><meta charset="utf-8"><title>Terminal color previews</title><style>body{background:#202124;color:white;font-family:system-ui}pre{font-family:monospace;font-size:13px;line-height:1.2;display:inline-block;padding:12px}article{overflow:auto}</style>${previews.join("\n")}`)
})

test("correlation ignores unrelated acknowledgements, failures and stale results", async () => {
  const ops = new PendingOperations()
  const dispatch = new EventDispatcher()
  dispatch.on("MenuResult", (reply) => ops.complete(reply))
  let done = false
  const pending = ops.wait("save").then((error) => { done = true; return error })
  dispatch.dispatch("Ack")
  dispatch.dispatch({ Error: "another request failed" })
  dispatch.dispatch({ MenuResult: { request_id: "old-save", error: null } })
  await Promise.resolve()
  expect(done).toBe(false)
  dispatch.dispatch({ MenuResult: { request_id: "save", error: "Validation failed" } })
  expect(await pending).toBe("Validation failed")
  expect(ops.size).toBe(0)
  const disconnected = ops.wait("draft")
  ops.close()
  expect(await disconnected).toContain("draft is retained")
})

test("live snapshots preserve form drafts and detect only content changes", () => {
  const guard = new FormGuard()
  const state = { mode: "edit", list_sel: 1, field: "name", draft_name: "Example", draft_body: "full prompt", editing: false }
  guard.observe({ Agents: state })
  expect(guard.dirty({ Agents: { ...state, field: "description" } })).toBe(false)
  expect(guard.dirty({ Agents: { ...state, draft_body: "changed prompt" } })).toBe(true)
  expect(guard.canLeave({ Agents: { ...state, editor: ["prompt", {}] } })).toBe(false)
  guard.observe({ Agents: { ...state, draft_body: "changed prompt" } })
  expect(guard.dirty({ Agents: { ...state, draft_body: "changed prompt" } })).toBe(true)
})

test("native secret input edits in the middle, selects, pastes, and remains masked", async () => {
  const requests: unknown[] = []
  const view = await testRender(<ModeSurface mode={{ Settings: { page: "provider_form", provider_editing: true, prov_modal: { name: "Example", endpoint: "example", api_key: "existing-secret", field: 2 } } }} theme={themeFor("dark")} columns={80} onMove={() => {}} send={(r) => requests.push(r)} />, { width: 80, height: 24 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    await act(async () => { await view.mockInput.typeText("abc"); view.mockInput.pressArrow("left"); await view.mockInput.typeText("X") })
    expect(requests).toContainEqual({ MenuReplace: { field: "API key", value: "abXc" } })
    await act(async () => { view.mockInput.pressKey("a", { ctrl: true }); await view.mockInput.pasteBracketedText("replacement") })
    expect(requests).toContainEqual({ MenuReplace: { field: "API key", value: "replacement" } })
    expect(view.captureCharFrame()).not.toContain("replacement")
    expect(view.captureCharFrame()).not.toContain("existing-secret")
  } finally { await act(async () => { await view.dispose() }) }
})

test("a click directly selects the intended model without arrow replay", async () => {
  const requests: unknown[] = []
  const view = await testRender(<ModeSurface mode={{ Settings: { page: "models", model_sel: 0, models: [{ uuid: "model-id", name: "Example model", model_id: "example", roles: [] }] } }} theme={themeFor("dark")} columns={80} onMove={() => {}} send={(r) => requests.push(r)} />, { width: 80, height: 24 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    await act(async () => { await view.waitForVisualIdle() })
    const lines = view.captureCharFrame().split("\n")
    const y = lines.findIndex((line) => line.includes("Example model"))
    await act(async () => { await view.mockMouse.click(8, y) })
    expect(requests).toContainEqual({ MenuSelect: { group: "active", index: 5, id: "model-id" } })
    expect(requests.some((r) => JSON.stringify(r).includes('"Up"') || JSON.stringify(r).includes('"Down"'))).toBe(false)
  } finally { await act(async () => { await view.dispose() }) }
})

test("new and resume remote only pick a saved host", async () => {
  for (const sub of ["new:browse", "resume:browse"]) {
    const view = await testRender(<ModeSurface mode={{ Remote: { sub, hosts: [{ name: "box", user: "u", host: "h", port: 22 }], filtered: [0], selected: 0 } }} theme={themeFor("dark")} columns={80} onMove={() => {}} send={() => {}} />, { width: 80, height: 24 })
    try {
      await act(async () => { await view.waitForVisualIdle() })
      const frame = view.captureCharFrame()
      expect(frame).toContain("box")
      expect(frame).not.toContain("+ New")
      expect(frame).not.toContain("import")
      expect(frame).not.toContain("delete")
    } finally { await act(async () => { await view.dispose() }) }
  }
})

test("inactive panel titles accent only their existing shortcut letter", async () => {
  const theme = themeFor("dark")
  const view = await testRender(<Fullscreen title="Settings" hint="Esc back" theme={theme}><Panel title="General" mark={0} hot={false} theme={theme}><text>Settings fields</text></Panel></Fullscreen>, { width: 50, height: 24 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    const line = view.captureSpans().lines.find((line) => line.spans.map((s) => s.text).join("").includes("General"))!
    expect(line.spans.some((span) => span.text === "G" && rgbToHex(span.fg).toLowerCase() === theme.accent)).toBe(true)
    expect(line.spans.some((span) => span.text.includes("eneral") && rgbToHex(span.fg).toLowerCase() === theme.dim)).toBe(true)
  } finally { await act(async () => { await view.dispose() }) }
})

for (const [width, height] of [[50, 24], [80, 18], [100, 40], [160, 45]]) {
  test(`management actions sit in the top-right border at ${width}×${height}`, async () => {
    const fixtures = [
      { mode: { Settings: { page: "providers", providers: [{ name: "Example", endpoint: "https://example.test/v1", api_type: "openai" }], prov_sel: 0 } }, title: "Providers", label: "+ Add (a)", code: "a" },
      { mode: { Settings: { page: "oauth", oauth_flow: { kind: "idle" }, oauth_drafts: [], oauth_sel: 0 } }, title: "OAuth", label: "+ Connect (a)", code: "a" },
      { mode: { Settings: { page: "models", model_filter: "all", models: [], model_sel: 0 } }, title: "Models", label: "+ Session", index: 1 },
      { mode: { Agents: { mode: "browse", agents: [] } }, title: "Agents", label: "+ New (n)", code: "n" },
      { mode: { Mcp: { mode: "browse", servers: [] } }, title: "MCP servers", label: "+ New (n)", code: "n" },
      { mode: { Remote: { sub: "manage:browse", hosts: [], filtered: [] } }, title: "Hosts", label: "+ New (n)", code: "n" },
    ]
    for (const fixture of fixtures) {
      const requests: unknown[] = []
      const view = await testRender(<ModeSurface mode={fixture.mode} theme={themeFor("dark")} columns={width!} onMove={() => {}} send={(r) => requests.push(r)} />, { width, height })
      try {
        for (let pass = 0; pass < 4; pass++) await act(async () => { await view.waitForVisualIdle() })
        const frame = view.captureCharFrame()
        const lines = frame.split("\n")
        const y = lines.findIndex((line) => line.includes(fixture.title) && line.includes("╭"))
        expect(lines[y]).toContain(fixture.label)
        expect(lines[y]!.indexOf(fixture.label)).toBeGreaterThan(width! / 2)
        expect(frame.split(fixture.label).length - 1).toBe(1)
        expect(frame).not.toContain("+ Add provider")
        expect(frame).not.toContain("+ Add global model")
        expect(frame).not.toContain("+ Connect a provider")
        if (fixture.title === "Providers") {
          expect(frame.split("https://example.test/v1").length - 1).toBe(1)
          expect(frame).not.toContain("openai")
        }
        await act(async () => { await view.mockMouse.click(lines[y]!.indexOf(fixture.label) + 2, y) })
        if (fixture.code) expect(requests).toContainEqual({ SendKey: { code: { Char: fixture.code }, mods: 0 } })
        else {
          expect(requests[0]).toMatchObject({ MenuSelect: { group: "active", index: fixture.index } })
          expect(requests[1]).toEqual({ SendKey: { code: "Enter", mods: 0 } })
        }
      } finally { await act(async () => { await view.dispose() }) }
    }
  })

  test(`toggle layouts stay aligned and visible at ${width}×${height}`, async () => {
    const pressed: number[] = []
    const view = await testRender(<Fullscreen title="Toggles" hint="Esc back" theme={themeFor("dark")}><Panel title="Controls" hot theme={themeFor("dark")}><Lines rows={[
      { label: "Main role", toggle: true, toggleLayout: "before" },
      { label: "Tool permission", toggle: false, toggleLayout: "before" },
      { label: "Awareness", toggle: true },
      { label: "A long setting name that must not push its switch off the screen".repeat(3), detail: "status ".repeat(40), toggle: false },
    ]} selected={0} theme={themeFor("dark")} onPress={(index) => pressed.push(index)} /></Panel></Fullscreen>, { width, height })
    try {
      for (let pass = 0; pass < 3; pass++) await act(async () => { await view.waitForVisualIdle() })
      const lines = view.captureCharFrame().split("\n")
      expect(lines.find((line) => line.includes("Main role"))).toContain("[━●] Main role")
      expect(lines.find((line) => line.includes("Tool permission"))).toContain("[●─] Tool permission")
      const enabled = lines.find((line) => line.includes("Awareness"))!
      const disabled = lines.find((line) => line.includes("A long setting"))!
      expect(enabled.indexOf("[━●]")).toBe(disabled.indexOf("[●─]"))
      expect(enabled.indexOf("[━●]")).toBeGreaterThan(width! - 10)
      const y = lines.findIndex((line) => line.includes("Main role"))
      await act(async () => { await view.mockMouse.click(lines[y]!.indexOf("[━●]") + 1, y) })
      expect(pressed).toEqual([0])
    } finally { await act(async () => { await view.dispose() }) }
  })
}

test("all toggle families share the switch glyph and keep their selection handlers", async () => {
  const fixtures = [
    { mode: { Settings: { page: "model_form", model_modal: { role_picker: { checked: [true, false, false, false, false], cursor: 0 } } } }, label: "main", layout: "before" },
    { mode: { Agents: { mode: "edit", tool_picker: { options: ["Hidden tool", "Visible tool"], checked: [true, false], filter: "Visible", cursor: 0 } } }, label: "Visible tool", layout: "before" },
    { mode: { Skill: { all: [{ name: "Example skill", description: "Skill description", is_active: true }], selected: 0 } }, label: "Example skill", layout: "before" },
    { mode: { Extensions: { mode: "use", rows: [{ name: "Example extension", active: true }], list_sel: 0 } }, label: "Example extension", layout: "before" },
    { mode: { Mcp: { mode: "edit", field: "enabled", draft_enabled: true, draft_transport: "stdio" } }, label: "enabled", layout: "right" },
    { mode: { Mcp: { mode: "browse", servers: [{ name: "Example server", enabled: true }], list_sel: 0 } }, label: "Example server", layout: "right" },
  ]
  for (const fixture of fixtures) {
    const requests: unknown[] = []
    const moved: number[] = []
    const height = 24
    const view = await testRender(<ModeSurface mode={fixture.mode} theme={themeFor("dark")} columns={80} onMove={(index) => moved.push(index)} send={(request) => requests.push(request)} />, { width: 80, height })
    try {
      for (let pass = 0; pass < 4; pass++) await act(async () => { await view.waitForVisualIdle() })
      const frame = view.captureCharFrame()
      expect(frame).not.toMatch(/[☑☐]/)
      const lines = frame.split("\n")
      const y = lines.findIndex((line) => line.includes(fixture.label))
      const line = lines[y]!
      const x = Math.max(line.indexOf("[━●]"), line.indexOf("[●─]"))
      expect(x).toBeGreaterThanOrEqual(0)
      if (fixture.layout === "before") expect(x).toBeLessThan(line.indexOf(fixture.label))
      else expect(x).toBeGreaterThan(line.indexOf(fixture.label))
      if (fixture.label === "Visible tool") expect(line).toContain("[●─]")
      await act(async () => { await view.mockMouse.click(x + 1, y) })
      expect(requests.length + moved.length).toBeGreaterThan(0)
    } finally { await act(async () => { await view.dispose() }) }
  }
})

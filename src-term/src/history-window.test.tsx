import { expect, test } from "bun:test"
import { act, useMemo, useState } from "react"
import { SyntaxStyle, ScrollBoxRenderable } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { HistoryPane } from "./chat.tsx"
import { fold } from "./protocol.ts"
import { themeFor } from "./theme.ts"

/**
 * Read the test screen. On a slow runner the renderer can still be writing a
 * frame (OutputBusy); wait for it to settle and read again.
 */
async function frame(view: { captureCharFrame: () => string; waitForVisualIdle: () => Promise<unknown> }): Promise<string> {
  for (let attempt = 0; ; attempt++) {
    try {
      return view.captureCharFrame()
    } catch (err) {
      if (attempt >= 50 || !String(err).includes("OutputBusy")) throw err
      await act(async () => { await Bun.sleep(10); await view.waitForVisualIdle() })
    }
  }
}

test("soak: progress appends, paging, and rewind keep mounts, anchors, and RSS bounded", async () => {
  let snap = fold(null, { Snapshot: { sessions: [{ id: "s", messages: [{ role: "system", content: "sys" }] }], global: {} } })!
  let turn = 0
  const append = (turns: number) => {
    for (let i = 0; i < turns; i++, turn++) {
      const from_index = snap.sessions[0]!.messages.length
      const messages = [
        { role: "user", content: `Prompt ${turn}` },
        { role: "assistant", content: `Reply ${turn}\n` + "Some content to wrap. ".repeat(10) },
      ]
      snap = fold(snap, { Delta: { SessionProgress: { session_id: "s", from_index, messages, streaming: null, stream_reasoning: "", tokens_in: turn, tokens_out: turn, tokens_cached: 0, cost: 0 } } })!
    }
  }
  let updateSession: (next: (typeof snap.sessions)[0]) => void = () => {}
  function Screen() {
    const [shown, setShown] = useState(snap.sessions[0]!)
    updateSession = setShown
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    return <HistoryPane session={shown} theme={themeFor("dark")} syntaxStyle={style} />
  }
  const view = await testRender(<Screen />, { width: 80, height: 24 })
  const idle = async () => { await act(async () => { await view.waitForVisualIdle() }) }
  const find = (node: any): ScrollBoxRenderable | undefined => node instanceof ScrollBoxRenderable ? node : node.getChildren().map(find).find(Boolean)
  const turns = (box: ScrollBoxRenderable) => box.getChildren()[0].getChildren().filter((n) => String(n.id).startsWith("turn-"))
  const shows = async (text: string) => {
    for (let i = 0; i < 100 && !(await frame(view)).includes(text); i++) {
      await act(async () => { await Bun.sleep(10); await view.waitForVisualIdle() })
    }
    return await frame(view)
  }
  let box: ScrollBoxRenderable
  const toTail = async () => {
    for (let i = 0; i < 12 && !(await frame(view)).includes(`Prompt ${turn - 1}`); i++) {
      await act(async () => {
        box.scrollTo(box.scrollHeight)
        await view.mockMouse.scroll(box.x + 2, Math.max(box.y + 2, box.y + box.height - 3), "down")
      })
      await idle()
    }
  }
  try {
    await idle()
    box = find(view.renderer.root)!
    const rss: number[] = []
    for (let cycle = 0; cycle < 10; cycle++) {
      for (let batch = 0; batch < 10; batch++) {
        append(20)
        await act(async () => { updateSession(snap.sessions[0]!) })
      }
      await idle()
      expect(await shows(`Prompt ${turn - 1}`)).toContain(`Prompt ${turn - 1}`)
      expect(turns(box).length).toBeLessThanOrEqual(30)
      for (let page = 0; page < 4; page++) {
        await act(async () => { box.scrollTo(0) })
        await idle()
        const anchor = turns(box).find((n) => n.y + n.height > box.viewport.y)!
        const before = { id: anchor.id, y: anchor.y - box.viewport.y }
        await act(async () => { await view.mockMouse.scroll(box.x + 2, box.y + 2, "up") })
        await idle()
        const after = turns(box).find((n) => n.id === before.id)
        expect(after && after.y - box.viewport.y).toBe(before.y)
        expect(turns(box).length).toBeLessThanOrEqual(90)
      }
      await toTail()
      expect(await shows(`Prompt ${turn - 1}`)).toContain(`Prompt ${turn - 1}`)
      Bun.gc(true)
      rss.push(process.memoryUsage().rss)
    }
    // ubuntu-latest measured 4.14 MiB per cycle after GC. 8 MiB still flags a real leak.
    expect((rss[9]! - rss[5]!) / 4).toBeLessThan(8 * 1024 * 1024)
    expect(Math.max(...rss) - rss[0]!).toBeLessThan(200 * 1024 * 1024)

    const kept = snap.sessions[0]!.messages.slice(0, 1 + 2 * 150)
    snap = fold(snap, { Snapshot: { sessions: [{ id: "s", messages: structuredClone(kept) }], global: {} } })!
    expect(snap.sessions[0]!.history_rev).toBe(1)
    await act(async () => { updateSession(snap.sessions[0]!) })
    await idle()
    const ids = turns(box).map((n) => Number(String(n.id).slice("turn-".length)))
    expect(Math.max(...ids)).toBeLessThan(kept.length)
    expect(await shows("Prompt 149")).toContain("Prompt 149")
    expect(await frame(view)).not.toContain("Reading history")
  } finally { await act(async () => { await view.dispose() }) }
}, 120_000)

test("thousands of turns page both ways with bounded native mounts and footprint", async () => {
  const messages = Array.from({ length: 3000 }, (_, i) => [
    { role: "user", content: `Prompt ${i}` },
    { role: "assistant", content: `Reply ${i}\n` + "Some content to wrap. ".repeat(10) },
  ]).flat()
  const session = fold(null, { Snapshot: { sessions: [{ id: "s", messages }], global: {} } })!.sessions[0]
  let updateSession: (next: typeof session) => void = () => {}
  function Screen() {
    const [shown, setShown] = useState(session)
    updateSession = setShown
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    return <HistoryPane session={shown} theme={themeFor("dark")} syntaxStyle={style} />
  }
  const view = await testRender(<Screen />, { width: 80, height: 24 })
  const idle = async () => { await act(async () => { await view.waitForVisualIdle() }) }
  const find = (node: any): ScrollBoxRenderable | undefined => node instanceof ScrollBoxRenderable ? node : node.getChildren().map(find).find(Boolean)
  try {
    await idle()
    const box = find(view.renderer.root)!
    const count = () => box.getChildren()[0].getChildren().filter(n => String(n.id).startsWith("turn-")).length
    expect(count()).toBe(30)
    Bun.gc(true)
    const baseline = process.memoryUsage().rss
    for (let i = 0; i < 8; i++) {
      await act(async () => { box.scrollTo(0); await view.mockMouse.scroll(box.x + 2, box.y + 2, "up") })
      await idle()
      expect(count()).toBeLessThanOrEqual(90)
    }
    expect(await frame(view)).not.toContain("Prompt 2999")
    const first = box.getChildren()[0].getChildren()[0]
    await act(async () => { updateSession({ ...session, streaming: "LIVE UPDATE" }) })
    await idle()
    expect(box.getChildren()[0].getChildren()[0]).toBe(first)
    expect(await frame(view)).not.toContain("LIVE UPDATE")
    if (process.platform !== "win32") {
      await act(async () => { process.emit("SIGUSR2") })
      await idle()
      expect(count()).toBeLessThanOrEqual(30)
    }
    for (let i = 0; i < 24; i++) {
      await act(async () => { box.scrollTo(box.scrollHeight); await view.mockMouse.scroll(box.x + 2, box.y + 2, "down") })
      await idle()
      expect(count()).toBeLessThanOrEqual(90)
    }
    expect(await frame(view)).toContain("LIVE UPDATE")
    Bun.gc(true)
    expect(process.memoryUsage().rss - baseline).toBeLessThan(160 * 1024 * 1024)
  } finally { await act(async () => { await view.dispose() }) }
}, 30_000)

test("a reduced window stays on the tail when the reader reaches the bottom", async () => {
  const messages = Array.from({ length: 200 }, (_, i) => [
    { role: "user", content: `Prompt ${i}` },
    { role: "assistant", content: `Reply ${i}\n` + "word ".repeat(30) },
  ]).flat()
  const session = fold(null, { Snapshot: { sessions: [{ id: "s", messages }], global: {} } })!.sessions[0]
  let updateSession: (next: typeof session) => void = () => {}
  function Screen() {
    const [shown, setShown] = useState(session)
    updateSession = setShown
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    return <HistoryPane session={shown} theme={themeFor("dark")} syntaxStyle={style} />
  }
  const view = await testRender(<Screen />, { width: 80, height: 24 })
  const idle = async () => { await act(async () => { await view.waitForVisualIdle() }) }
  const find = (node: any): ScrollBoxRenderable | undefined => node instanceof ScrollBoxRenderable ? node : node.getChildren().map(find).find(Boolean)
  try {
    await idle()
    const box = find(view.renderer.root)!
    if (process.platform !== "win32") {
      await act(async () => { process.emit("SIGUSR2") })
      await idle()
    }
    await act(async () => { updateSession({ ...session }) })
    await idle()
    await act(async () => { box.scrollTo(0); await view.mockMouse.scroll(box.x + 2, box.y + 2, "up") })
    await idle()
    expect(await frame(view)).not.toContain("Prompt 199")
    for (let i = 0; i < 16; i++) {
      await act(async () => { box.scrollTo(box.scrollHeight); await view.mockMouse.scroll(box.x + 2, box.y + box.height - 3, "down") })
      await idle()
    }
    const tail = await frame(view)
    expect(tail).toContain("Prompt 199")
    expect(tail).not.toContain("Prompt 0\n")
  } finally { await act(async () => { await view.dispose() }) }
}, 30_000)

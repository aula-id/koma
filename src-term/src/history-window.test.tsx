import { expect, test } from "bun:test"
import { act, useMemo, useState } from "react"
import { SyntaxStyle, ScrollBoxRenderable } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { HistoryPane } from "./chat.tsx"
import { fold } from "./protocol.ts"
import { themeFor } from "./theme.ts"

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
    expect(view.captureCharFrame()).not.toContain("Prompt 2999")
    const first = box.getChildren()[0].getChildren()[0]
    await act(async () => { updateSession({ ...session, streaming: "LIVE UPDATE" }) })
    await idle()
    expect(box.getChildren()[0].getChildren()[0]).toBe(first)
    expect(view.captureCharFrame()).not.toContain("LIVE UPDATE")
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
    expect(view.captureCharFrame()).toContain("LIVE UPDATE")
    Bun.gc(true)
    expect(process.memoryUsage().rss - baseline).toBeLessThan(160 * 1024 * 1024)
  } finally { await act(async () => { await view.dispose() }) }
}, 30_000)

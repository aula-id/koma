import { expect, test } from "bun:test"
import { act, useMemo } from "react"
import { SyntaxStyle } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { COMPOSER_TIPS, Chat, tipReveal } from "./chat.tsx"
import { fold } from "./protocol.ts"
import { themeFor } from "./theme.ts"

test("composer tips start with help and reveal one step at a time", () => {
  expect(COMPOSER_TIPS[0]).toContain("/help")
  expect(COMPOSER_TIPS.some((tip) => tip.includes("Shift+Tab"))).toBe(true)
  expect(COMPOSER_TIPS.length).toBe(28)
  expect(COMPOSER_TIPS.some((tip) => tip.includes("$"))).toBe(false)
  expect(tipReveal("/help lists commands", 5, 40)).toBe("/help")
  expect(tipReveal("/help lists commands", 100, 6)).toBe("/help ")
  expect(tipReveal(COMPOSER_TIPS[0] ?? "", 2, 80)).toBe("/h")
  expect((COMPOSER_TIPS[0] ?? "").length).toBeGreaterThan(2)
})

test("a composer tip keeps typing past the first character", async () => {
  const theme = themeFor("dark")
  const snapshot = fold(null, { Snapshot: {
    foreground_id: "s",
    sessions: [{ id: "s", name: "S", messages: [] }],
    global: { mode: "Chat", palette: "dark", input: "" },
  } })!
  function Harness() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    return <Chat snap={snapshot} theme={theme} syntaxStyle={style} focused send={() => {}} sendTracked={async () => "ack"} onFlushError={() => {}} />
  }
  const view = await testRender(<Harness />, { width: 80, height: 24 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    for (let step = 0; step < 8; step++) {
      await act(async () => { await new Promise((resolve) => setTimeout(resolve, 25)) })
    }
    expect(view.captureCharFrame()).toContain("/help")
  } finally {
    await act(async () => { await view.dispose() })
  }
})

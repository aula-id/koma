import { expect, test } from "bun:test"
import { act, useMemo } from "react"
import { SyntaxStyle } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { Chat, drssHead, modelLabel, promptMark } from "./chat.tsx"

import { fold } from "./protocol.ts"
import { themeFor } from "./theme.ts"

test("the DRSS comet rests after it leaves the label", () => {
  expect(drssHead(0)).toBe(0)
  expect(drssHead(80 * 5)).toBe(5)
  expect(drssHead(80 * 6)).toBe(6)
  expect(drssHead(80 * 10)).toBe(0)
})

test("a selected effort sits beside the model name", () => {
  expect(modelLabel("grok-4.7", "medium")).toBe("grok-4.7 · medium")
  expect(modelLabel("grok-4.7", "")).toBe("grok-4.7")
  expect(modelLabel("grok-4.7", "  ")).toBe("grok-4.7")
})

test("the prompt mark follows the first character and keeps it", () => {
  expect(promptMark("")).toBe("[$]")
  expect(promptMark("hello")).toBe("[$]")
  expect(promptMark("!ls")).toBe("[>]")
  expect(promptMark("/help")).toBe("[/]")
  expect(promptMark("@file")).toBe("[@]")
  expect(promptMark(" !ls")).toBe("[$]")
})

test("the prompt mark sits against the composer border", async () => {
  const theme = themeFor("dark")
  const snapshot = fold(null, { Snapshot: {
    foreground_id: "pad",
    sessions: [{ id: "pad", name: "Pad", cwd: "/tmp", messages: [] }],
    global: { mode: "Chat", palette: "dark", status: "ready", input: "asd\nalk" },
  } })!
  function Harness() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    return <Chat snap={snapshot} theme={theme} syntaxStyle={style} focused send={() => {}} sendTracked={async () => "ack"} onFlushError={() => {}} />
  }
  const view = await testRender(<Harness />, { width: 40, height: 16 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    const frame = view.captureCharFrame()
    const lines = frame.split("\n")
    const line = lines.find((row) => row.includes("[$]"))
    expect(line).toBeDefined()
    expect(line).toContain("│[$]")
    const koma = lines.findIndex((row) => row.includes("Koma"))
    const tips = lines.findIndex((row) => row.includes("Tips:"))
    const top = lines.findIndex((row) => row.includes("╭"))
    const prompt = lines.findIndex((row) => row.includes("[$]"))
    const bottom = lines.findIndex((row, index) => index > prompt && row.includes("╰"))
    const ram = lines.findIndex((row) => row.includes("RAM["))
    expect(tips).toBe(koma + 1)
    expect(top).toBe(tips + 1)
    expect(bottom).toBeGreaterThan(prompt)
    // The idle sweep may be showing its own label on a slow runner.
    expect(lines[bottom]).toMatch(/ready|cleanup/)
    expect(ram).toBe(bottom + 1)
    expect(lines[ram]).toContain("[!]")
    expect(frame).not.toContain("Latest")
  } finally {
    await act(async () => { await view.dispose() })
  }
})

test("the composer box grows with soft-wrapped rows", async () => {
  const theme = themeFor("dark")
  const rows = async (input: string) => {
    const snapshot = fold(null, { Snapshot: {
      foreground_id: "wrap",
      sessions: [{ id: "wrap", name: "Wrap", cwd: "/tmp", messages: [] }],
      global: { mode: "Chat", palette: "dark", input },
    } })!
    function Harness() {
      const renderer = useRenderer()
      const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
      return <Chat snap={snapshot} theme={theme} syntaxStyle={style} focused send={() => {}} sendTracked={async () => "ack"} onFlushError={() => {}} />
    }
    const view = await testRender(<Harness />, { width: 30, height: 30 })
    try {
      await act(async () => { await view.waitForVisualIdle() })
      await act(async () => { await view.waitForVisualIdle() })
      return view.captureCharFrame().split("\n").filter((row) => row.includes("│") && !row.includes("╭") && !row.includes("╰") && row.includes("a")).length
    } finally {
      await act(async () => { await view.dispose() })
    }
  }
  expect(await rows("a".repeat(70))).toBeGreaterThanOrEqual(3)
})

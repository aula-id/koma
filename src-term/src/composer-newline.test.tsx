import { expect, test } from "bun:test"
import { act, useMemo } from "react"
import { SyntaxStyle, type TextareaRenderable } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { Chat } from "./chat.tsx"
import { fold } from "./protocol.ts"
import { themeFor } from "./theme.ts"

function Harness({ requests, commands = [] }: { requests: unknown[]; commands?: { name: string; desc: string }[] }) {
  const renderer = useRenderer()
  const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
  const snapshot = fold(null, { Snapshot: {
    foreground_id: "s",
    sessions: [{ id: "s", name: "S", messages: [] }],
    global: { mode: "Chat", palette: "dark", input: "", cursor: 0, command_palette: commands },
  } })!
  const theme = themeFor("dark")
  return <Chat snap={snapshot} theme={theme} syntaxStyle={style} focused send={(request) => requests.push(request)} sendTracked={async () => "ack"} onFlushError={() => {}} />
}

async function composer(commands: { name: string; desc: string }[] = []) {
  const requests: unknown[] = []
  const view = await testRender(<Harness requests={requests} commands={commands} />, { width: 80, height: 24, kittyKeyboard: true })
  await act(async () => { await view.waitForVisualIdle() })
  const area = () => view.renderer.currentFocusedRenderable as TextareaRenderable
  const submitted = () => requests.some((request) => (request as { SendKey?: { code?: string } }).SendKey?.code === "Enter")
  return { view, submitted, area }
}

async function typeThen(view: Awaited<ReturnType<typeof testRender>>, press: () => void) {
  await act(async () => { await view.mockInput.typeText("ab") })
  await act(async () => {
    press()
    await new Promise((resolve) => setTimeout(resolve, 40))
  })
  await act(async () => { await view.mockInput.typeText("c") })
}

test("shift+enter and ctrl+j insert a newline and do not submit", async () => {
  const { view, submitted, area } = await composer()
  try {
    await typeThen(view, () => view.mockInput.pressEnter({ shift: true }))
    expect(area().plainText).toBe("ab\nc")
    expect(submitted()).toBe(false)
    await act(async () => { view.mockInput.pressKey("j", { ctrl: true }); await new Promise((resolve) => setTimeout(resolve, 40)) })
    await act(async () => { await view.mockInput.typeText("d") })
    expect(area().plainText).toBe("ab\nc\nd")
    expect(submitted()).toBe(false)
  } finally {
    await act(async () => { await view.dispose() })
  }
})

test("plain enter still submits", async () => {
  const { view, submitted, area } = await composer()
  try {
    await act(async () => { await view.mockInput.typeText("hello") })
    await act(async () => { view.mockInput.pressEnter(); await new Promise((resolve) => setTimeout(resolve, 40)) })
    expect(area().plainText).toBe("hello")
    expect(submitted()).toBe(true)
  } finally {
    await act(async () => { await view.dispose() })
  }
})

for (const sequence of ["\u001b[13;2u", "\u001b[27;2;13~", "\n", "\u001b\r", "\u001b[10;5u"]) {
  test(`${JSON.stringify(sequence)} inserts a newline and does not submit`, async () => {
    const { view, submitted, area } = await composer()
    try {
      await typeThen(view, () => view.mockInput.pressKeys([sequence]))
      expect(area().plainText).toBe("ab\nc")
      expect(submitted()).toBe(false)
    } finally {
      await act(async () => { await view.dispose() })
    }
  })
}

test("shift+enter does not run a slash command while the palette is open", async () => {
  const { view, submitted, area } = await composer([{ name: "/help", desc: "help" }])
  try {
    await typeThen(view, () => view.mockInput.pressEnter({ shift: true }))
    expect(area().plainText).toBe("ab\nc")
    expect(submitted()).toBe(false)
    await typeThen(view, () => view.mockInput.pressKeys(["\u001b\r"]))
    expect(area().plainText).toBe("ab\ncab\nc")
    expect(submitted()).toBe(false)
  } finally {
    await act(async () => { await view.dispose() })
  }
})

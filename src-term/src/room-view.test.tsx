import { expect, test } from "bun:test"
import { testRender } from "@opentui/react/test-utils"
import { act } from "react"
import { RoomView, authorInk, type Line } from "./room.tsx"
import { themeFor } from "./theme.ts"

const theme = themeFor("dark")
const lines: Line[] = [
  { id: "1", author: "you", text: "scan the staging api" },
  { id: "2", author: "chief", text: "Scope check first. I will ask red to probe auth." },
  { id: "3", author: "red", text: "Found a token in /debug." },
]
const panes = { lanes: "red running 4 steps\nreviewer done", swim: "red  ▶ probe ▶ probe", shelf: "claims/auth-1\nscope/api", map: "goal api\n ○ claim auth" }
const counts = { open: 2, verified: 1, returned: 0, reports: 0 }

async function frame(width: number, height: number, over: Partial<Parameters<typeof RoomView>[0]> = {}) {
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => {
    view = await testRender(<RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft="" onDraft={() => {}} onSend={() => {}} {...over} />, { width, height })
  })
  await act(async () => { await view.renderOnce() })
  const text = view.captureCharFrame()
  await act(async () => { await view.dispose() })
  return text
}

test("wide room shows the chat beside the lanes pane", async () => {
  const text = await frame(120, 30)
  expect(text).toContain("secure room")
  expect(text).toContain("2 open · 1 verified")
  expect(text).toContain("scan the staging api")
  expect(text).toContain("Found a token")
  expect(text).toContain("red running")
  expect(text).toContain("[$]")
})

test("narrow room shows the chat alone with pane tabs", async () => {
  const text = await frame(60, 24)
  expect(text).toContain("Found a token")
  expect(text).toContain("Lanes")
  expect(text).not.toContain("red running")
})

test("an empty room says how to start", async () => {
  const text = await frame(100, 24, { lines: [], panes: { lanes: "", swim: "", shelf: "", map: "" }, phase: "starting the secure daemon" })
  expect(text).toContain("Message the Chief to start")
  expect(text).toContain("starting the secure daemon")
  expect(text).toContain("No lane is running")
})

test("specialists take distinct inks", () => {
  expect(authorInk("chief", theme)).toBe(theme.accent)
  expect(authorInk("red-api", theme)).toBe(theme.error)
  expect(authorInk("reviewer", theme)).toBe(theme.info)
})

test("typing @r offers the specialists above the composer", async () => {
  const text = await frame(100, 24, { draft: "@r" })
  expect(text).toContain("people")
  expect(text).toContain("@red")
  expect(text).toContain("@reviewer")
  expect(text).not.toContain("@chief")
})

test("/status opens every lane full screen and esc comes back", async () => {
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => {
    view = await testRender(<RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft="/status" onDraft={() => {}} onSend={() => {}} />, { width: 100, height: 24 })
  })
  await act(async () => { await view.mockInput.pressEnter() })
  await act(async () => { await view.renderOnce() })
  const text = view.captureCharFrame()
  await act(async () => { await view.dispose() })
  expect(text).toContain("secure room · status")
  expect(text).toContain("Who is working")
  expect(text).toContain("Activity")
  expect(text).not.toContain("[$]")
})

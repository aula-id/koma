import { expect, test } from "bun:test"
import { testRender } from "@opentui/react/test-utils"
import { act } from "react"
import { RoomView, authorInk } from "./room.tsx"
import { say, type Line } from "./room-turns.ts"
import { themeFor } from "./theme.ts"

const theme = themeFor("dark")
const lines: Line[] = [
  say("you", "scan the staging api"),
  say("chief", "Scope check first. I will ask red to probe auth.", "chat"),
  say("red", "Found a token in /debug.", "chat"),
]
const panes = { lanes: "red running 4 steps\nreviewer done", swim: "red  ▶ probe ▶ probe", shelf: "claims/auth-1\nscope/api", map: "goal api\n ○ claim auth" }
const counts = { open: 2, verified: 1, returned: 0, reports: 0 }

async function frame(width: number, height: number, over: Partial<Parameters<typeof RoomView>[0]> = {}) {
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => {
    view = await testRender(<RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft="" onDraft={() => {}} onSend={() => {}} {...over} />, { width, height })
  })
  await act(async () => { await new Promise((r) => setTimeout(r, 30)); await view.waitForVisualIdle(); await view.renderOnce() })
  const text = view.captureCharFrame()
  await act(async () => { await view.dispose() })
  return text
}

test("the chat is one full-width column at any size, never beside a pane", async () => {
  for (const [w, h] of [[120, 30], [60, 24]] as const) {
    const text = await frame(w, h)
    expect(text).toContain("secure room")
    expect(text).toContain("scan the staging api")
    expect(text).toContain("Found a token")
    expect(text).toContain("[$]")
    expect(text).not.toContain("red running")
    const rows = text.split("\n")
    const chief = rows.findIndex((r) => r.includes("chief"))
    const red = rows.findIndex((r) => r.trim().endsWith("red") || r.includes("┊ red"))
    expect(chief).toBeGreaterThan(-1)
    expect(red).toBeGreaterThan(chief)
  }
})

test("an empty room says how to start", async () => {
  const text = await frame(100, 24, { lines: [], panes: { lanes: "", swim: "", shelf: "", map: "" }, phase: "starting the secure daemon" })
  expect(text).toContain("Message the Chief to start")
  expect(text).toContain("starting the secure daemon")
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

const models = {
  choices: [
    { id: "m1", name: "Fast", model_id: "fast-1", provider: "acme" },
    { id: "m2", name: "Strong", model_id: "strong-1", provider: "other" },
  ],
  lanes: [
    { avatar: "red", display: "Lab Red", lane: "chat", model: "session default" },
    { avatar: "red", display: "Lab Red", lane: "work", model: "Strong" },
    { avatar: "clerk", display: "Clerk", lane: "work", model: "session default" },
  ],
}

test("/model walks minion, lane, then a searchable model and reports the choice", async () => {
  const picks: string[][] = []
  let asked = 0
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => {
    view = await testRender(<RoomView theme={theme} lines={[]} panes={panes} counts={counts} phase="connected" draft="/model" onDraft={() => {}} onSend={() => {}} models={models} onOpenModels={() => { asked++ }} onSetModel={(a, l, m) => { picks.push([a, l, m]) }} />, { width: 100, height: 28 })
  })
  const text = async () => { await act(async () => { await view.renderOnce() }); return view.captureCharFrame() }
  await act(async () => { await view.mockInput.pressEnter() })
  expect(asked).toBe(1)
  let t = await text()
  expect(t).toContain("Which minion")
  expect(t).toContain("Lab Red")
  expect(t).toContain("work: Strong")
  await act(async () => { await view.mockInput.pressEnter() })
  t = await text()
  expect(t).toContain("Lab Red · which lane")
  await act(async () => { view.mockInput.pressArrow("down") })
  await act(async () => { await view.mockInput.pressEnter() })
  t = await text()
  expect(t).toContain("Lab Red · work model")
  expect(t).toContain("Search")
  expect(t).toContain("Fast")
  await act(async () => { await view.mockInput.typeText("stro") })
  t = await text()
  expect(t).toContain("Strong")
  expect(t).not.toContain("Fast")
  await act(async () => { await view.mockInput.pressEnter() })
  expect(picks).toEqual([["red", "work", "m2"]])
  t = await text()
  expect(t).toContain("Which minion")
  await act(async () => { await view.dispose() })
})

test("ctrl+c asks to quit to chat; enter confirms and esc stays", async () => {
  const answers: boolean[] = []
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => {
    view = await testRender(<RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft="" onDraft={() => {}} onSend={() => {}} asking onAnswer={(yes) => { answers.push(yes) }} />, { width: 100, height: 24 })
  })
  await act(async () => { await view.renderOnce() })
  const text = view.captureCharFrame()
  expect(text).toContain("Quit to chat?")
  expect(text).toContain("Stay (esc)")
  await act(async () => { await view.mockInput.pressEnter() })
  await act(async () => { view.mockInput.pressEscape(); await Bun.sleep(80) })
  expect(answers).toEqual([true, false])
  await act(async () => { await view.dispose() })
})

test("a turn shows collapsed thinking, a tool card and markdown, and long lines wrap", async () => {
  const long = "The working directory is /Users/someone/Projects/retas/koma and nothing else is attached to this room right now, so I ran pwd myself."
  const turn: Line = {
    id: "t", author: "chief", lane: "chat",
    items: [
      { kind: "reasoning", text: "The user wants the cwd, run pwd." },
      { kind: "tool", id: "c1", name: "bash", args: '{"command":"pwd"}', result: { ok: true, text: "/Users/someone/Projects/retas/koma" } },
      { kind: "text", text: `**Short answer:** ${long}` },
    ],
  }
  const text = await frame(70, 30, { lines: [say("you", "pwd?"), turn] })
  expect(text).toContain("reasoning")
  expect(text).not.toContain("The user wants the cwd")
  expect(text).toContain("bash")
  expect(text).toContain("pwd")
  expect(text).toContain("Short answer:")
  expect(text).not.toContain("**")
  expect(text).toContain("so I ran pwd myself.")
})

import { useState } from "react"

function Live({ sent, start = "" }: { sent: string[]; start?: string }) {
  const [draft, setDraft] = useState(start)
  return <RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft={draft} onDraft={setDraft} onSend={(text) => { sent.push(text ?? draft); setDraft("") }} />
}

type View = Awaited<ReturnType<typeof testRender>>
/** One step of a script. Each runs in its own act so the screen's state settles between keys, as it does live. */
type Step = (view: View) => Promise<void> | void

async function live(width: number, height: number, steps: Step[], start = "") {
  const sent: string[] = []
  let view!: View
  await act(async () => { view = await testRender(<Live sent={sent} start={start} />, { width, height }) })
  for (const step of steps) await act(async () => { await step(view); await new Promise((r) => setTimeout(r, 40)) })
  await act(async () => { await view.waitForVisualIdle(); await view.renderOnce() })
  const text = view.captureCharFrame()
  await act(async () => { await view.dispose() })
  return { text, sent }
}

test("the composer grows with a multi-line draft instead of staying one tight row", async () => {
  const { text } = await live(80, 30, [], "alpha\nbeta\ngamma\ndelta")
  for (const word of ["alpha", "beta", "gamma", "delta"]) expect(text).toContain(word)
  expect(text).toContain("Tips:")
  expect(text).toContain("Koma")
})

test("typing /st and pressing enter runs /status at once, like the coding chat", async () => {
  const { text, sent } = await live(100, 24, [
    (view) => view.mockInput.typeText("/st"),
    (view) => view.mockInput.pressEnter(),
  ])
  expect(text).toContain("secure room · status")
  expect(sent).toEqual([])
})

test("tab completes a name with a space, and a message after it is sent to the lane", async () => {
  const { text, sent } = await live(100, 24, [
    (view) => view.mockInput.typeText("@re"),
    (view) => view.mockInput.pressTab(),
    (view) => view.mockInput.typeText("check auth"),
    (view) => view.mockInput.pressEnter(),
  ])
  expect(sent).toEqual(["@red check auth"])
  expect(text).not.toContain("commands")
})

test("enter on a bare name only writes it and waits for the message; esc closes the popup", async () => {
  const first = await live(100, 24, [
    (view) => view.mockInput.typeText("@rev"),
    (view) => view.mockInput.pressEnter(),
  ])
  expect(first.sent).toEqual([])
  expect(first.text).toContain("@reviewer")
  const second = await live(100, 24, [
    (view) => view.mockInput.typeText("/"),
    (view) => view.mockInput.pressEscape(),
  ])
  expect(second.text).not.toContain("commands")
})

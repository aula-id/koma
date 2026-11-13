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
  await act(async () => { await new Promise((r) => setTimeout(r, 80)); await view.waitForVisualIdle(); await view.renderOnce(); await new Promise((r) => setTimeout(r, 40)); await view.renderOnce() })
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
  expect(text).toContain("Lanes")
  expect(text).toContain("Goals and gates")
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

test("the status screen is a roster table, a gates panel and a readable timeline", async () => {
  const table = [
    { avatar: "chief", display: "Chief", alias: "CF", role: "chief", lane: "chat", phase: "running", task: "answering you", started_ms: 1000, last_event_ms: 98_000, steps: 3, last_tool: "bash", error: "" },
    { avatar: "red", display: "Red", alias: "LR", role: "red", lane: "work", phase: "running", task: "probe: check the login", started_ms: 1000, last_event_ms: 97_000, steps: 12, last_tool: "grep", error: "" },
    { avatar: "reviewer", display: "Reviewer", alias: "RV", role: "reviewer", lane: "", phase: "idle", task: "", started_ms: 0, last_event_ms: 0, steps: 0, last_tool: "", error: "" },
  ]
  const events = [
    { at_ms: 99_000, avatar: "red", marker: "○", label: "claims/auth-1" },
    { at_ms: 90_000, avatar: "red", marker: "◆", label: "probe" },
  ]
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => {
    view = await testRender(<RoomView theme={theme} lines={lines} panes={{ ...panes, table, events, nowMs: 100_000 }} counts={counts} phase="connected" draft="/status" onDraft={() => {}} onSend={() => {}} />, { width: 120, height: 36 })
  })
  await act(async () => { await view.mockInput.pressEnter() })
  await act(async () => { await view.renderOnce() })
  const text = view.captureCharFrame()
  await act(async () => { await view.dispose() })
  for (const word of ["Chief", "answering you", "probe: check the login", "grep", "Reviewer", "idle", "3s", "claims/auth-1", "10s", "2 running · 1 idle", "last tool"]) expect(text).toContain(word)
  expect(text).not.toContain("chief ◆ chief")
  const rows = text.split("\n")
  expect(rows.findIndex((r) => r.includes("claims/auth-1"))).toBeLessThan(rows.findIndex((r) => r.includes("probe") && r.includes("◆")))
})

test("a paste goes to the daemon, and what it sends back lands at the caret", async () => {
  const pasted: string[] = []
  let images = 0
  let latest = ""
  let insert!: (text: string) => void
  function Harness() {
    const [draft, setDraft] = useState("")
    const [inserted, setInserted] = useState<{ id: number; text: string } | null>(null)
    latest = draft
    insert = (text) => setInserted((cur) => ({ id: (cur?.id ?? 0) + 1, text }))
    return <RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft={draft} onDraft={setDraft} onSend={() => {}}
      inserted={inserted} onPaste={(t) => pasted.push(t)} onPasteImage={() => { images += 1 }} />
  }
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => { view = await testRender(<Harness />, { width: 100, height: 24 }) })
  await act(async () => { await view.mockInput.pasteBracketedText("line one\nline two"); await new Promise((r) => setTimeout(r, 20)) })
  expect(pasted).toEqual(["line one\nline two"])
  expect(latest).toBe("")
  await act(async () => { view.mockInput.pressKey("v", { ctrl: true }); await new Promise((r) => setTimeout(r, 20)) })
  expect(images).toBe(1)
  await act(async () => { await view.mockInput.typeText("see "); await new Promise((r) => setTimeout(r, 20)) })
  await act(async () => { insert("[Image #1]"); await new Promise((r) => setTimeout(r, 40)) })
  await act(async () => { await view.mockInput.typeText(" please"); await new Promise((r) => setTimeout(r, 20)) })
  expect(latest).toBe("see [Image #1] please")
  await act(async () => { await view.dispose() })
})

test("the markers in a draft show as chips above the composer", async () => {
  const text = await frame(100, 24, { draft: "what is [Image #1] and [Pasted Text #2]?" })
  expect(text).toContain("Image #1")
  expect(text).toContain("Pasted Text #2")
})

test("Esc interrupts, /stop names who, and a mistyped command stays in the box", async () => {
  const log: string[] = []
  const sent: string[] = []
  function Box() {
    const [draft, setDraft] = useState("")
    return <RoomView theme={theme} lines={lines} panes={panes} counts={counts} phase="connected" draft={draft} onDraft={setDraft}
      onSend={(t) => { sent.push(t ?? draft); setDraft("") }} onInterrupt={() => log.push("interrupt")} onStop={(w) => log.push(`stop ${w}`)} />
  }
  let view!: View
  await act(async () => { view = await testRender(<Box />, { width: 100, height: 24 }) })
  const step = (run: (v: View) => Promise<void> | void) => act(async () => { await run(view); await new Promise((r) => setTimeout(r, 40)) })
  await step((v) => v.mockInput.typeText("/stop red "))
  await step((v) => v.mockInput.pressEnter())
  expect(log).toEqual(["stop red"])
  await step((v) => v.mockInput.typeText("/shlef "))
  await step((v) => v.mockInput.pressEnter())
  expect(sent).toEqual([])
  await act(async () => { await view.renderOnce() })
  expect(view.captureCharFrame()).toContain("did you mean /shelf")
  await step((v) => v.mockInput.pressEscape())
  expect(log).toEqual(["stop red", "interrupt"])
  await act(async () => { await view.dispose() })
})

test("under the composer, each minion is its tag and a mark, with no state words", async () => {
  const table = [
    { avatar: "chief", display: "Chief", alias: "CF", role: "chief", lane: "chat", phase: "running", task: "", started_ms: 0, last_event_ms: 0, steps: 1, last_tool: "", error: "" },
    { avatar: "red", display: "Red", alias: "LR", role: "red", lane: "", phase: "idle", task: "", started_ms: 0, last_event_ms: 0, steps: 0, last_tool: "", error: "" },
    { avatar: "reviewer", display: "Reviewer", alias: "RV", role: "reviewer", lane: "", phase: "stopped", task: "", started_ms: 0, last_event_ms: 0, steps: 0, last_tool: "", error: "" },
  ]
  const text = await frame(110, 24, { panes: { ...panes, table } })
  expect(text).toMatch(/CF [⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]/)
  expect(text).toContain("LR ○")
  expect(text).toContain("RV ■")
  for (const word of ["thinking", "ready", "stopped"]) expect(text).not.toContain(word)
})

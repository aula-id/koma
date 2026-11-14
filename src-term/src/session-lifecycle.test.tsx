import { expect, test } from "bun:test"
import { mkdtempSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { act, useEffect, useMemo } from "react"
import { SyntaxStyle } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { SessionScreen } from "./session.tsx"
import { DaemonLink } from "./socket.ts"
import { decodeFrames, encodeFrame, type Control } from "./protocol.ts"
import type { DaemonTransport } from "./transport.ts"

class Transport implements DaemonTransport {
  readonly remote = true
  requests: any[] = []
  opened: string[] = []
  receive: (bytes: Uint8Array) => void = () => {}
  close: () => void = () => {}
  error: (error: unknown) => void = () => {}
  respond: (request: any) => void = () => {}
  mode: unknown = { Onboard: { cursor: 0 } }
  openingError: string | null = null
  connected = true
  ended = false
  async open(_id: string, receive: typeof this.receive, close: typeof this.close, error: typeof this.error) {
    this.opened.push(_id)
    this.receive = receive; this.close = close; this.error = error
    if (this.openingError) throw new Error(this.openingError)
    if (this.connected) this.emit({ Hello: { version: "fixture" } })
  }
  emit(event: unknown) { this.receive(encodeFrame({ event })) }
  write(bytes: Uint8Array) {
    for (const request of decodeFrames(bytes) as any[]) {
      this.requests.push(request)
      if (this.connected && request.Attach) this.emit({ Snapshot: { foreground_id: "s", sessions: [{ id: "s" }], global: { mode: this.mode } } })
      if (request === "GetMenuCapabilities") this.emit({ MenuCapabilities: { version: 1 } })
      this.respond(request)
    }
    return bytes.length
  }
  end() { this.ended = true; this.close() }
}

test("only the correlated successful quit reply exits and expected EOF stays quiet", async () => {
  const transport = new Transport(), controls: Control[] = []
  const link = new DaemonLink(() => {}, (control) => controls.push(control), transport)
  await link.start("s")
  const result = link.sendMenu("Onboard", { Key: { code: "Esc", mods: 0 } })
  const request = transport.requests.at(-1).Menu
  transport.emit({ MenuResult: { request_id: "other", error: null, exit: true } })
  expect(controls).toEqual([])
  transport.emit("Ack")
  expect(link.operations.size).toBe(1)
  transport.emit({ MenuResult: { request_id: request.request_id, error: null, exit: true } })
  expect(await result).toBeNull()
  transport.close()
  expect(controls).toEqual([{ kind: "exit" }])
  expect(transport.ended).toBe(true)
})

test("old menu replies and failed actions do not exit", async () => {
  const transport = new Transport(), controls: Control[] = []
  const link = new DaemonLink(() => {}, (control) => controls.push(control), transport)
  await link.start("s")
  for (const error of [null, "Cannot leave"]) {
    const result = link.sendMenu("Onboard", { Key: { code: "Enter", mods: 0 } })
    const { request_id } = transport.requests.at(-1).Menu
    transport.emit({ MenuResult: { request_id, error, ...(error ? { exit: true } : {}) } })
    expect(await result).toBe(error)
  }
  expect(controls).toEqual([])
  link.detach()
})

test("disconnect settles operations, preserves the first error and blocks further writes", async () => {
  const transport = new Transport(), controls: Control[] = []
  const link = new DaemonLink(() => {}, (control) => controls.push(control), transport)
  await link.start("s")
  const pending = link.sendMenu("Onboard", { Key: { code: "Enter", mods: 0 } })
  transport.error(new Error("original transport failure"))
  transport.close()
  expect(await pending).toBe("original transport failure")
  const count = transport.requests.length
  link.send({ SendKey: { code: "Enter", mods: 0 } })
  expect(await link.sendTracked({ ReplaceInput: { text: "draft", cursor: 5 } })).toBe("not connected")
  expect(await link.sendMenu("Onboard", { Key: { code: "Esc", mods: 0 } })).toBe("Not connected")
  expect(transport.requests.length).toBe(count)
  expect(controls).toEqual([{ kind: "disconnect", text: "original transport failure" }])
  expect(link.operations.size).toBe(0)
})

async function settle(view: Awaited<ReturnType<typeof testRender>>) {
  for (let pass = 0; pass < 3; pass++) {
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 10)); await view.waitForVisualIdle() })
  }
}

for (const key of ["escape", "q", "ctrl+c", "signal"]) test(`setup ${key} follows daemon-confirmed exit`, async () => {
  const transport = new Transport(), ctrlC = { current: () => {} }
  let exited = 0
  transport.respond = (request) => {
    if (request.Menu) transport.emit({ MenuResult: { request_id: request.Menu.request_id, error: null, exit: true } })
  }
  function Harness() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    useEffect(() => () => style.destroy(), [style])
    return <SessionScreen id="s" transport={transport} syntaxStyle={style} ctrlC={ctrlC} onExit={() => { exited++ }} onHome={() => {}} onOpen={() => {}} onRemote={() => {}} onPalette={() => {}} />
  }
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => { view = await testRender(<Harness />, { width: 80, height: 24, exitOnCtrlC: false }) })
  try {
    await settle(view)
    expect(view.captureCharFrame()).toContain("Welcome to Koma")
    await act(async () => {
      if (key === "escape") view.mockInput.pressEscape()
      else if (key === "q") await view.mockInput.typeText("q")
      else if (key === "ctrl+c") view.mockInput.pressCtrlC()
      else ctrlC.current()
      await new Promise((resolve) => setTimeout(resolve, 40))
    })
    expect(exited).toBe(1)
    expect(view.captureCharFrame()).not.toContain("not connected")
  } finally { await act(async () => { await view.dispose() }) }
})

for (const phase of ["connecting", "disconnected", "failed"]) test(`${phase} can leave without a daemon reply`, async () => {
  const transport = new Transport(), ctrlC = { current: () => {} }
  transport.connected = phase === "disconnected"
  if (phase === "failed") transport.openingError = "cannot open daemon"
  let exited = 0, home = 0
  function Harness() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    useEffect(() => () => style.destroy(), [style])
    return <SessionScreen id="s" transport={transport} syntaxStyle={style} ctrlC={ctrlC} onExit={() => { exited++ }} onHome={() => { home++ }} onOpen={() => {}} onRemote={() => {}} onPalette={() => {}} />
  }
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => { view = await testRender(<Harness />, { width: 80, height: 24, exitOnCtrlC: false }) })
  try {
    await settle(view)
    if (phase === "failed") expect(view.captureCharFrame()).toContain("cannot open daemon")
    if (phase === "disconnected") {
      await act(async () => { transport.error(new Error("connection lost during setup")) })
      await settle(view)
      expect(view.captureCharFrame()).toContain("connection lost during setup")
      expect(view.captureCharFrame()).not.toContain("Welcome to Koma")
    }
    const count = transport.requests.length
    await act(async () => { await view.mockInput.typeText("xyz"); await view.mockInput.pasteBracketedText("blocked") })
    expect(transport.requests.length).toBe(count)
    if (phase !== "connecting") {
      expect(view.captureCharFrame()).toContain("Connection suspended")
      expect(view.captureCharFrame()).toContain("Restart (r)")
      expect(view.captureCharFrame()).toContain("Exit (esc)")
      expect(view.captureCharFrame()).not.toContain("Back")
    }
    await act(async () => { view.mockInput.pressEscape(); await new Promise((resolve) => setTimeout(resolve, 40)) })
    expect(home).toBe(phase === "connecting" ? 1 : 0)
    expect(exited).toBe(phase === "connecting" ? 0 : 1)
    if (phase === "connecting") {
      await act(async () => { ctrlC.current() })
      expect(exited).toBe(1)
      await act(async () => { view.mockInput.pressCtrlC(); await new Promise((resolve) => setTimeout(resolve, 30)) })
      expect(exited).toBe(2)
    }
  } finally { await act(async () => { await view.dispose() }) }
})

for (const trigger of ["enter", "click", "shortcut"]) test(`suspended Restart reconnects the same session / ${trigger}`, async () => {
  const transport = new Transport()
  let exited = 0
  function Harness() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    useEffect(() => () => style.destroy(), [style])
    return <SessionScreen id="same-session" transport={transport} syntaxStyle={style} onExit={() => { exited++ }} onHome={() => {}} onOpen={() => {}} onRemote={() => {}} onPalette={() => {}} />
  }
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => { view = await testRender(<Harness />, { width: 50, height: 24, exitOnCtrlC: false }) })
  try {
    await settle(view)
    await act(async () => { transport.error(new Error("connection lost")) })
    await settle(view)
    expect(view.captureCharFrame()).toContain("Connection suspended")
    await act(async () => {
      if (trigger === "enter") view.mockInput.pressEnter()
      else if (trigger === "shortcut") await view.mockInput.typeText("r")
      else {
        const lines = view.captureCharFrame().split("\n"), y = lines.findIndex((line) => line.includes("Restart (r)"))
        await view.mockMouse.click(lines[y]!.indexOf("Restart") + 1, y)
      }
    })
    await settle(view)
    expect(transport.opened).toEqual(["same-session", "same-session"])
    expect(view.captureCharFrame()).toContain("Welcome to Koma")
    expect(exited).toBe(0)
  } finally { await act(async () => { await view.dispose() }) }
})

// `/security` turns the session screen into the room screen. Returning early above a hook
// changes the hook count and React throws "Rendered fewer hooks than expected".
test.skipIf(process.platform === "win32")("entering the secure room keeps the hook order", async () => {
  const transport = new Transport(), ctrlC = { current: () => {} }
  const saved = { bin: process.env.KOMA_BIN, data: process.env.KOMA_TEST_DATA_DIR }
  // The room asks `koma` to switch daemons. `true` accepts any arguments and does nothing.
  process.env.KOMA_BIN = "/usr/bin/true"
  process.env.KOMA_TEST_DATA_DIR = mkdtempSync(join(tmpdir(), "koma-room-hooks-"))
  function Harness() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
    useEffect(() => () => style.destroy(), [style])
    return <SessionScreen id="s" transport={transport} syntaxStyle={style} ctrlC={ctrlC} onExit={() => {}} onHome={() => {}} onOpen={() => {}} onRemote={() => {}} onPalette={() => {}} />
  }
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => { view = await testRender(<Harness />, { width: 80, height: 24, exitOnCtrlC: false }) })
  try {
    await settle(view)
    expect(view.captureCharFrame()).toContain("Welcome to Koma")
    // The fixture connects as a remote host. `/security` only enters the room locally, and the
    // handler reads this when the frame arrives.
    Object.assign(transport, { remote: false })
    await act(async () => { transport.emit("EnterSecure") })
    // The loading screen is up at once; the room replaces it when the daemon answers or fails.
    await view.renderOnce()
    expect(view.captureCharFrame()).toContain("opening session")
    // Nothing answers (`true` is the daemon). Ctrl+C on the loading screen leaves without a prompt.
    // Leaving only detaches, so the session screen is back at once.
    await act(async () => { ctrlC.current() })
    await act(async () => { await Bun.sleep(50) })
    await view.renderOnce()
    expect(view.captureCharFrame()).not.toContain("Quit to chat?")
    expect(view.captureCharFrame()).toContain("Welcome to Koma")
  } finally {
    await act(async () => { await view.dispose() })
    if (saved.bin === undefined) delete process.env.KOMA_BIN; else process.env.KOMA_BIN = saved.bin
    if (saved.data === undefined) delete process.env.KOMA_TEST_DATA_DIR; else process.env.KOMA_TEST_DATA_DIR = saved.data
  }
})

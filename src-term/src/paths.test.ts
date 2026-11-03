import { expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { createServer, type Socket } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { dataDir, savedPalette, sessionDir, socketPath } from "./paths.ts"
import { DaemonLink } from "./socket.ts"
import { encodeFrame, FrameReader } from "./protocol.ts"

test("launcher data directory is shared by palette, sessions, and Unix endpoints", () => {
  const dir = mkdtempSync(join(tmpdir(), "koma data "))
  const previous = process.env.KOMA_DATA_DIR
  process.env.KOMA_DATA_DIR = dir
  try {
    writeFileSync(join(dir, "config.json"), JSON.stringify({ palette: "light" }))
    expect(dataDir()).toBe(dir)
    expect(savedPalette()).toBe("light")
    expect(sessionDir("missing workdir", "fixture").startsWith(join(dir, "sessions"))).toBe(true)
    expect(socketPath("fixture", "linux")).toBe(join(dir, "run", "fixture.sock"))
    expect(socketPath("fixture", "win32")).toBe(String.raw`\\.\pipe\koma-fixture`)
  } finally {
    if (previous == null) delete process.env.KOMA_DATA_DIR
    else process.env.KOMA_DATA_DIR = previous
    rmSync(dir, { recursive: true, force: true })
  }
})

test("local daemon link probes and attaches over a Unix socket or Windows named pipe", async () => {
  const requests = await attachFixture([])
  expect(requests).toContainEqual({ Attach: { foreground_id: null, cwd: process.cwd() } })
  expect(requests).toContain("GetMenuCapabilities")
  expect(requests).not.toContainEqual({ SetProgressDeltas: { enabled: true } })
  expect(requests).not.toContain("Resync")
})

test("a daemon advertising progress deltas gets the opt-in, and a gap in history asks for a resync", async () => {
  const requests = await attachFixture(["session_progress"])
  expect(requests).toContainEqual({ SetProgressDeltas: { enabled: true } })
  expect(requests.filter((request) => request === "Resync")).toHaveLength(1)
})

/** Attach a DaemonLink to a fixture daemon and return every request it sent. */
async function attachFixture(capabilities: string[]): Promise<unknown[]> {
  const root = mkdtempSync(join(process.platform === "win32" ? tmpdir() : "/tmp", "koma-ipc-"))
  const previousDir = process.env.KOMA_DATA_DIR
  const previousBuild = process.env.KOMA_BUILD
  process.env.KOMA_DATA_DIR = root
  process.env.KOMA_BUILD = "fixture-build"
  mkdirSync(join(root, "run"))
  const id = crypto.randomUUID()
  const clients = new Set<Socket>()
  const requests: unknown[] = []
  let resolveSnapshot = () => {}
  const received = new Promise<void>(resolve => { resolveSnapshot = resolve })
  const errors: string[] = []
  const server = createServer(socket => {
    clients.add(socket)
    socket.on("error", () => {}) // The liveness probe closes before reading Hello.
    socket.on("close", () => clients.delete(socket))
    const reader = new FrameReader()
    socket.on("data", (data: Buffer) => {
      for (const request of reader.push(data)) {
        requests.push(request)
        if (typeof request === "object" && request !== null && "Attach" in request) {
          socket.write(encodeFrame({ seq: 0, event: { Hello: { version: "fixture-build", capabilities } } }))
          const sessions = [{ id, messages: [{ role: "user", content: "hi" }] }]
          const frame = encodeFrame({ seq: 1, event: { Snapshot: { foreground_id: id, sessions, global: { mode: "Chat" } } } })
          socket.write(frame.subarray(0, 3))
          socket.write(frame.subarray(3))
          if (capabilities.length > 0) {
            const gap = (seq: number) => encodeFrame({ seq, event: { Delta: { SessionProgress: { session_id: id, from_index: 5, messages: [] } } } })
            socket.write(gap(2))
            socket.write(gap(3))
          }
        }
      }
    })
  })
  const link = new DaemonLink(snapshot => { if (snapshot) resolveSnapshot() }, control => {
    if (control.kind === "error") errors.push(control.text)
  })
  try {
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject)
      server.listen(socketPath(id), resolve)
    })
    await link.start(id)
    await Promise.race([received, Bun.sleep(1000).then(() => { throw new Error("daemon snapshot not received") })])
    await Bun.sleep(50)
    expect(errors).toEqual([])
    return requests
  } finally {
    link.detach()
    for (const socket of clients) socket.destroy()
    await new Promise<void>(resolve => server.close(() => resolve()))
    if (previousDir == null) delete process.env.KOMA_DATA_DIR
    else process.env.KOMA_DATA_DIR = previousDir
    if (previousBuild == null) delete process.env.KOMA_BUILD
    else process.env.KOMA_BUILD = previousBuild
    rmSync(root, { recursive: true, force: true })
  }
}

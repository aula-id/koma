import { describe, expect, test } from "bun:test"
import { decodeFrames, encodeFrame, fold, FrameReader, modeName, needsRestart, showImagePath } from "./protocol.ts"
import { uuidV5Simple } from "./paths.ts"
import { themeFor } from "./theme.ts"

describe("frames", () => {
  test("round-trips a replace-input request", () => {
    const request = { ReplaceInput: { text: "hi", cursor: 2 } }
    const frames = decodeFrames(encodeFrame(request))
    expect(frames).toEqual([request])
  })

  test("splits a length prefix across pushes", () => {
    const bytes = encodeFrame({ SendKey: { code: "Enter", mods: 0 } })
    const readerBytes = bytes.subarray(0, 3)
    const rest = bytes.subarray(3)
    const reader = new FrameReader()
    expect(reader.push(readerBytes)).toEqual([])
    expect(reader.push(rest)).toEqual([{ SendKey: { code: "Enter", mods: 0 } }])
  })

  test("reads fragmented and coalesced frames without losing the next header", () => {
    const first = encodeFrame({ text: "안녕 🌱" })
    const second = encodeFrame({ n: 2 })
    const all = new Uint8Array(first.length + second.length)
    all.set(first)
    all.set(second, first.length)
    const reader = new FrameReader()
    const frames: unknown[] = []
    for (let i = 0; i < all.length; i += 3) frames.push(...reader.push(all.subarray(i, i + 3)))
    expect(frames).toEqual([{ text: "안녕 🌱" }, { n: 2 }])
  })

  test("rejects an oversized length and invalid JSON", () => {
    expect(() => new FrameReader().push(new Uint8Array([4, 0, 0, 1]))).toThrow("over the")
    expect(() => new FrameReader().push(new Uint8Array([0, 0, 0, 1, 0xff]))).toThrow()
  })
})

describe("fold", () => {
  test("snapshot then token and input", () => {
    const first = fold(null, {
      Snapshot: {
        foreground_id: "s1",
        sessions: [{ id: "s1", name: "one", cwd: "/tmp", messages: [] }],
        global: { input: "", cursor: 0, status: "ready", palette: "light", mode: "Chat" },
      },
    })
    expect(first?.global.palette).toBe("light")
    expect(first?.global.input).toBe("")
    const streamed = fold(first, { Delta: { TokenAppended: { session_id: "s1", text: "Hi" } } })
    expect(streamed?.sessions[0]?.streaming).toBe("Hi")
    const typed = fold(streamed, { Delta: { InputChanged: { text: "yo", cursor: 2 } } })
    expect(typed?.global.input).toBe("yo")
    expect(typed?.global.cursor).toBe(2)
    expect(typed?.sessions[0]?.streaming).toBe("Hi")
  })

  test("mode name reads a struct variant", () => {
    expect(modeName("Chat")).toBe("Chat")
    expect(modeName({ Settings: { tab: 1 } })).toBe("Settings")
  })
})

describe("images and session paths", () => {
  test("parses a show_image result", () => {
    expect(showImagePath("image resolved: /tmp/a.png (shot)\nRendered inline")).toBe("/tmp/a.png")
    expect(showImagePath("ok")).toBeNull()
  })

  test("uuid v5 matches the OID namespace", () => {
    expect(uuidV5Simple("6ba7b812-9dad-11d1-80b4-00c04fd430c8", "/tmp")).toBe("d19635b298845e6a98fe96538aac9d3f")
  })
})

describe("build skew", () => {
  test("restarts once when Hello disagrees with this binary", () => {
    expect(needsRestart("old", "new", false)).toBe(true)
    expect(needsRestart("new", "new", false)).toBe(false)
    expect(needsRestart(null, "new", false)).toBe(false)
    expect(needsRestart("old", "new", true)).toBe(false)
    expect(needsRestart("old", undefined, false)).toBe(false)
  })
})

describe("theme", () => {
  test("paints the named palette and falls back to dark", () => {
    const theme = themeFor("forest")
    expect(theme.name).toBe("forest")
    expect(theme.bg).toBe("#333d29")
    expect(theme.accent).toBe("#a68a64")
    const unknown = themeFor("nope")
    expect(unknown.name).toBe("nope")
    expect(unknown.bg).toBe("#000000")
    expect(themeFor(undefined).name).toBe("dark")
  })
})


test("queued media and build information survive snapshots, deltas, and old peers", () => {
  const attachment = { kind: "image", marker_n: 1, rel_path: "images/1.png" }
  const initial = { foreground_id: "s", sessions: [{ id: "s", pending_steer: ["queued [Image #1]"], pending_steer_attachments: [[attachment]] }], global: { current_version: "0.4.5", latest_version: "0.4.6" } }
  const snap = fold(null, { Snapshot: initial })!
  const delta = fold(snap, { Delta: { InputChanged: { text: "next", cursor: 4 } } })!
  expect(delta.sessions[0]!.pending_steer).toEqual(["queued [Image #1]"])
  expect(delta.sessions[0]!.pending_steer_attachments).toEqual([[attachment]])
  expect(delta.global.current_version).toBe("0.4.5")
  expect(delta.global.latest_version).toBe("0.4.6")
  expect(fold(null, { Snapshot: { sessions: [{ id: "old" }], global: {} } })!.sessions[0]!.pending_steer_attachments).toEqual([])
})

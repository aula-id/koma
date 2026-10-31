import { describe, expect, test } from "bun:test"
import { decodeFrames, encodeFrame, fold, FrameReader, modeName, needsRestart, needsResync, showImagePath } from "./protocol.ts"
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
  test("snapshots reuse unchanged messages and replace changed nested tool calls", () => {
    const raw = { sessions: [{ id: "s", messages: [{ role: "assistant", content: "ok", tool_calls: [{ id: "t", function: { name: "x", arguments: "{}" } }] }] }], global: {} }
    const first = fold(null, { Snapshot: raw })!
    const second = fold(first, { Snapshot: structuredClone(raw) })!
    expect(second.sessions[0].messages).toBe(first.sessions[0].messages)
    const changed = structuredClone(raw)
    changed.sessions[0].messages[0].tool_calls[0].function.arguments = "changed"
    expect(fold(second, { Snapshot: changed })!.sessions[0].messages[0]).not.toBe(first.sessions[0].messages[0])
  })
  test("history revision moves only when history is replaced", () => {
    const msg = (role: string, content: string) => ({ role, content })
    const base = [msg("system", "sys"), msg("user", "a"), msg("assistant", "b")]
    const snapOf = (messages: unknown[]) => ({ Snapshot: { sessions: [{ id: "s", messages }], global: {} } })
    const first = fold(null, snapOf(structuredClone(base)))!
    const appended = fold(first, snapOf([...structuredClone(base), msg("user", "c")]))!
    expect(appended.sessions[0]!.history_rev).toBe(0)
    expect(appended.sessions[0]!.messages[1]).toBe(first.sessions[0]!.messages[1])
    expect(appended.sessions[0]!.messages[3]!.content).toBe("c")

    const rewound = fold(appended, snapOf(structuredClone(base).slice(0, 2)))!
    expect(rewound.sessions[0]!.history_rev).toBe(1)
    expect(rewound.sessions[0]!.messages[1]).toBe(first.sessions[0]!.messages[1])

    const resent = fold(rewound, snapOf([msg("system", "sys"), msg("user", "a"), msg("assistant", "other")]))!
    expect(resent.sessions[0]!.history_rev).toBe(1)
    const replaced = fold(resent, snapOf([msg("system", "sys"), msg("user", "a"), msg("assistant", "edited")]))!
    expect(replaced.sessions[0]!.history_rev).toBe(2)

    const compacted = fold(replaced, snapOf([msg("system", "sys"), msg("assistant", "[summary]"), msg("assistant", "edited")]))!
    expect(compacted.sessions[0]!.history_rev).toBe(3)
    expect(fold(compacted, snapOf(structuredClone(compacted.sessions[0]!.messages)))!.sessions[0]!.messages).toBe(compacted.sessions[0]!.messages)
  })

  test("progress deltas append history, keep old objects, and flag gaps for a resync", () => {
    const snap = fold(null, { Snapshot: { sessions: [{ id: "s", messages: [{ role: "user", content: "a" }], streaming: "" }], global: {} } })!
    const progress = (from_index: number, messages: unknown[], extra: Record<string, unknown> = {}) =>
      ({ Delta: { SessionProgress: { session_id: "s", from_index, messages, streaming: null, stream_reasoning: "", tokens_in: 4, tokens_out: 2, tokens_cached: 1, cost: 0.5, ...extra } } })
    const step = progress(1, [{ role: "assistant", content: "b" }])
    expect(needsResync(snap, step)).toBe(false)
    const next = fold(snap, step)!
    const session = next.sessions[0]!
    expect(session.messages.map((m) => m.content)).toEqual(["a", "b"])
    expect(session.messages[0]).toBe(snap.sessions[0]!.messages[0])
    expect(session.history_rev).toBe(0)
    expect(session.streaming).toBeNull()
    expect([session.tokens_in, session.tokens_out, session.tokens_cached, session.cost]).toEqual([4, 2, 1, 0.5])

    const counters = fold(next, progress(2, [], { streaming: "", tokens_out: 9 }))!
    expect(counters.sessions[0]!.messages).toBe(session.messages)
    expect(counters.sessions[0]!.streaming).toBe("")

    expect(needsResync(next, progress(1, []))).toBe(true)
    expect(needsResync(next, progress(5, []))).toBe(true)
    expect(needsResync(next, { Delta: { SessionProgress: { session_id: "missing", from_index: 0, messages: [] } } })).toBe(true)
    expect(needsResync(next, { Delta: { TokenAppended: { session_id: "s", text: "x" } } })).toBe(false)
    expect(fold(next, progress(5, [{ role: "assistant", content: "lost" }]))!.sessions[0]!.messages).toBe(session.messages)
  })

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

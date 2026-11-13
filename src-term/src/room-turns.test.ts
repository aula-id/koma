import { expect, test } from "bun:test"
import { forSide, reduceLines, say, plain, teamCount, userCount, type Line } from "./room-turns.ts"
import type { RoomFrame } from "./room-link.ts"

const run = (frames: RoomFrame[], start: Line[] = []) => frames.reduce(reduceLines, start)

test("a turn with thinking, a tool and a reply is one message", () => {
  const lines = run([
    { type: "part", author: "chief", lane: "chat", part: { kind: "reasoning", text: "look at the cwd" } },
    { type: "chief", text: "Checking." },
    { type: "part", author: "chief", lane: "chat", part: { kind: "tool_call", id: "c1", name: "bash", args: '{"command":"pwd"}' } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "tool_result", id: "c1", ok: true, result: "/work" } },
    { type: "chief", text: "It is /work." },
  ])
  expect(lines).toHaveLength(1)
  expect(lines[0]!.items.map((i) => i.kind)).toEqual(["reasoning", "text", "tool", "text"])
  const tool = lines[0]!.items[2]!
  expect(tool.kind === "tool" && tool.result).toEqual({ ok: true, text: "/work" })
})

test("the user's message and another author start a new message", () => {
  const lines = run(
    [{ type: "chief", text: "one" }, { type: "lane", avatar: "red", text: "two" }, { type: "chief", text: "three" }],
    [say("you", "hi")],
  )
  expect(lines.map((l) => l.author)).toEqual(["you", "chief", "red", "chief"])
})

test("two replies in a row from one lane join, and the work lane stays apart from chat", () => {
  const lines = run([
    { type: "chief", text: "a" },
    { type: "chief", text: "b" },
    { type: "part", author: "chief", lane: "work", part: { kind: "tool_call", id: "w", name: "read", args: "{}" } },
  ])
  expect(lines).toHaveLength(2)
  expect(plain(lines[0]!)).toBe("a\n\nb")
  expect(lines[1]!.lane).toBe("work")
})

test("a result for an unknown call and non-message frames change nothing", () => {
  const start = [say("chief", "hello", "chat")]
  expect(run([{ type: "part", author: "chief", lane: "chat", part: { kind: "tool_result", id: "nope", ok: true, result: "x" } }], start)).toBe(start)
  expect(run([{ type: "notice", text: "n" }, { type: "closed" }], start)).toBe(start)
})

test("live pieces build one item that the whole reply then replaces in place", () => {
  const lines = run([
    { type: "part", author: "chief", lane: "chat", part: { kind: "reasoning_delta", text: "let me " } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "reasoning_delta", text: "think" } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "text_delta", text: "Hel" } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "text_delta", text: "lo" } },
  ])
  expect(lines).toHaveLength(1)
  expect(lines[0]!.items).toEqual([
    { kind: "reasoning", text: "let me think", streaming: true, to: "user" },
    { kind: "text", text: "Hello", streaming: true, to: "user" },
  ])
  const done = run([
    { type: "part", author: "chief", lane: "chat", part: { kind: "reasoning", text: "let me think." } },
    { type: "chief", text: "Hello there." },
  ], lines)
  expect(done).toHaveLength(1)
  expect(done[0]!.items).toEqual([
    { kind: "reasoning", text: "let me think.", to: "user" },
    { kind: "text", text: "Hello there.", to: "user" },
  ])
})

test("a second step streams after the first without merging into it", () => {
  const lines = run([
    { type: "part", author: "chief", lane: "chat", part: { kind: "text_delta", text: "first" } },
    { type: "chief", text: "first." },
    { type: "part", author: "chief", lane: "chat", part: { kind: "tool_call", id: "c", name: "bash", args: "{}" } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "tool_result", id: "c", ok: true, result: "x" } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "text_delta", text: "sec" } },
    { type: "part", author: "chief", lane: "chat", part: { kind: "text_delta", text: "ond" } },
  ])
  expect(lines).toHaveLength(1)
  expect(lines[0]!.items.map((i) => i.kind)).toEqual(["text", "tool", "text"])
  expect(lines[0]!.items[2]).toEqual({ kind: "text", text: "second", streaming: true, to: "user" })
  const closed = run([{ type: "visible", author: "chief", text: "lane failed: x" }], lines)
  expect(closed[0]!.items[2]).toEqual({ kind: "text", text: "second", streaming: false, to: "user" })
})

import { capLines, KEEP_LINES, mountedFrom, RESULT_CHARS } from "./room-turns.ts"
import { expect as want, test as it } from "bun:test"

it("capLines keeps the newest lines and clips a huge tool result", () => {
  const many = Array.from({ length: KEEP_LINES + 50 }, (_, i) => say("chief", `m${i}`))
  const kept = capLines(many)
  want(kept.length).toBe(KEEP_LINES)
  want(kept[kept.length - 1]).toBe(many[many.length - 1]!)
  const big: Line = { id: "t", author: "red", lane: "work", items: [{ kind: "tool", id: "1", name: "bash", args: "{}", result: { ok: true, text: "x".repeat(RESULT_CHARS * 3) } }] }
  const clipped = capLines([big])[0]!.items[0]!
  want(clipped.kind === "tool" && clipped.result!.text.length < RESULT_CHARS + 20).toBe(true)
  want(capLines(many.slice(0, 3))).toEqual(many.slice(0, 3))
})

it("mountedFrom draws fewer lines under pressure", () => {
  const lines = Array.from({ length: 200 }, (_, i) => say("chief", `m${i}`))
  want(mountedFrom(lines, true)).toBeGreaterThan(mountedFrom(lines, false))
  want(mountedFrom(lines.slice(0, 5), false)).toBe(0)
})

test("a line is for the user or the team by what the daemon said, and old lines are worked out from who spoke", () => {
  const lines = run([
    { type: "part", author: "chief", lane: "chat", part: { kind: "tool_call", id: "c1", name: "group_status", args: "{}" }, audience: "team" },
    { type: "chief", text: "Red is on it." },
    { type: "visible", author: "chief", text: "Assignment for Lab Red: probe", audience: "team" },
    { type: "part", author: "red", lane: "work", part: { kind: "tool_call", id: "c2", name: "bash", args: "{}" } },
    { type: "visible", author: "chief", text: "Assignment for Lab Red: older line" },
    { type: "visible", author: "chief", text: "All verified." },
    { type: "visible", author: "red", text: "stopped by you (work)" },
    { type: "visible", author: "red", text: "lane failed: x" },
    { type: "lane", avatar: "red", text: "I found it" },
  ])
  const says = (side: "user" | "team") => forSide(lines, side).map((l) => `${l.author}:${l.items.map((i) => i.kind === "tool" ? i.name : i.text).join("|")}`)
  expect(says("user")).toEqual(["chief:Red is on it.", "chief:All verified.", "red:stopped by you (work)", "red:I found it"])
  expect(says("team")).toEqual(["chief:group_status", "chief:Assignment for Lab Red: probe", "red:bash", "chief:Assignment for Lab Red: older line", "red:lane failed: x"])
})

test("one message with steps for the team and a reply for the user shows each side only its own", () => {
  const lines = run([
    { type: "part", author: "chief", lane: "chat", part: { kind: "tool_call", id: "c1", name: "group_status", args: "{}" }, audience: "team" },
    { type: "chief", text: "Red found two." },
  ])
  expect(lines).toHaveLength(1)
  const user = forSide(lines, "user")
  const team = forSide(lines, "team")
  expect(user[0]!.items.map((i) => i.kind)).toEqual(["text"])
  expect(team[0]!.items.map((i) => i.kind)).toEqual(["tool"])
  // The same objects come back while nothing changes, so a quiet room does not redraw.
  expect(forSide(lines, "user")[0]).toBe(user[0])
  expect(teamCount(lines)).toBe(1)
  expect(userCount(lines)).toBe(1)
  expect(userCount([say("you", "hi")])).toBe(0)
})

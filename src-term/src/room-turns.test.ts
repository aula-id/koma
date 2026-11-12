import { expect, test } from "bun:test"
import { reduceLines, say, plain, type Line } from "./room-turns.ts"
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

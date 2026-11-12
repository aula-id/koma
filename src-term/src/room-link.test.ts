import { expect, test } from "bun:test"
import { parseRoomFrame } from "./room-link.ts"

test("room frames keep the chief, the lanes, the shelf and the map", () => {
  expect(parseRoomFrame("hello")).toEqual({ type: "hello", kind: "" })
  expect(parseRoomFrame({ hello: { kind: "secure" } })).toEqual({ type: "hello", kind: "secure" })
  expect(parseRoomFrame({ chief: { text: "ready" } })).toEqual({ type: "chief", text: "ready" })
  expect(parseRoomFrame({ lane_chat: { avatar: "red", text: "looking" } })).toEqual({ type: "lane", avatar: "red", text: "looking" })
  expect(parseRoomFrame({ visible: { author: "chief", text: "dispatched" } })).toEqual({ type: "visible", author: "chief", text: "dispatched" })
  expect(parseRoomFrame({ status: { digest: "red running", timeline: "red ·", shelf: "claims/a", trajectory: "goal", open: 1, verified: 0, returned: 0, reports: 0 } })).toMatchObject({
    type: "status",
    digest: "red running",
    shelf: "claims/a",
    trajectory: "goal",
  })
  expect(parseRoomFrame({ closed: null })).toEqual({ type: "closed" })
  expect(parseRoomFrame({ Snapshot: {} })).toBeNull()
})

test("a part frame carries thinking, a tool call or a tool result", () => {
  expect(parseRoomFrame({ part: { author: "chief", lane: "chat", part: { kind: "reasoning", text: "hm" } } })).toEqual({ type: "part", author: "chief", lane: "chat", part: { kind: "reasoning", text: "hm" } })
  expect(parseRoomFrame({ part: { author: "red", lane: "work", part: { kind: "tool_call", id: "c1", name: "bash", args: "{}" } } })).toEqual({ type: "part", author: "red", lane: "work", part: { kind: "tool_call", id: "c1", name: "bash", args: "{}" } })
  expect(parseRoomFrame({ part: { author: "red", lane: "work", part: { kind: "tool_result", id: "c1", ok: false, result: "boom" } } })).toEqual({ type: "part", author: "red", lane: "work", part: { kind: "tool_result", id: "c1", ok: false, result: "boom" } })
  expect(parseRoomFrame({ part: { author: "red", lane: "work", part: { kind: "other" } } })).toBeNull()
})

test("live delta parts parse", () => {
  expect(parseRoomFrame({ part: { author: "chief", lane: "chat", part: { kind: "text_delta", text: "Hel" } } })).toEqual({ type: "part", author: "chief", lane: "chat", part: { kind: "text_delta", text: "Hel" } })
  expect(parseRoomFrame({ part: { author: "chief", lane: "chat", part: { kind: "reasoning_delta", text: "hm" } } })).toEqual({ type: "part", author: "chief", lane: "chat", part: { kind: "reasoning_delta", text: "hm" } })
})

test("a pasted frame carries the text to put in the draft", () => {
  expect(parseRoomFrame({ pasted: { insert: "[Image #1]" } })).toEqual({ type: "pasted", insert: "[Image #1]" })
})

import { createConnection, createServer } from "node:net"
import { mkdtempSync } from "node:fs"
import { FrameReader } from "./protocol.ts"
import { RoomLink } from "./room-link.ts"

test("a message sent before the daemon answers is delivered once it connects", async () => {
  const sock = `${mkdtempSync("/tmp/koma-link-")}/s.sock`
  const got: unknown[] = []
  const server = createServer((c) => {
    const reader = new FrameReader()
    c.on("data", (d: Buffer) => { for (const f of reader.push(d)) got.push(f) })
  })
  await new Promise<void>((r) => server.listen(sock, () => r()))
  const link = new RoomLink(() => {}, () => {})
  link.sendUser("first words")
  link.paste("pasted")
  await Bun.sleep(20)
  expect(got).toEqual([])
  const socket = createConnection({ path: sock })
  await new Promise<void>((r) => socket.once("connect", () => r()))
  link.attach(socket)
  await Bun.sleep(60)
  link.close()
  server.close()
  expect(got).toEqual(["hello", { user: { text: "first words" } }, { paste: { text: "pasted" } }])
})

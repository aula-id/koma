import { expect, test } from "bun:test"
import { suggest } from "./room-suggest.ts"

test("a bare @ lists everyone you can talk to, and never the clerk", () => {
  expect(suggest("@").rows.map((r) => r.label)).toEqual(["@chief", "@red", "@reviewer", "@reporter", "@red-web", "@red-api", "@researcher", "@re", "@builder", "@blue"])
})
test("matching is by prefix and fills a trailing space", () => {
  expect(suggest("@rev").rows).toEqual([{ label: "@reviewer", detail: "verifies findings independently", insert: "@reviewer " }])
})
test("slash offers the way back to coding", () => {
  expect(suggest("/").rows.map((r) => r.label)).toEqual(["/model", "/status", "/shelf", "/trajectory", "/report", "/screen", "/stop", "/security", "/leave"])
})
test("nothing is offered once the word is finished or plain text", () => {
  expect(suggest("@red probe").rows).toEqual([])
  expect(suggest("hello").rows).toEqual([])
  expect(suggest("@nobody").rows).toEqual([])
})

import { classify, suggest as suggestFor } from "./room-suggest.ts"
import { chosen } from "./room-popup.ts"

test("a mistyped command is caught, a path and a message are not", () => {
  expect(classify("/shel")).toEqual({ kind: "unknown", word: "/shel", near: "/shelf" })
  expect(classify("/shlef")).toEqual({ kind: "unknown", word: "/shlef", near: "/shelf" })
  expect(classify("/zzzz")).toEqual({ kind: "unknown", word: "/zzzz", near: "" })
  expect(classify("/status")).toEqual({ kind: "message" })
  expect(classify("/Users/me/app.log is huge")).toEqual({ kind: "message" })
  expect(classify("scan it")).toEqual({ kind: "message" })
})

test("/stop names who to stop, and defaults to everything", () => {
  expect(classify("/stop")).toEqual({ kind: "stop", who: "all" })
  expect(classify("/stop @Red")).toEqual({ kind: "stop", who: "red" })
  const rows = suggestFor("/stop ").rows
  expect(rows.map((r) => r.label)).toEqual(["all", "red", "reviewer", "reporter", "red-web", "red-api", "researcher", "re", "builder", "blue"])
  expect(chosen(rows[1]!)).toEqual({ run: "/stop red" })
  expect(chosen(suggestFor("/sto").rows[0]!)).toEqual({ insert: "/stop " })
})

test("/leave has two ways out, switch and stop, chosen like /stop's names", () => {
  expect(chosen(suggestFor("/lea").rows[0]!)).toEqual({ insert: "/leave " })
  const rows = suggestFor("/leave ").rows
  expect(rows.map((r) => r.label)).toEqual(["switch", "stop"])
  expect(chosen(rows[0]!)).toEqual({ run: "/leave switch" })
  expect(chosen(rows[1]!)).toEqual({ run: "/leave stop" })
  expect(suggestFor("/leave s").rows.map((r) => r.label)).toEqual(["switch", "stop"])
  expect(suggestFor("/leave st").rows.map((r) => r.label)).toEqual(["stop"])
  expect(classify("/leave stop")).toEqual({ kind: "message" })
  expect(classify("/leave later")).toEqual({ kind: "unknown", word: "/leave later", near: "/leave switch" })
})

test("/screen offers its four views and a choice under it runs", () => {
  expect(chosen(suggestFor("/scr").rows[0]!)).toEqual({ insert: "/screen " })
  const rows = suggestFor("/screen ").rows
  expect(rows.map((r) => r.label)).toEqual(["combine", "split", "user", "team"])
  expect(chosen(rows[2]!)).toEqual({ run: "/screen user" })
  expect(suggestFor("/screen t").rows.map((r) => r.label)).toEqual(["team"])
  expect(classify("/screen split")).toEqual({ kind: "screen", mode: "split" })
  expect(classify("/screen TEAM")).toEqual({ kind: "screen", mode: "team" })
  expect(classify("/screen wide")).toEqual({ kind: "unknown", word: "/screen wide", near: "/screen combine" })
})

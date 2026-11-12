import { expect, test } from "bun:test"
import { suggest } from "./room-suggest.ts"

test("a bare @ lists everyone you can talk to, and never the clerk", () => {
  expect(suggest("@").rows.map((r) => r.label)).toEqual(["@chief", "@red", "@reviewer", "@reporter"])
})
test("matching is by prefix and fills a trailing space", () => {
  expect(suggest("@rev").rows).toEqual([{ label: "@reviewer", detail: "verifies findings independently", insert: "@reviewer " }])
})
test("slash offers the way back to coding", () => {
  expect(suggest("/").rows.map((r) => r.label)).toEqual(["/security", "/leave"])
})
test("nothing is offered once the word is finished or plain text", () => {
  expect(suggest("@red probe").rows).toEqual([])
  expect(suggest("hello").rows).toEqual([])
  expect(suggest("@nobody").rows).toEqual([])
})

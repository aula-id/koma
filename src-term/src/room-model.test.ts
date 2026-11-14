import { expect, test } from "bun:test"
import { filterModels, minions } from "./room-model.ts"
import { parseRoomFrame } from "./room-link.ts"

const choices = [
  { id: "m1", name: "Fast", model_id: "fast-1", provider: "acme" },
  { id: "m2", name: "Strong", model_id: "strong-1", provider: "other" },
]

test("search matches name, id and provider, every word", () => {
  expect(filterModels(choices, "").length).toBe(2)
  expect(filterModels(choices, "stro").map((c) => c.id)).toEqual(["m2"])
  expect(filterModels(choices, "acme fast").map((c) => c.id)).toEqual(["m1"])
  expect(filterModels(choices, "acme strong")).toEqual([])
})

test("lanes group under their minion", () => {
  const rows = [
    { avatar: "red", display: "Lab Red", lane: "chat", model: "a" },
    { avatar: "red", display: "Lab Red", lane: "work", model: "b" },
    { avatar: "clerk", display: "Clerk", lane: "work", model: "c" },
  ]
  expect(minions(rows).map((m) => [m.avatar, m.lanes.length])).toEqual([["red", 2], ["clerk", 1]])
})

test("model and notice frames parse", () => {
  expect(parseRoomFrame({ models: { choices: [{ id: "m1", name: "Fast", model_id: "f", provider: "acme" }], lanes: [] } })).toEqual({ type: "models", choices: [{ id: "m1", name: "Fast", model_id: "f", provider: "acme" }], lanes: [] })
  expect(parseRoomFrame({ notice: { text: "no" } })).toEqual({ type: "notice", text: "no" })
})

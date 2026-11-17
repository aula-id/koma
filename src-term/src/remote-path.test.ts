import { expect, test } from "bun:test"
import { parentOf } from "./remote-path.ts"

test("parentOf ignores a trailing slash", () => {
  expect(parentOf("/home/x/")).toBe("/home")
  expect(parentOf("/home/x")).toBe("/home")
  expect(parentOf("/home")).toBe("/")
  expect(parentOf("/")).toBe("/")
})

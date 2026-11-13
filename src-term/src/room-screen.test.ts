import { expect, test } from "bun:test"
import { isScreen, splitLayout } from "./room-screen.ts"

test("a split sits side by side over 80 columns and stacks at 80 or fewer", () => {
  expect(splitLayout(81, 30)).toEqual({ kind: "side" })
  expect(splitLayout(200, 8)).toEqual({ kind: "side" })
  expect(splitLayout(80, 30)).toEqual({ kind: "stack" })
  expect(splitLayout(40, 30)).toEqual({ kind: "stack" })
})

test("a stacked split needs room for two panes and otherwise falls back to one view", () => {
  expect(splitLayout(80, 16)).toEqual({ kind: "stack" })
  expect(splitLayout(80, 15)).toEqual({ kind: "single" })
  expect(splitLayout(60, 10)).toEqual({ kind: "single" })
})

test("only the four views are screens", () => {
  for (const word of ["combine", "split", "user", "team"]) expect(isScreen(word)).toBe(true)
  for (const word of ["chief", "minions", "", "Split"]) expect(isScreen(word)).toBe(false)
})

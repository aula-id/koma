import { expect, test } from "bun:test"
import { CLOSED, chosen, completion, dismissed, moved, popupView } from "./room-popup.ts"

test("the popup opens for a lone / or @ word and closes once the word has a space", () => {
  expect(popupView("/st", CLOSED).open).toBe(true)
  expect(popupView("/status ", CLOSED).open).toBe(false)
  expect(popupView("hello", CLOSED).open).toBe(false)
  expect(popupView("@r", CLOSED).rows.map((r) => r.label)).toEqual(["@red", "@reviewer", "@reporter"])
})

test("up and down stop at the ends instead of wrapping", () => {
  const draft = "@r"
  let state = CLOSED
  let view = popupView(draft, state)
  state = moved(state, draft, view, -1)
  expect(popupView(draft, state).selected).toBe(0)
  for (let i = 0; i < 6; i++) state = moved(state, draft, popupView(draft, state), 1)
  expect(popupView(draft, state).selected).toBe(2)
})

test("editing the draft starts the highlight over and reopens a dismissed popup", () => {
  const view = popupView("/", CLOSED)
  const state = dismissed("/", view)
  expect(popupView("/", state).open).toBe(false)
  expect(popupView("/s", state).open).toBe(true)
  const down = moved(CLOSED, "/", view, 1)
  expect(popupView("/", down).selected).toBe(1)
  expect(popupView("/m", down).selected).toBe(0)
})

test("tab completes with a space, enter runs a command and only names a person", () => {
  const [model] = popupView("/mo", CLOSED).rows
  const [red] = popupView("@re", CLOSED).rows
  expect(completion(model!)).toBe("/model ")
  expect(chosen(model!)).toEqual({ run: "/model" })
  expect(chosen(red!)).toEqual({ insert: "@red " })
})

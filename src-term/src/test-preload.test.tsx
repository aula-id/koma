import { expect, test } from "bun:test"
import { act } from "react"
import { testRender } from "@opentui/react/test-utils"
import { RETRYING, retrying } from "../test-preload.ts"

const busy = () => new Error("ot_session_acquire_buffer_lease failed: OutputBusy")

test("every testRender screen read goes through the retry", async () => {
  const view = await testRender(<text>hello</text>, { width: 20, height: 2 })
  try {
    expect((view.captureCharFrame as unknown as Record<symbol, boolean>)[RETRYING]).toBe(true)
    await act(async () => { await view.waitForVisualIdle() })
    expect(view.captureCharFrame()).toContain("hello")
  } finally {
    await act(async () => { await view.dispose() })
  }
})

test("a busy buffer is read again; any other error is not", () => {
  let calls = 0
  expect(retrying(() => { if (calls++ < 3) throw busy(); return "frame" }, 0)()).toBe("frame")
  expect(calls).toBe(4)
  expect(() => retrying(() => { throw new Error("boom") }, 0)()).toThrow("boom")
  let forever = 0
  expect(() => retrying(() => { forever++; throw busy() }, 0)()).toThrow("OutputBusy")
  expect(forever).toBe(201)
})

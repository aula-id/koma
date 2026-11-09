import { expect, spyOn, test } from "bun:test"
import { act } from "react"
import { testRender } from "@opentui/react/test-utils"
import { cleaningNow, sweepVisibly, useCleaning, useIdleCollect } from "./pressure.ts"

const wait = (ms: number) => act(async () => { await new Promise((resolve) => setTimeout(resolve, ms)) })

test("a live turn gets only the non-blocking hint; the full sweep waits for quiet", async () => {
  const gc = spyOn(Bun, "gc").mockImplementation(() => {})
  const idle = { current: false }
  function Probe() {
    useIdleCollect(() => idle.current, 30, () => 200 * 1024 * 1024, 10, 5)
    return <box />
  }
  const view = await testRender(<Probe />, { width: 20, height: 6 })
  try {
    await wait(70)
    expect(gc).not.toHaveBeenCalledWith(true)
    expect(gc).toHaveBeenCalledWith(false)
    idle.current = true
    await wait(80)
    expect(gc).not.toHaveBeenCalledWith(true)
    await wait(1600)
    expect(gc).toHaveBeenCalledWith(true)
  } finally {
    gc.mockRestore()
    await act(async () => { await view.dispose() })
  }
})

test("the cleanup label paints before the blocking sweep and clears after it", async () => {
  const seen: boolean[] = []
  function Probe() {
    seen.push(useCleaning())
    return <box />
  }
  const view = await testRender(<Probe />, { width: 20, height: 6 })
  try {
    // The previous test's sweep holds the label for its minimum time.
    for (let i = 0; i < 100 && cleaningNow(); i++) await wait(20)
    expect(cleaningNow()).toBe(false)
    let ranWhileLabelled = false
    let labelBeforeWork = false
    await act(async () => {
      sweepVisibly(() => { ranWhileLabelled = cleaningNow() }, 20, 40)
      labelBeforeWork = cleaningNow()
    })
    expect(labelBeforeWork).toBe(true)
    expect(seen.at(-1)).toBe(true)
    await wait(30)
    expect(ranWhileLabelled).toBe(true)
    expect(cleaningNow()).toBe(true)
    await wait(40)
    expect(cleaningNow()).toBe(false)
    expect(seen.at(-1)).toBe(false)
  } finally {
    await act(async () => { await view.dispose() })
  }
})

test("a throwing sweep still clears the label", async () => {
  sweepVisibly(() => { throw new Error("boom") }, 1, 1)
  await new Promise((resolve) => setTimeout(resolve, 30))
  expect(cleaningNow()).toBe(false)
})

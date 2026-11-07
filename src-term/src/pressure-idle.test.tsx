import { expect, spyOn, test } from "bun:test"
import { act } from "react"
import { testRender } from "@opentui/react/test-utils"
import { useIdleCollect } from "./pressure.ts"

test("idle collect sweeps only while the turn is quiet", async () => {
  const gc = spyOn(Bun, "gc").mockImplementation(() => {})
  const idle = { current: false }
  function Probe() {
    useIdleCollect(() => idle.current, 30, () => 200 * 1024 * 1024)
    return <box />
  }
  const view = await testRender(<Probe />, { width: 20, height: 6 })
  try {
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 70)) })
    expect(gc).not.toHaveBeenCalled()
    idle.current = true
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 80)) })
    expect(gc).toHaveBeenCalledWith(true)
  } finally {
    gc.mockRestore()
    await act(async () => { await view.dispose() })
  }
})

/**
 * Test-only: screen reads retry while the renderer is still writing a frame.
 *
 * `captureCharFrame` takes a lease on the native output buffer. On a slow
 * runner a frame can still hold it, and the read throws `OutputBusy` even
 * though the test did nothing wrong. Every `testRender` result gets a
 * `captureCharFrame` that waits briefly and reads again; any other error is
 * rethrown unchanged.
 */
import { mock } from "bun:test"
import * as real from "@opentui/react/test-utils"

const realTestRender = real.testRender

export const RETRYING = Symbol.for("koma.test.retryingCapture")

export function retrying<T>(read: () => T, pause = 5): () => T {
  const wrapped = () => {
    for (let attempt = 0; ; attempt++) {
      try {
        return read()
      } catch (err) {
        if (attempt >= 200 || !String(err).includes("OutputBusy")) throw err
        Bun.sleepSync(pause)
      }
    }
  }
  return Object.assign(wrapped, { [RETRYING]: true })
}

mock.module("@opentui/react/test-utils", () => ({
  ...real,
  testRender: async (...args: Parameters<typeof realTestRender>) => {
    const view = await realTestRender(...args)
    view.captureCharFrame = retrying(view.captureCharFrame)
    return view
  },
}))

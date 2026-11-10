import { expect, test } from "bun:test"
import { act } from "react"
import { testRender } from "@opentui/react/test-utils"
import { ModeSurface } from "./surface.tsx"
import { themeFor } from "../theme.ts"
import { memoryFootprint } from "../footprint.ts"

const catalog = Array.from({ length: 698 }, (_, i) => ({
  id: `tool-${i}`, blurb: `tool ${i} does a thing to a target`, image: i % 3 ? "recon" : "web", exec: "net", domain: "recon", ready_mark: i % 3 ? "ready" : "remote",
}))
const images = ["minimal", "recon", "recon-2", "recon-ext", "web", "web-ext"].map((id, i) => ({
  id, blurb: `${id} image`, tag: "", size_label: i ? "—" : "973M", local: i === 0, active: i === 0, status: i ? "remote" : "active",
}))

function body(pane: string, current: number) {
  return {
    Security: {
      pane, selected: 3, images_selected: 1, catalog, images, catalog_filter: "all",
      status: { running: true, installed: true, runtime_ok: true, runtime: "colima", catalog_ready: 112, catalog_total: 698, catalog_source: "guest", tools: [], refused: [] },
      image_pull: { id: "recon", phase: "downloading", current, total: 12_000_000_000, bps: 40_000_000, layers_done: 3, layers: 9, elapsed_ms: current / 40_000 },
    },
  }
}

/** Footprint growth over three back-to-back windows of a streaming pull on `pane`. */
async function soak(pane: string, seconds: number): Promise<[number, number, number]> {
  let set: (mode: unknown) => void = () => {}
  const { useState } = await import("react")
  function Screen() {
    const [mode, setMode] = useState<unknown>(body(pane, 0))
    set = setMode
    return <ModeSurface mode={mode as never} theme={themeFor("dark")} columns={140} onMove={() => {}} send={() => {}} />
  }
  const view = await testRender(<Screen />, { width: 140, height: 40 })
  try {
    await act(async () => { await view.waitForVisualIdle() })
    const tick = async (ms: number) => { await act(async () => { await Bun.sleep(ms); await view.waitForVisualIdle() }) }
    await tick(2000)
    const settle = () => { Bun.gc(true); Bun.shrink?.() }
    let step = 0
    const window = async () => {
      settle()
      const before = memoryFootprint()
      for (let i = 0; i < seconds * 4; i++, step++) {
        // The agent pushes progress about four times a second.
        await act(async () => { set(body(pane, step * 10_000_000)) })
        await tick(250)
      }
      settle()
      return memoryFootprint() - before
    }
    // The first window pays for warm-up (JIT, heap high-water, glyph caches).
    // A leak keeps growing in every window after it.
    return [await window(), await window(), await window()]
  } finally {
    await act(async () => { await view.dispose() })
  }
}

test("a long image pull does not keep growing memory", async () => {
  const mib = (n: number) => (n / 1048576).toFixed(1)
  for (const pane of ["tools", "images"]) {
    const [warm, a, b] = await soak(pane, 10)
    console.log(`${pane}: +${mib(warm)} MiB warm-up, +${mib(a)} / +${mib(b)} MiB steady per 10s of pulling`)
    // A real leak grows in both steady windows. One noisy window (a late GC
    // or heap resize on a shared runner) does not.
    expect(Math.min(a, b)).toBeLessThan(6 * 1024 * 1024)
  }
}, 180_000)

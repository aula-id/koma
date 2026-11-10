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

/** Native growth while a pull streams progress for `seconds` on `pane`. */
async function soak(pane: string, seconds: number): Promise<number> {
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
    Bun.gc(true)
    const before = memoryFootprint()
    for (let i = 0; i < seconds * 4; i++) {
      // The agent pushes progress about four times a second.
      await act(async () => { set(body(pane, i * 10_000_000)) })
      await tick(250)
    }
    Bun.gc(true)
    return memoryFootprint() - before
  } finally {
    await act(async () => { await view.dispose() })
  }
}

test("a long image pull does not grow native memory", async () => {
  for (const pane of ["tools", "images"]) {
    const grown = await soak(pane, 12)
    console.log(`${pane}: +${(grown / 1048576).toFixed(1)} MiB over 12s of pulling`)
    // A leak at the measured 90ms-spinner rate is ~5 MiB per 12s; allow noise.
    expect(grown).toBeLessThan(4 * 1024 * 1024)
  }
}, 120_000)

import { expect, test } from "bun:test"
import { testRender } from "@opentui/react/test-utils"
import { act } from "react"
import { ModelTable } from "./modes/usage.tsx"
import { themeFor } from "./theme.ts"

const theme = themeFor("dark")
const models = [
  { model_id: "grok-4.7", total_cost: 1.5, call_count: 10, tokens_in: 100, tokens_cached: 10, tokens_out: 20 },
  { model_id: "a-very-long-model-name-that-will-not-fit-in-the-column-at-all-xyz", total_cost: 1, call_count: 2, tokens_in: 1, tokens_cached: 0, tokens_out: 1 },
]

async function shown(width: number) {
  let view!: Awaited<ReturnType<typeof testRender>>
  await act(async () => { view = await testRender(<ModelTable models={models} width={width} metric="cost" total={2.5} theme={theme} />, { width: width + 2, height: 12 }) })
  await act(async () => { await view.renderOnce() })
  return view
}

const rowOf = (view: Awaited<ReturnType<typeof testRender>>, text: string) => view.captureCharFrame().split("\n").findIndex((row) => row.includes(text))

test("selecting a model whose name fits does not repeat it or move the rows", async () => {
  const view = await shown(100)
  const long = rowOf(view, "a-very-long")
  const y = rowOf(view, "grok-4.7")
  await act(async () => { await view.mockMouse.click(3, y); await new Promise((r) => setTimeout(r, 30)) })
  await act(async () => { await view.renderOnce() })
  expect(view.captureCharFrame().split("\n").filter((row) => row.includes("grok-4.7")).length).toBe(1)
  expect(rowOf(view, "a-very-long")).toBe(long)
  await act(async () => { await view.renderer.destroy() })
})

test("a name that had to be cut is shown whole under its selected row", async () => {
  const view = await shown(100)
  await act(async () => { await view.mockMouse.click(3, rowOf(view, "a-very-long")); await new Promise((r) => setTimeout(r, 30)) })
  await act(async () => { await view.renderOnce() })
  expect(view.captureCharFrame()).toContain("a-very-long-model-name-that-will-not-fit-in-the-column-at-all-xyz")
  await act(async () => { await view.renderer.destroy() })
})

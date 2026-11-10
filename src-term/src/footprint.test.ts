import { expect, test } from "bun:test"
import { memoryFootprint } from "./footprint.ts"

test("footprint is at least RSS and follows a real allocation", () => {
  Bun.gc(true)
  const before = memoryFootprint()
  expect(before).toBeGreaterThanOrEqual(process.memoryUsage().rss * 0.9)
  const block = new Uint8Array(64 * 1024 * 1024).fill(7)
  const after = memoryFootprint()
  expect(after - before).toBeGreaterThan(48 * 1024 * 1024)
  expect(block[block.length - 1]).toBe(7)
})

test.if(process.platform === "darwin")("macOS footprint matches the system's footprint tool", () => {
  const out = Bun.spawnSync(["footprint", String(process.pid)]).stdout.toString()
  const match = /phys_footprint:\s+(\d+(?:\.\d+)?)\s+(KB|MB|GB)/.exec(out)
  if (!match) return // tool unavailable on this runner
  const scale = { KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3 }[match[2] as "KB" | "MB" | "GB"]
  const reported = Number(match[1]) * scale
  expect(Math.abs(memoryFootprint() - reported)).toBeLessThan(reported * 0.1 + 8 * 1024 * 1024)
})

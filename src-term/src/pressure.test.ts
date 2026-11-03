import { describe, expect, test } from "bun:test"
import { afterGc, CALM, CLEANUP_INTERVAL_MS, GC_USEFUL_BYTES, pressureLimits, pressureNotice, pressureTick } from "./pressure.ts"

const MIB = 1024 * 1024
const limits = { high: 768 * MIB, low: 512 * MIB, warn: 2048 * MIB }

describe("pressure valve", () => {
  test("reads limits from KOMA_TERM_MEM and keeps defaults for bad parts", () => {
    expect(pressureLimits(undefined)).toEqual(limits)
    expect(pressureLimits("1000,600,3000")).toEqual({ high: 1000 * MIB, low: 600 * MIB, warn: 3000 * MIB })
    expect(pressureLimits("900,,x")).toEqual({ high: 900 * MIB, low: 512 * MIB, warn: 2048 * MIB })
    expect(pressureLimits("400").low).toBe(400 * MIB)
  })

  test("shrinks above the high mark and restores only below the low mark", () => {
    const start = CLEANUP_INTERVAL_MS
    const hot = pressureTick(CALM, 800 * MIB, start, limits)
    expect(hot.reduced).toBe(true)
    expect(hot.cleanups).toBe(1)
    const between = pressureTick(hot, 600 * MIB, start + 5000, limits)
    expect(between).toBe(hot)
    const cool = pressureTick(between, 500 * MIB, start + 10_000, limits)
    expect(cool.reduced).toBe(false)
    expect(cool.cleanups).toBe(1)
  })

  test("cleans up at most once per interval, and the diagnostic signal ignores the high mark", () => {
    const first = pressureTick(CALM, 0, CLEANUP_INTERVAL_MS, limits, true)
    expect(first.cleanups).toBe(1)
    expect(pressureTick(first, 900 * MIB, CLEANUP_INTERVAL_MS + 1000, limits).cleanups).toBe(1)
    expect(pressureTick(first, 900 * MIB, 2 * CLEANUP_INTERVAL_MS, limits).cleanups).toBe(2)
  })

  test("stops forcing GC after two weak cleanups in a row, and then explains the short window", () => {
    const hot = pressureTick(CALM, 800 * MIB, CLEANUP_INTERVAL_MS, limits)
    expect(afterGc(hot, 800 * MIB, 800 * MIB - GC_USEFUL_BYTES)).toBe(hot)
    const once = afterGc(hot, 800 * MIB, 790 * MIB)
    expect(once.gcUseful).toBe(true)
    expect(pressureNotice(once)).toBeNull()
    expect(afterGc(once, 800 * MIB, 700 * MIB).gcMisses).toBe(0)
    const stuck = afterGc(once, 800 * MIB, 790 * MIB)
    expect(stuck.gcUseful).toBe(false)
    expect(pressureNotice(stuck)).toBe("Memory high; reduced history window.")
  })

  test("warns above the warning mark until RSS falls below the low mark", () => {
    const warned = pressureTick(CALM, 2100 * MIB, CLEANUP_INTERVAL_MS, limits)
    expect(warned.warned).toBe(true)
    expect(pressureNotice(warned)).toContain("Restarting the terminal keeps your session")
    expect(pressureTick(warned, 600 * MIB, 2 * CLEANUP_INTERVAL_MS, limits).warned).toBe(true)
    expect(pressureTick(warned, 500 * MIB, 2 * CLEANUP_INTERVAL_MS, limits).warned).toBe(false)
  })
})

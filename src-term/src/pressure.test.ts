import { describe, expect, test } from "bun:test"
import { afterGc, CALM, CLEANUP_INTERVAL_MS, GC_MISSES, GC_USEFUL_BYTES, IDLE_COLLECT, IDLE_COLLECT_FLOOR, IDLE_COLLECT_MS, RECYCLE_GAP_MS, idleCollectDue, idleCollectResult, idleCollectStep, pressureLimits, pressureNotice, pressureTick, ramTone, recycleArmed, viewRecycleDue } from "./pressure.ts"

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

  test("a quiet turn collects on the interval, and a live turn never does", () => {
    expect(idleCollectDue(0, IDLE_COLLECT_MS, true)).toBe(true)
    expect(idleCollectDue(0, IDLE_COLLECT_MS / 2, true)).toBe(false)
    expect(idleCollectDue(0, IDLE_COLLECT_MS, false)).toBe(false)
    expect(idleCollectDue(1_000, 1_000 + IDLE_COLLECT_MS, false)).toBe(false)
  })

  test("a quiet collect skips a live turn and a small process, then pauses after two weak sweeps", () => {
    const hot = IDLE_COLLECT_FLOOR + GC_USEFUL_BYTES
    expect(idleCollectStep(IDLE_COLLECT, IDLE_COLLECT_MS, false, hot).collect).toBe(false)
    expect(idleCollectStep(IDLE_COLLECT, IDLE_COLLECT_MS, true, IDLE_COLLECT_FLOOR - 1).collect).toBe(false)
    const due = idleCollectStep(IDLE_COLLECT, IDLE_COLLECT_MS, true, hot)
    expect(due.collect).toBe(true)
    const once = idleCollectResult(due.state, hot, hot)
    expect(once.paused).toBe(false)
    expect(once.misses).toBe(1)
    const paused = idleCollectResult(once, hot, hot)
    expect(paused.misses).toBe(GC_MISSES)
    expect(paused.paused).toBe(true)
    expect(idleCollectStep(paused, IDLE_COLLECT_MS * 4, true, hot).collect).toBe(false)
    const climbed = idleCollectStep(paused, IDLE_COLLECT_MS * 4, true, paused.lastRss + GC_USEFUL_BYTES)
    expect(climbed.collect).toBe(true)
    expect(idleCollectResult(due.state, hot, hot - GC_USEFUL_BYTES).misses).toBe(0)
  })

  test("the view recycles only after a short window and a useless sweep, and not twice inside the gap", () => {
    const stuck = { ...pressureTick(CALM, 800 * MIB, CLEANUP_INTERVAL_MS, limits), gcUseful: false }
    expect(viewRecycleDue(stuck, 800 * MIB, limits.high, true, true)).toBe(true)
    expect(viewRecycleDue(stuck, 800 * MIB, limits.high, false, true)).toBe(false)
    expect(viewRecycleDue(stuck, limits.high - 1, limits.high, true, true)).toBe(false)
    expect(viewRecycleDue({ ...stuck, gcUseful: true }, 800 * MIB, limits.high, true, true)).toBe(false)
    expect(viewRecycleDue({ ...stuck, reduced: false }, 800 * MIB, limits.high, true, true)).toBe(false)
    expect(viewRecycleDue(stuck, 800 * MIB, limits.high, true, false)).toBe(false)
    expect(recycleArmed(1_000, 0)).toBe(true)
    expect(recycleArmed(1_000, 1_000)).toBe(false)
    expect(recycleArmed(1_000 + RECYCLE_GAP_MS, 1_000)).toBe(true)
  })

  test("the RAM row stays dim until the window shrinks, and turns error at the restart mark", () => {
    expect(ramTone(CALM)).toBe("dim")
    const hot = pressureTick(CALM, 800 * MIB, CLEANUP_INTERVAL_MS, limits)
    expect(ramTone(hot)).toBe("warn")
    expect(ramTone(pressureTick(CALM, 2100 * MIB, CLEANUP_INTERVAL_MS, limits))).toBe("error")
  })
})

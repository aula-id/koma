import { gcAndSweep } from "bun:jsc"
import { useEffect, useMemo, useRef, useState } from "react"

const MIB = 1024 * 1024
export const PRESSURE_CHECK_MS = 5000
export const CLEANUP_INTERVAL_MS = 30_000
/** Quiet-turn collection. A live turn is left alone so a full GC does not stall a paint. */
export const IDLE_COLLECT_MS = 30_000
/** Below this, a quiet collect only pauses. The idle client is already small. */
export const IDLE_COLLECT_FLOOR = 128 * MIB
/** After a view recycle, wait this long before asking for another. */
export const RECYCLE_GAP_MS = 10 * 60 * 1000
/** Forced GC stops after this many cleanups in a row free less than `GC_USEFUL_BYTES`. */
export const GC_USEFUL_BYTES = 32 * MIB
export const GC_MISSES = 2

export type PressureLimits = { high: number; low: number; warn: number }

export type Pressure = {
  /** The history window is shrunk until RSS falls below `low`. */
  reduced: boolean
  cleanups: number
  lastAt: number
  gcUseful: boolean
  /** RSS lags behind freed pages, so one weak GC is not enough to give up. */
  gcMisses: number
  /** GC stopped helping; tell the reader why history is shorter. */
  hint: boolean
  warned: boolean
}

export const CALM: Pressure = { reduced: false, cleanups: 0, lastAt: 0, gcUseful: true, gcMisses: 0, hint: false, warned: false }

/** `KOMA_TERM_MEM="high,low,warn"` in MiB. Missing or invalid parts keep their defaults. */
export function pressureLimits(raw = process.env.KOMA_TERM_MEM): PressureLimits {
  const [high, low, warn] = (raw ?? "").split(",").map((part) => {
    const n = Number(part.trim())
    return part.trim() !== "" && Number.isFinite(n) && n > 0 ? n * MIB : null
  })
  const limits = { high: high ?? 768 * MIB, low: low ?? 512 * MIB, warn: warn ?? 2048 * MIB }
  if (limits.low > limits.high) limits.low = limits.high
  return limits
}

/** One RSS reading. A cleanup happened when `cleanups` grew. `forced` is the diagnostic signal. */
export function pressureTick(state: Pressure, rss: number, now: number, limits: PressureLimits, forced = false): Pressure {
  let next = state
  if (rss >= limits.warn && !next.warned) next = { ...next, warned: true }
  if (next.warned && rss < limits.low) next = { ...next, warned: false }
  if (next.reduced && !forced && rss < limits.low) next = { ...next, reduced: false }
  if ((forced || rss > limits.high) && now - next.lastAt >= CLEANUP_INTERVAL_MS) {
    next = { ...next, reduced: true, lastAt: now, cleanups: next.cleanups + 1 }
  }
  return next
}

export function afterGc(state: Pressure, before: number, after: number): Pressure {
  if (before - after >= GC_USEFUL_BYTES) return state.gcMisses ? { ...state, gcMisses: 0 } : state
  const gcMisses = state.gcMisses + 1
  if (gcMisses < GC_MISSES) return { ...state, gcMisses }
  return { ...state, gcMisses, gcUseful: false, hint: true }
}

export function pressureNotice(state: Pressure): string | null {
  if (state.warned) return "Memory is high. Restarting the terminal keeps your session."
  if (state.hint && state.reduced) return "Memory high; reduced history window."
  return null
}

/**
 * Whether this tick may collect. A live turn never collects.
 * The timer can fire a little early; a quarter of the interval is slack, capped at one second.
 */
export function idleCollectDue(lastAt: number, now: number, idle: boolean, interval = IDLE_COLLECT_MS): boolean {
  if (!idle) return false
  const slack = Math.min(1000, Math.floor(interval / 4))
  return now - lastAt >= interval - slack
}

export type IdleCollect = { lastAt: number; misses: number; paused: boolean; lastRss: number }

export const IDLE_COLLECT: IdleCollect = { lastAt: 0, misses: 0, paused: false, lastRss: 0 }

/**
 * One quiet tick. A live turn, a small process, and a paused sweep do not collect.
 * A paused sweep resumes after RSS climbs by `GC_USEFUL_BYTES`.
 */
export function idleCollectStep(state: IdleCollect, now: number, idle: boolean, rss: number, interval = IDLE_COLLECT_MS): { state: IdleCollect; collect: boolean } {
  if (!idle || rss < IDLE_COLLECT_FLOOR) return { state, collect: false }
  let current = state
  if (current.paused) {
    if (rss < current.lastRss + GC_USEFUL_BYTES) return { state: current, collect: false }
    current = { ...current, paused: false, misses: 0 }
  }
  if (!idleCollectDue(current.lastAt, now, true, interval)) return { state: current, collect: false }
  return { state: { ...current, lastAt: now, lastRss: rss }, collect: true }
}

/** Two weak sweeps pause collection until RSS climbs. A useful sweep clears the misses. */
export function idleCollectResult(state: IdleCollect, before: number, after: number): IdleCollect {
  if (before - after >= GC_USEFUL_BYTES) return state.misses ? { ...state, misses: 0, lastRss: after } : { ...state, lastRss: after }
  const misses = state.misses + 1
  if (misses < GC_MISSES) return { ...state, misses, lastRss: after }
  return { ...state, misses, paused: true, lastRss: after }
}

/**
 * A fresh launch may recycle. A process spawned to replace a hot view waits out the gap.
 */
export function recycleArmed(now: number, recycledAt = Number(process.env.KOMA_RECYCLED_AT)): boolean {
  if (!Number.isFinite(recycledAt) || recycledAt <= 0) return true
  return now - recycledAt >= RECYCLE_GAP_MS
}

/**
 * Restart the view only when the window is already short, the sweep stopped helping,
 * and this process is still above the high mark. A live turn is left alone.
 */
export function viewRecycleDue(state: Pressure, rss: number, high: number, idle: boolean, armed: boolean): boolean {
  return armed && idle && state.reduced && !state.gcUseful && rss >= high
}

/**
 * Sweep the JavaScript heap while the turn is quiet.
 * This does not shrink the history window and does not change the RAM color.
 * Native widget and wasm pages stay; only memory the JS heap just freed can return.
 */
export function useIdleCollect(
  idle: () => boolean,
  intervalMs = IDLE_COLLECT_MS,
  readRss: () => number = () => process.memoryUsage().rss,
): void {
  const idleRef = useRef(idle)
  const readRef = useRef(readRss)
  idleRef.current = idle
  readRef.current = readRss
  const state = useRef(IDLE_COLLECT)
  useEffect(() => {
    const timer = setInterval(() => {
      const rss = readRef.current()
      const step = idleCollectStep(state.current, Date.now(), idleRef.current(), rss, intervalMs)
      state.current = step.state
      if (!step.collect) return
      gcAndSweep()
      Bun.gc(true)
      state.current = idleCollectResult(state.current, rss, readRef.current())
    }, intervalMs)
    return () => clearInterval(timer)
  }, [intervalMs])
}

/** Footer color for this client's RSS. The restart case wins over a short window. */
export function ramTone(state: Pressure): "dim" | "warn" | "error" {
  if (state.warned) return "error"
  if (state.reduced || state.hint) return "warn"
  return "dim"
}

/**
 * Shrink the history window under memory pressure and restore it once RSS drops.
 * State resets per session; GC usefulness and the high-memory warning carry over.
 * `onCleanup` runs before the shrink renders; `afterLayout` runs GC once it has committed.
 */
export function usePressureValve(
  sessionId: string,
  onCleanup: () => void,
  afterLayout: (run: () => void) => void,
): Pressure {
  const limits = useMemo(() => pressureLimits(), [])
  const [state, setState] = useState(CALM)
  const ref = useRef(state)
  const hooks = useRef({ onCleanup, afterLayout })
  hooks.current = { onCleanup, afterLayout }
  const gcFrom = useRef<number | null>(null)
  const session = useRef(sessionId)
  const update = (next: Pressure) => {
    if (next === ref.current) return
    ref.current = next
    setState(next)
  }

  useEffect(() => {
    if (session.current === sessionId) return
    session.current = sessionId
    const kept = ref.current
    update({ ...CALM, gcUseful: kept.gcUseful, gcMisses: kept.gcMisses, warned: kept.warned })
  }, [sessionId])

  useEffect(() => {
    const check = (forced: boolean) => {
      const rss = process.memoryUsage().rss
      const prev = ref.current
      const next = pressureTick(prev, rss, Date.now(), limits, forced)
      if (next.cleanups !== prev.cleanups) {
        hooks.current.onCleanup()
        if (next.gcUseful) gcFrom.current = rss
      }
      update(next)
    }
    const timer = setInterval(() => check(false), PRESSURE_CHECK_MS)
    // Windows has no SIGUSR2. The RSS timer still shrinks the window and can recycle the view.
    if (process.platform === "win32") return () => clearInterval(timer)
    const signal = () => check(true)
    process.on("SIGUSR2", signal)
    return () => { clearInterval(timer); process.off("SIGUSR2", signal) }
  }, [limits])

  useEffect(() => {
    const before = gcFrom.current
    if (before == null) return
    gcFrom.current = null
    hooks.current.afterLayout(() => {
      Bun.gc(true)
      update(afterGc(ref.current, before, process.memoryUsage().rss))
    })
  }, [state.cleanups])

  return state
}

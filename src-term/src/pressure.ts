import { useEffect, useMemo, useRef, useState } from "react"

const MIB = 1024 * 1024
export const PRESSURE_CHECK_MS = 5000
export const CLEANUP_INTERVAL_MS = 30_000
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
    if (process.platform === "win32") return
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

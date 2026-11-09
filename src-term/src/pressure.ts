import { gcAndSweep } from "bun:jsc"
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react"

const MIB = 1024 * 1024
export const PRESSURE_CHECK_MS = 5000
export const CLEANUP_INTERVAL_MS = 30_000
/** Quiet-turn collection. A live turn is left alone so a full GC does not stall a paint. */
export const IDLE_COLLECT_MS = 30_000
/** Below this, a quiet collect only pauses. The idle client is already small. */
export const IDLE_COLLECT_FLOOR = 128 * MIB
/** After a view recycle, wait this long before asking for another. Kept for the launcher gap. */
export const RECYCLE_GAP_MS = 10 * 60 * 1000
/** Shortest time "Cleanup…" stays on the status line, so a fast sweep is still readable. */
export const CLEANUP_MIN_MS = 500
/** Wait before a blocking sweep so the "Cleanup…" label paints first. */
export const CLEANUP_PAINT_MS = 60
/** During a live turn, a non-blocking collection hint at most this often. */
export const LIVE_COLLECT_MS = 10_000
/** A full sweep runs this long after a turn ends, once the last paint has settled. */
export const TURN_END_SWEEP_MS = 1500
/** How often the collector looks at the turn and RSS. */
export const COLLECT_TICK_MS = 1000
/** Forced GC stops after this many cleanups in a row free less than `GC_USEFUL_BYTES`. */
export const GC_USEFUL_BYTES = 32 * MIB
export const GC_MISSES = 2
/** A valve that gave up on GC tries again once RSS has grown this much since. */
export const GC_REARM_BYTES = 128 * MIB

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
  /** RSS when GC was given up; `GC_REARM_BYTES` above it turns GC back on. */
  stuckRss: number
}

export const CALM: Pressure = { reduced: false, cleanups: 0, lastAt: 0, gcUseful: true, gcMisses: 0, hint: false, warned: false, stuckRss: 0 }

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
  // Freed pages often stay in RSS, so two weak sweeps can be a false verdict. Growth re-arms GC.
  if (!next.gcUseful && rss >= next.stuckRss + GC_REARM_BYTES) next = { ...next, gcUseful: true, gcMisses: 0, hint: false }
  if ((forced || rss > limits.high) && now - next.lastAt >= CLEANUP_INTERVAL_MS) {
    next = { ...next, reduced: true, lastAt: now, cleanups: next.cleanups + 1 }
  }
  return next
}

export function afterGc(state: Pressure, before: number, after: number): Pressure {
  if (before - after >= GC_USEFUL_BYTES) return state.gcMisses ? { ...state, gcMisses: 0 } : state
  const gcMisses = state.gcMisses + 1
  if (gcMisses < GC_MISSES) return { ...state, gcMisses }
  return { ...state, gcMisses, gcUseful: false, hint: true, stuckRss: after }
}

export function pressureNotice(state: Pressure): string | null {
  if (state.warned) return "Memory is high. History window reduced."
  if (state.hint && state.reduced) return "Memory high; reduced history window."
  return null
}

/**
 * Whether a quiet tick may run the full sweep. A live turn never does.
 * The timer can fire a little early; a quarter of the interval is slack, capped at one second.
 */
export function idleCollectDue(lastAt: number, now: number, idle: boolean, interval = IDLE_COLLECT_MS): boolean {
  if (!idle) return false
  const slack = Math.min(1000, Math.floor(interval / 4))
  return now - lastAt >= interval - slack
}

export type IdleCollect = {
  lastAt: number
  misses: number
  paused: boolean
  lastRss: number
  /** Last non-blocking hint during a live turn. */
  lightAt: number
  /** A turn ran since the last turn-end sweep. */
  liveSeen: boolean
  /** When the turn went quiet; 0 while it is live. */
  quietSince: number
}

export const IDLE_COLLECT: IdleCollect = { lastAt: 0, misses: 0, paused: false, lastRss: 0, lightAt: 0, liveSeen: false, quietSince: 0 }

/** `light` is a non-blocking hint; `full` blocks the UI thread and shows "Cleanup…". */
export type CollectAction = "none" | "light" | "full"

/**
 * One collector tick.
 * - Live turn: a non-blocking hint every `LIVE_COLLECT_MS`, so garbage does not pile up
 *   through a long turn without stalling the stream.
 * - Turn end: one full sweep `TURN_END_SWEEP_MS` after the turn goes quiet.
 * - Quiet: the interval sweep from `idleCollectStep`.
 * A process under `IDLE_COLLECT_FLOOR` never collects.
 */
export function collectStep(state: IdleCollect, now: number, idle: boolean, rss: number, interval = IDLE_COLLECT_MS): { state: IdleCollect; action: CollectAction } {
  if (!idle) {
    const live = state.liveSeen && state.quietSince === 0 ? state : { ...state, liveSeen: true, quietSince: 0 }
    if (rss < IDLE_COLLECT_FLOOR || now - live.lightAt < LIVE_COLLECT_MS) return { state: live, action: "none" }
    return { state: { ...live, lightAt: now }, action: "light" }
  }
  let current = state
  if (current.liveSeen) {
    if (current.quietSince === 0) current = { ...current, quietSince: now }
    if (now - current.quietSince < TURN_END_SWEEP_MS) return { state: current, action: "none" }
    current = { ...current, liveSeen: false, quietSince: 0 }
    // The turn made fresh garbage, so a paused sweep gets another try.
    if (rss >= IDLE_COLLECT_FLOOR) return { state: { ...current, lastAt: now, lastRss: rss, paused: false, misses: 0 }, action: "full" }
  }
  const step = idleCollectStep(current, now, true, rss, interval)
  return { state: step.state, action: step.collect ? "full" : "none" }
}

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
 * Recycle used to exit the view and let the launcher respawn it. That kill/reappear
 * flicker is disabled: keep the process, shrink the window, and sweep in place.
 * The launcher still accepts exit 76 if something else asks for a recycle.
 */
export function viewRecycleDue(_state: Pressure, _rss: number, _high: number, _idle: boolean, _armed: boolean): boolean {
  return false
}

let cleaning = 0
const cleaningListeners = new Set<() => void>()

function setCleaning(delta: number) {
  cleaning = Math.max(0, cleaning + delta)
  for (const listener of cleaningListeners) listener()
}

/** True while a blocking sweep is pending or running. */
export function cleaningNow(): boolean {
  return cleaning > 0
}

/** The status line reads this to show "Cleanup…". */
export function useCleaning(): boolean {
  return useSyncExternalStore(
    (listener) => { cleaningListeners.add(listener); return () => { cleaningListeners.delete(listener) } },
    cleaningNow,
  )
}

/** Full collection, then hand freed heap pages back so RSS can actually fall. */
export function fullSweep(): void {
  gcAndSweep()
  Bun.gc(true)
  ;(Bun as { shrink?: () => void }).shrink?.()
}

/**
 * Show "Cleanup…", let it paint, then run the blocking `work`.
 * The label stays at least `CLEANUP_MIN_MS`. A throw from `work` is dropped and the label still clears.
 */
export function sweepVisibly(work: () => void, paintMs = CLEANUP_PAINT_MS, minMs = CLEANUP_MIN_MS): void {
  const started = Date.now()
  setCleaning(1)
  setTimeout(() => {
    try {
      work()
    } catch {
      // A failed sweep frees nothing; it must not take the view down from a timer.
    } finally {
      setTimeout(() => setCleaning(-1), Math.max(0, minMs - (Date.now() - started)))
    }
  }, paintMs)
}

/**
 * Collect while the client runs. A live turn gets a non-blocking hint; a full sweep
 * waits for the turn to end or go quiet, and shows "Cleanup…" while it blocks.
 * This does not shrink the history window and does not change the RAM color.
 * Native widget and wasm pages stay; only memory the JS heap freed can return.
 */
export function useIdleCollect(
  idle: () => boolean,
  intervalMs = IDLE_COLLECT_MS,
  readRss: () => number = () => process.memoryUsage().rss,
  tickMs = COLLECT_TICK_MS,
  paintMs = CLEANUP_PAINT_MS,
): void {
  const idleRef = useRef(idle)
  const readRef = useRef(readRss)
  idleRef.current = idle
  readRef.current = readRss
  const state = useRef(IDLE_COLLECT)
  useEffect(() => {
    let sweeping = false
    const timer = setInterval(() => {
      if (sweeping) return
      const rss = readRef.current()
      const step = collectStep(state.current, Date.now(), idleRef.current(), rss, intervalMs)
      state.current = step.state
      if (step.action === "light") {
        Bun.gc(false)
        return
      }
      if (step.action !== "full") return
      sweeping = true
      sweepVisibly(() => {
        try {
          fullSweep()
          state.current = idleCollectResult(state.current, rss, readRef.current())
        } finally {
          sweeping = false
        }
      }, paintMs)
    }, tickMs)
    return () => clearInterval(timer)
  }, [intervalMs, tickMs, paintMs])
}

/** Footer color for this client's RSS. The high-memory warning wins over a short window. */
export function ramTone(state: Pressure): "dim" | "warn" | "error" {
  if (state.warned) return "error"
  if (state.reduced || state.hint) return "warn"
  return "dim"
}

/**
 * Shrink the history window under memory pressure and restore it once RSS drops.
 * State resets per session; GC usefulness and the high-memory warning carry over.
 * `onCleanup` runs before the shrink renders; `afterLayout` starts the sweep once it has
 * committed, and the sweep shows "Cleanup…" before it blocks.
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
    update({ ...CALM, gcUseful: kept.gcUseful, gcMisses: kept.gcMisses, warned: kept.warned, stuckRss: kept.stuckRss })
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
    // Windows has no SIGUSR2. The RSS timer still shrinks the window.
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
      sweepVisibly(() => {
        fullSweep()
        update(afterGc(ref.current, before, process.memoryUsage().rss))
      })
    })
  }, [state.cleanups])

  return state
}

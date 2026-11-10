import { readFileSync } from "node:fs"

/**
 * Memory this process is charged for, in bytes: what Activity Monitor or `top`
 * would show, not just the pages that happen to be in RAM.
 *
 * RSS undercounts badly on macOS. The OS compresses or swaps pages nobody
 * touches, and leaked native memory is exactly that: a terminal measured at
 * 290 MB RSS had a 1.96 GB footprint, so the pressure valve never fired.
 *
 * - macOS: `phys_footprint` from `proc_pid_rusage` (counts compressed and swapped pages).
 * - Linux: RSS plus `VmSwap`.
 * - Elsewhere, or if a probe fails: RSS.
 */
export function memoryFootprint(): number {
  const rss = process.memoryUsage().rss
  try {
    if (process.platform === "darwin") return Math.max(rss, darwinFootprint() ?? 0)
    if (process.platform === "linux") return rss + linuxSwap()
  } catch {
    // fall through to RSS
  }
  return rss
}

type Rusage = (pid: number, flavor: number, buffer: Uint8Array) => number
let rusage: Rusage | null | undefined
const buffer = new Uint8Array(512)
/** `struct rusage_info_v0`: 16-byte uuid, then u64 fields; `ri_phys_footprint` is the 8th. */
const PHYS_FOOTPRINT_OFFSET = 72
const RUSAGE_INFO_V0 = 0

function darwinFootprint(): number | null {
  if (rusage === undefined) rusage = loadRusage()
  if (!rusage) return null
  if (rusage(process.pid, RUSAGE_INFO_V0, buffer) !== 0) return null
  const value = new DataView(buffer.buffer).getBigUint64(PHYS_FOOTPRINT_OFFSET, true)
  return Number(value)
}

function loadRusage(): Rusage | null {
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { dlopen, FFIType } = require("bun:ffi") as typeof import("bun:ffi")
    const lib = dlopen("/usr/lib/libSystem.B.dylib", {
      proc_pid_rusage: { args: [FFIType.i32, FFIType.i32, FFIType.ptr], returns: FFIType.i32 },
    })
    return (pid, flavor, buf) => lib.symbols.proc_pid_rusage(pid, flavor, buf)
  } catch {
    return null
  }
}

function linuxSwap(): number {
  const status = readFileSync("/proc/self/status", "utf8")
  const match = /^VmSwap:\s+(\d+)\s+kB/m.exec(status)
  return match ? Number(match[1]) * 1024 : 0
}

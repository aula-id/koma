import { appendFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

type Counts = { loadedMessages: number; mountedMessages: number; mountedTurns: number; loadedContentBytes: number; streaming: boolean; cleanups: number }

export const memlogEnabled = process.env.KOMA_TERM_MEMLOG === "1"
let source: (() => Counts) | null = null
const traffic = { fullSnapshots: 0, deltas: 0, frameBytes: 0 }

/** Counts are read once per log tick, never per snapshot. */
export function registerMemorySource(read: (() => Counts) | null): void {
  if (memlogEnabled) source = read
}

export function countFrames(bytes: number, events: readonly unknown[]): void {
  if (!memlogEnabled) return
  traffic.frameBytes += bytes
  for (const frame of events) {
    const event = (frame as { event?: unknown } | null)?.event
    if (!event || typeof event !== "object") continue
    if ("Snapshot" in event) traffic.fullSnapshots++
    else if ("Delta" in event) traffic.deltas++
  }
}

export function startMemoryLog(): void {
  if (!memlogEnabled) return
  const path = join(tmpdir(), `koma-term-memory-${process.pid}.jsonl`)
  let writing = false
  const timer = setInterval(() => {
    if (writing) return
    let counts: Counts | null = null
    try {
      counts = source?.() ?? null
    } catch {
      counts = null
    }
    writing = true
    const line = JSON.stringify({ time: new Date().toISOString(), memory: process.memoryUsage(), ...traffic, ...counts }) + "\n"
    appendFile(path, line, { mode: 0o600 }).then(
      () => { writing = false },
      () => clearInterval(timer),
    )
  }, 1000)
  timer.unref()
}

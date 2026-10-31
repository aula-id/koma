import { appendFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

type Counts = { loadedMessages: number; mountedMessages: number; mountedTurns: number; loadedContentBytes: number; streaming: boolean; cleanups: number }
let counts: Counts | null = null

export function setMemoryCounts(next: Counts | null): void {
  if (process.env.KOMA_TERM_MEMLOG === "1") counts = next
}

export function startMemoryLog(): void {
  if (process.env.KOMA_TERM_MEMLOG !== "1") return
  const path = join(tmpdir(), `koma-term-memory-${process.pid}.jsonl`)
  const timer = setInterval(() => {
    try {
      appendFileSync(path, JSON.stringify({ time: new Date().toISOString(), memory: process.memoryUsage(), ...counts }) + "\n", { mode: 0o600 })
    } catch {
      clearInterval(timer)
    }
  }, 1000)
  timer.unref()
}

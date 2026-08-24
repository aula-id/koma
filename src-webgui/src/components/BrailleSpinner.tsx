import { useSyncExternalStore } from 'react'

// koma's terminal "cooking" indicator glyphs (TUI parity). Every in-app
// loading spinner shares this ONE animation so they tick in lockstep off a
// single interval instead of each running its own timer.
export const BRAILLE_FRAMES = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏']
const BRAILLE_INTERVAL_MS = 80

// One app-wide ticker, ref-counted by live subscribers: the interval only runs
// while at least one spinner is mounted, and every spinner reads the same frame.
let frameIdx = 0
let timer: ReturnType<typeof setInterval> | null = null
const listeners = new Set<() => void>()

function subscribe(cb: () => void): () => void {
  listeners.add(cb)
  if (timer === null) {
    timer = setInterval(() => {
      frameIdx = (frameIdx + 1) % BRAILLE_FRAMES.length
      listeners.forEach((l) => l())
    }, BRAILLE_INTERVAL_MS)
  }
  return () => {
    listeners.delete(cb)
    if (listeners.size === 0 && timer !== null) {
      clearInterval(timer)
      timer = null
    }
  }
}

// The current braille frame, driven by the shared ticker above. Standalone hook
// so a caller can render the glyph however it likes (e.g. a status badge).
export function useBrailleFrame(): string {
  const idx = useSyncExternalStore(
    subscribe,
    () => frameIdx,
    () => frameIdx,
  )
  return BRAILLE_FRAMES[idx]
}

// Drop-in replacement for `<Loader2 size={N} className="animate-spin ..." />`:
// same fixed square footprint (so layout never shifts frame-to-frame), color
// inherited from `className` (e.g. text-koma-accent) like the Lucide icon it
// replaces. Do NOT pass `animate-spin` — the braille frames self-animate.
//
// Main-thread driven (setInterval + React). Fine for chrome badges. For the
// session-switch / cold-boot overlay use {@link BootSpinner} instead — CSS
// compositor animation survives Snapshot parse jank.
export function BrailleSpinner({
  size = 14,
  className = '',
}: {
  size?: number
  className?: string
}) {
  const frame = useBrailleFrame()
  return (
    <span
      aria-hidden
      className={`inline-flex flex-none items-center justify-center leading-none ${className}`}
      style={{ width: size, height: size, fontSize: size, lineHeight: 1 }}
    >
      {frame}
    </span>
  )
}

/**
 * Game-style loader: CSS `transform` spin on the compositor thread.
 * Keeps moving while JS is blocked on fat Snapshot apply / React reconcile.
 * Prefer this on full-screen boot/switch overlays; keep {@link BrailleSpinner}
 * for small in-app busy affordances.
 */
export function BootSpinner({
  size = 28,
  className = '',
}: {
  size?: number
  className?: string
}) {
  const border = Math.max(2, Math.round(size / 12))
  return (
    <span
      aria-hidden
      className={`koma-boot-ring ${className}`}
      style={{ width: size, height: size, borderWidth: border }}
    />
  )
}

/**
 * Small CSS braille glyph for phase rows (no React re-render per frame).
 * Content keyframes are best-effort; still cheaper than useSyncExternalStore.
 */
export function BootBrailleGlyph({
  size = 13,
  className = '',
}: {
  size?: number
  className?: string
}) {
  return (
    <span
      aria-hidden
      className={`koma-boot-braille ${className}`}
      style={{ width: size, height: size, fontSize: size }}
    />
  )
}

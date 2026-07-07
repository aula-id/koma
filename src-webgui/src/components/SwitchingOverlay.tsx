import { useEffect, useRef, useState } from 'react'
import { motion } from 'framer-motion'
import { Loader2 } from 'lucide-react'
import { useKoma } from '../store/koma'

type SwitchingOverlayProps = {
  onCancel: () => void
}

// After this long still switching, surface a "taking longer than expected"
// hint so a genuinely-stuck swap (the deterministic Hub-clear in koma.ts
// missing some future degrade path, or a wedged daemon) doesn't leave the
// user staring at a silent spinner with no signal. A legit build-skew daemon
// restart can take ~8s, so this must not fire before that.
const STUCK_HINT_MS = 10_000
// Last-resort auto-cancel so the UI can never be trapped behind this overlay
// forever, even if the user never notices the hint or clicks Cancel.
const AUTO_CANCEL_MS = 25_000

// Full-screen loader shown while a session swap (SelectSession/NewSession) is
// in flight. The host gives no reliable "swap started" push on this build —
// the attach can block synchronously for seconds (build-skew daemon restart,
// cold session spawn) — and during that window the webview would otherwise
// keep rendering the stale previous session. Raised optimistically by
// ResumePalette's startSwitching() the instant the request is sent; cleared
// by koma.ts the moment the next authoritative Snapshot lands (success) or
// Hub lands (attach failure/degrade — the host always bounces back to the
// swapper with a fresh Hub push on that path).
//
// The in-flight swap itself cannot be interrupted (the client thread blocks
// synchronously on attach), so Cancel is best-effort: it just dismisses the
// loader and bails back to the resume hub. Whatever session was being
// switched to still lands eventually and is applied normally when its
// Snapshot arrives.
export function SwitchingOverlay({ onCancel }: SwitchingOverlayProps) {
  const to = useKoma((s) => s.ui.switchingTo)
  const [stuck, setStuck] = useState(false)
  const timersRef = useRef<{ hint: number; autoCancel: number } | null>(null)
  const onCancelRef = useRef(onCancel)
  onCancelRef.current = onCancel

  useEffect(() => {
    if (!to) {
      setStuck(false)
      return
    }
    const hint = window.setTimeout(() => setStuck(true), STUCK_HINT_MS)
    const autoCancel = window.setTimeout(() => onCancelRef.current(), AUTO_CANCEL_MS)
    timersRef.current = { hint, autoCancel }
    return () => {
      window.clearTimeout(hint)
      window.clearTimeout(autoCancel)
      timersRef.current = null
    }
  }, [to])

  if (!to) return null

  return (
    <div className="absolute inset-0 z-[60] flex flex-col items-center justify-center gap-4 bg-koma-bg/90 backdrop-blur-sm">
      <motion.div
        initial={{ opacity: 0, scale: 0.96 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="flex flex-col items-center gap-4"
      >
        <Loader2 size={28} className="animate-spin text-koma-accent" />
        <div className="text-[13px] text-koma-fg opacity-70">switching to {to}…</div>
        {stuck && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.16, ease: 'easeOut' }}
            className="text-[12px] text-koma-fg opacity-50"
          >
            Taking longer than expected…
          </motion.div>
        )}
        <button
          onClick={onCancel}
          className={`rounded-md border px-3 py-1.5 text-[12px] transition-colors ${
            stuck
              ? 'border-koma-accent bg-koma-accent/10 text-koma-accent opacity-100 hover:bg-koma-accent/20'
              : 'border-koma-border bg-koma-panel text-koma-fg opacity-70 hover:opacity-100 hover:bg-koma-hover'
          }`}
        >
          Cancel
        </button>
      </motion.div>
    </div>
  )
}

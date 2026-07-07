import { motion } from 'framer-motion'
import { Loader2 } from 'lucide-react'
import { useKoma } from '../store/koma'

type SwitchingOverlayProps = {
  onCancel: () => void
}

// Full-screen loader shown while a session swap (SelectSession/NewSession) is
// in flight. The host gives no reliable "swap started" push on this build —
// the attach can block synchronously for seconds (build-skew daemon restart,
// cold session spawn) — and during that window the webview would otherwise
// keep rendering the stale previous session. Raised optimistically by
// ResumePalette's startSwitching() the instant the request is sent; cleared
// by koma.ts the moment the next authoritative Snapshot lands.
//
// The in-flight swap itself cannot be interrupted (the client thread blocks
// synchronously on attach), so Cancel is best-effort: it just dismisses the
// loader and bails back to the resume hub. Whatever session was being
// switched to still lands eventually and is applied normally when its
// Snapshot arrives.
export function SwitchingOverlay({ onCancel }: SwitchingOverlayProps) {
  const to = useKoma((s) => s.ui.switchingTo)
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
        <button
          onClick={onCancel}
          className="rounded-md border border-koma-border bg-koma-panel px-3 py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:opacity-100 hover:bg-koma-hover"
        >
          Cancel
        </button>
      </motion.div>
    </div>
  )
}

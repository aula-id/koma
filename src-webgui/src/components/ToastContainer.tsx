import { useEffect } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { AlertTriangle, Info, X } from 'lucide-react'
import { useKoma } from '../store/koma'

// How long a toast stays before it auto-dismisses. Errors (safeguard blocks,
// harness flags) linger a touch longer than info notices so they're not missed.
const INFO_MS = 4_000
const ERROR_MS = 7_000

// Transient toast surface — the GUI home for the host's per-session
// `SessionRuntime.toast` (Status envelope `toast`/`kind`). Safeguard/harness
// notices ("harness flagged: …", "classifier unavailable …") + generic host
// toasts land here. Bottom-centre, above the composer, auto-dismissing; a
// manual close is offered too. Coloured by severity (error vs info) via the
// --koma-* roles, lucide icons (never glyphs).
export function ToastContainer() {
  const toast = useKoma((s) => s.ui.toast)
  const dismissToast = useKoma((s) => s.dismissToast)

  // Arm a fresh auto-dismiss timer whenever a new toast id appears. Keyed on the
  // id (not the object) so a re-fired identical-text toast still restarts the
  // clock, and so a dismissed→null gap doesn't leave a stale timer running.
  useEffect(() => {
    if (!toast) return
    const ttl = toast.kind === 'error' ? ERROR_MS : INFO_MS
    const t = window.setTimeout(() => dismissToast(toast.id), ttl)
    return () => window.clearTimeout(t)
  }, [toast, dismissToast])

  const isError = toast?.kind === 'error'

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-4 z-[70] flex justify-center px-4">
      <AnimatePresence>
        {toast && (
          <motion.div
            key={toast.id}
            initial={{ opacity: 0, y: 12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.98 }}
            transition={{ duration: 0.16, ease: 'easeOut' }}
            className={`pointer-events-auto flex max-w-[520px] items-start gap-2 rounded-md border px-3 py-2 shadow-lg backdrop-blur-sm ${
              isError
                ? 'border-koma-error/50 bg-koma-error/10 text-koma-error'
                : 'border-koma-border bg-koma-panel/95 text-koma-fg'
            }`}
          >
            {isError ? (
              <AlertTriangle size={14} className="mt-0.5 flex-none" />
            ) : (
              <Info size={14} className="mt-0.5 flex-none text-koma-accent" />
            )}
            <span className="min-w-0 flex-1 whitespace-pre-wrap break-words text-[12px] leading-snug">
              {toast.text}
            </span>
            <button
              onClick={() => dismissToast(toast.id)}
              aria-label="Dismiss"
              className="flex-none opacity-50 transition-opacity hover:opacity-100"
            >
              <X size={13} />
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}

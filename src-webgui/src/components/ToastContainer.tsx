import { useEffect, useMemo } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Info, OctagonAlert, TriangleAlert, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { ToastEntry } from '../store/koma'

// How long a toast stays before it auto-dismisses. Errors (safeguard blocks,
// harness flags) linger a touch longer than info notices so they're not missed.
const INFO_MS = 4_000
const ERROR_MS = 7_000

// `PaletteInfo.colors` is the 11 role colours in a FIXED order (see koma.ts):
// [bg, fg, dim, accent, panel, sel_bg, sel_fg, success, warn, error, info].
// These are the indices of the four severity roles we tint toast icons with.
const ROLE_INDEX: Record<'success' | 'warn' | 'error' | 'info', number> = {
  success: 7,
  warn: 8,
  error: 9,
  info: 10,
}

// Every severity renders through the SAME neutral themed container (panel
// background, fg text, dim border) — only the icon (glyph + colour) changes.
// "success"/unrecognised kinds keep today's Info glyph (the "compacted ✓"
// notice already reads fine); only the tint colour differs per role.
const ICONS: Record<ToastEntry['kind'], typeof Info> = {
  info: Info,
  success: Info,
  warn: TriangleAlert,
  error: OctagonAlert,
}

// Transient toast surface — the GUI home for the host's per-session
// `SessionRuntime.toast` (Status envelope `toast`/`kind`). Safeguard/harness
// notices ("harness flagged: …", "classifier unavailable …") + generic host
// toasts land here. Bottom-centre, above the composer, auto-dismissing; a
// manual close is offered too. Every severity shares one neutral themed
// container; only the lucide icon (glyph + colour, tinted from the active
// palette's role colours) marks severity — never a full-colour container.
export function ToastContainer() {
  const toast = useKoma((s) => s.ui.toast)
  const dismissToast = useKoma((s) => s.dismissToast)
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)

  // Arm a fresh auto-dismiss timer whenever a new toast id appears. Keyed on the
  // id (not the object) so a re-fired identical-text toast still restarts the
  // clock, and so a dismissed→null gap doesn't leave a stale timer running.
  useEffect(() => {
    if (!toast) return
    const ttl = toast.kind === 'error' ? ERROR_MS : INFO_MS
    const t = window.setTimeout(() => dismissToast(toast.id), ttl)
    return () => window.clearTimeout(t)
  }, [toast, dismissToast])

  // Active palette's severity role colours, keyed the same as ICONS. Falls
  // back to the themed fg colour when the active palette isn't advertised yet
  // (e.g. before the first Config push) or is missing a role entry.
  const roleColor = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    const at = (idx: number) => active?.colors?.[idx] || 'var(--koma-fg)'
    return {
      success: at(ROLE_INDEX.success),
      warn: at(ROLE_INDEX.warn),
      error: at(ROLE_INDEX.error),
      info: at(ROLE_INDEX.info),
    }
  }, [palettes, theme])

  const kind = toast?.kind ?? 'info'
  const Icon = ICONS[kind]
  const tint = roleColor[kind]

  return (
    <div className="pointer-events-none absolute inset-x-0 top-12 z-[70] flex justify-end px-4">
      <AnimatePresence>
        {toast && (
          <motion.div
            key={toast.id}
            initial={{ opacity: 0, y: 12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.98 }}
            transition={{ duration: 0.16, ease: 'easeOut' }}
            className="pointer-events-auto flex max-w-[340px] items-start gap-2 rounded-md border border-koma-border bg-koma-panel/95 px-2.5 py-1.5 text-koma-fg shadow-lg backdrop-blur-sm"
          >
            <Icon size={14} className="mt-0.5 flex-none" style={{ color: tint }} />
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

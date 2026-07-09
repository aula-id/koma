import { useEffect, useMemo, useRef, useState } from 'react'
import { motion } from 'framer-motion'
import { Check, Loader2, Minus, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { LoadPhase } from '../store/koma'

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

// Danger/error palette role tint — same lookup ToastContainer/SessionRowActions
// use for their error icon (index 9 of the 11-role `PaletteInfo.colors` array).
// Never a hardcoded red/orange.
const ERROR_ROLE_INDEX = 9

function useErrorTint(): string {
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)
  return useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[ERROR_ROLE_INDEX] || 'var(--koma-fg)'
  }, [palettes, theme])
}

// One phase row's status glyph: running = filled accent dot with the same
// pulse treatment as the live-session dots elsewhere (StartScreen/ResumePalette);
// pending = dim hollow dot; done = dim check; skipped = dim dash; failed = an
// error-role-tinted x.
function PhaseGlyph({ phase, errorTint }: { phase: LoadPhase; errorTint: string }) {
  switch (phase) {
    case 'running':
      return <span className="h-2 w-2 flex-none animate-pulse rounded-full bg-koma-accent" />
    case 'done':
      return <Check size={13} className="flex-none text-koma-fg opacity-50" />
    case 'skipped':
      return <Minus size={13} className="flex-none text-koma-fg opacity-40" />
    case 'failed':
      return <X size={13} className="flex-none" style={{ color: errorTint }} />
    case 'pending':
    default:
      return <span className="h-2 w-2 flex-none rounded-full border border-koma-fg opacity-30" />
  }
}

function PhaseRow({ label, phase, errorTint }: { label: string; phase: LoadPhase; errorTint: string }) {
  return (
    <div className="flex items-center gap-2 text-[12px] text-koma-fg opacity-80">
      <PhaseGlyph phase={phase} errorTint={errorTint} />
      <span>{label}</span>
    </div>
  )
}

// TUI-parity startup splash — the koma wordmark + the two cold-session warm-up
// phase lines (indexing workspace / reading project docs), mirroring the TUI's
// startup screen. The wordmark reuses StartScreen/Onboarding's brand text
// recipe (font-bold text-koma-fg, no accent on the text itself); the whole app
// is already set in the KomaMono monospace face globally (#app in styles.css),
// so the phase labels are monospace with no extra classes needed.
function LoadingSplash({ workspace, awareness }: { workspace: LoadPhase; awareness: LoadPhase }) {
  const errorTint = useErrorTint()
  return (
    <div className="flex flex-col items-center gap-4">
      <span className="text-[22px] font-bold text-koma-fg">koma</span>
      <div className="flex flex-col gap-1.5">
        <PhaseRow label="indexing workspace" phase={workspace} errorTint={errorTint} />
        <PhaseRow label="reading project docs" phase={awareness} errorTint={errorTint} />
      </div>
    </div>
  )
}

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
//
// Also stays mounted (rendering the TUI-parity startup splash) while
// `ui.loading?.active` is true, even after `ui.switchingTo` has already
// cleared — the cold-session warm-up (workspace indexing / awareness reading)
// can outlast the attach itself, so the splash's own condition is ORed in
// alongside the classic switch-spinner condition.
export function SwitchingOverlay({ onCancel }: SwitchingOverlayProps) {
  const to = useKoma((s) => s.ui.switchingTo)
  const loading = useKoma((s) => s.ui.loading)
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

  if (!to && !loading?.active) return null

  return (
    <div className="absolute inset-0 z-[60] flex flex-col items-center justify-center gap-4 bg-koma-bg/90 backdrop-blur-sm">
      <motion.div
        initial={{ opacity: 0, scale: 0.96 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="flex flex-col items-center gap-4"
      >
        {loading?.active ? (
          <LoadingSplash workspace={loading.workspace} awareness={loading.awareness} />
        ) : (
          <>
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
          </>
        )}
      </motion.div>
    </div>
  )
}

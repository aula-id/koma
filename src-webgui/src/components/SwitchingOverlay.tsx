import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { Check, Minus, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { BootstrapState, LoadPhase } from '../store/koma'
import { BootBrailleSpinner } from './BrailleSpinner'

// Duplicated from Titlebar.tsx's private (unexported) `post` helper — this
// overlay covers the titlebar region too and needs the same win-drag/maximize
// IPC post.
function post(msg: unknown) {
  try {
    window.ipc?.postMessage(JSON.stringify(msg))
  } catch {
    /* ipc unavailable */
  }
}

type SwitchingOverlayProps = {
  onCancel: () => void
}

// After this long still in pure "switching to …" (no Snapshot / Loading yet),
// surface a "taking longer than expected" hint so a wedged attach doesn't leave
// the user staring at a silent spinner. A legit build-skew daemon restart can
// take ~8s, so this must not fire before that.
const STUCK_HINT_MS = 10_000
// Last-resort auto-cancel ONLY for the pre-attach "switching to …" branch.
// Healthy Loading/indexing can legitimately run longer than this (large
// workspaces); auto-cancel must not fire there — that was detaching a live
// session and reopening the resume palette on every slow new-session.
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

// One phase row's status glyph: running = CSS braille (no JS ticker — survives
// main-thread Snapshot jank); pending = dim hollow dot; done = dim check;
// skipped = dim dash; failed = an error-role-tinted x.
function PhaseGlyph({ phase, errorTint }: { phase: LoadPhase; errorTint: string }) {
  switch (phase) {
    case 'running':
      return <BootBrailleSpinner size={13} className="flex-none" />
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

function PhaseRow({
  label,
  phase,
  errorTint,
}: {
  label: string
  phase: LoadPhase
  errorTint: string
}) {
  return (
    <div className="flex items-center gap-2 text-[12px] text-koma-fg opacity-80">
      <PhaseGlyph phase={phase} errorTint={errorTint} />
      <span>{label}</span>
    </div>
  )
}

// TUI-parity startup splash — the koma wordmark + the two cold-session warm-up
// phase lines (indexing workspace / reading project docs), mirroring the TUI's
// startup screen. Phase "running" glyphs use CSS braille, not the shared JS
// ticker, so they keep stepping while Snapshot apply blocks the main thread.
function LoadingSplash({
  bootstrap,
  workspace,
  awareness,
}: {
  bootstrap: BootstrapState | null
  workspace?: LoadPhase
  awareness?: LoadPhase
}) {
  const errorTint = useErrorTint()

  return (
    <div className="flex flex-col items-center gap-4">
      <span className="text-[22px] font-bold text-koma-fg">koma</span>
      <div className="flex flex-col gap-1.5">
        {bootstrap && (
          <>
            <PhaseRow label="loading session state" phase={bootstrap.session} errorTint={errorTint} />
            <PhaseRow label="loading configuration" phase={bootstrap.config} errorTint={errorTint} />
            <PhaseRow label="loading workspace settings" phase={bootstrap.settings} errorTint={errorTint} />
            <PhaseRow label="discovering repositories" phase={bootstrap.repos} errorTint={errorTint} />
          </>
        )}
        {workspace && <PhaseRow label="indexing workspace" phase={workspace} errorTint={errorTint} />}
        {awareness && <PhaseRow label="reading project docs" phase={awareness} errorTint={errorTint} />}
      </div>
    </div>
  )
}

// Full-screen loader shown while a session swap (SelectSession/NewSession) is
// in flight, and while cold-session warm-up (`ui.loading`) runs afterward.
// Raised optimistically by ResumePalette's startSwitching() and/or host
// Switching pushes. Cleared when Loading goes inactive (or Hub bounce).
//
// Presentation is side-loaded: CSS stepped braille (BootBrailleSpinner), not
// BrailleSpinner's setInterval — same glyphs, no React tick. Fat Snapshot
// parse can stall React; CSS keeps stepping. Overlay bg is fully opaque so a
// transparent wry window never shows through as a white freeze.
//
// Attach runs off the UI thread; Cancel is best-effort (HostCtl::ToSwapper
// after the in-flight attach returns).
export function SwitchingOverlay({ onCancel }: SwitchingOverlayProps) {
  const to = useKoma((s) => s.ui.switchingTo)
  const sessionId = useKoma((s) => s.session.id)
  const remoteState = useKoma((s) => s.remoteState)
  const remoteConnecting =
    !!to?.startsWith('remote ') &&
    !['ready', 'connected', 'error', 'disconnected'].includes(remoteState.state)
  const loading = useKoma((s) => s.ui.loading)
  const bootstrap = useKoma((s) => s.ui.bootstrap)
  const loadingDismissed = useKoma((s) => s.ui.loadingDismissed)
  const dismissLoading = useKoma((s) => s.dismissLoading)
  const showWarm = !!loading?.active && !loadingDismissed
  // Once Snapshot marks session done, don't keep the full-screen splash up for
  // trailing config/settings/repos — those were jamming chat behind the overlay.
  const showSplash =
    showWarm || (!!bootstrap && bootstrap.session !== 'done' && bootstrap.session !== 'skipped')
  // Pre-attach only: still waiting on the host, no Snapshot/Loading progress yet.
  // Once attach lands (session id set) or warm-up splash is up, auto-cancel must
  // not fire — CancelSwitch would detach a healthy session and reopen resume.
  const preAttachWaiting = !!to && !sessionId && !showSplash
  const [stuck, setStuck] = useState(false)
  const onCancelRef = useRef(onCancel)
  onCancelRef.current = onCancel

  useEffect(() => {
    if (!preAttachWaiting) {
      setStuck(false)
      return
    }
    const hint = window.setTimeout(() => setStuck(true), STUCK_HINT_MS)
    const autoCancel = window.setTimeout(() => onCancelRef.current(), AUTO_CANCEL_MS)
    return () => {
      window.clearTimeout(hint)
      window.clearTimeout(autoCancel)
    }
  }, [preAttachWaiting])

  // If settings/repos never reply, force-complete bootstrap and log why.
  // Depend on presence only — resetting on every phase tick would never fire.
  const bootstrapping = !!bootstrap
  useEffect(() => {
    if (!bootstrapping) return
    const t = window.setTimeout(() => {
      const b = useKoma.getState().ui.bootstrap
      if (!b) return
      try {
        window.komaIpc?.({
          r: 'WriteErrorLog',
          message: `[ui-pace] bootstrap timeout session=${b.session} config=${b.config} settings=${b.settings} repos=${b.repos}`,
        })
      } catch {
        /* ignore */
      }
      useKoma.getState().skipBootstrapRemaining()
    }, 8000)
    return () => window.clearTimeout(t)
  }, [bootstrapping])

  if (!to && !showSplash) return null

  // The overlay is full-screen, including the titlebar region — without this
  // it'd swallow every mousedown there and the frameless window couldn't be
  // dragged/maximized while switching/loading. Mirrors Titlebar.tsx's own
  // handleMouseDown (drag/double-click-maximize); `post` is duplicated here
  // (3 lines) since Titlebar's is a private, unexported helper. Clicks on an
  // actual button (e.g. Cancel) are excluded so they keep working.
  function handleMouseDown(e: MouseEvent<HTMLDivElement>) {
    if (e.button !== 0) return
    const target = e.target as HTMLElement
    if (target.closest('button, input, textarea, select, [contenteditable="true"]')) return
    if (e.detail === 2) {
      post({ t: 'win', a: 'max' })
      return
    }
    post({ t: 'win', a: 'drag' })
  }

  return (
    <div
      className="koma-boot-overlay absolute inset-0 z-[60] flex flex-col items-center justify-center gap-4"
      onMouseDown={handleMouseDown}
    >
      <div className="koma-boot-panel flex flex-col items-center gap-4">
        {showSplash ? (
          <>
            <LoadingSplash
              bootstrap={bootstrap}
              workspace={showWarm ? loading?.workspace : undefined}
              awareness={showWarm ? loading?.awareness : undefined}
            />
            {showWarm && (
              <button
                onClick={dismissLoading}
                className="mt-6 rounded-md border border-koma-border bg-koma-panel px-3 py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                Skip
              </button>
            )}
          </>
        ) : (
          <>
            <BootBrailleSpinner size={28} className="text-koma-accent" />
            <div className="text-[13px] text-koma-fg opacity-70">
              {remoteConnecting
                ? `${remoteState.state.replace('_', ' ')} ${to?.replace(/^remote /, '')}…`
                : `switching to ${to}…`}
            </div>
            {stuck && (
              <div className="text-[12px] text-koma-fg opacity-50">
                Taking longer than expected…
              </div>
            )}
            {remoteState.state === 'auth_required' ? (
              <div className="text-[12px] text-koma-fg opacity-50">
                Waiting for password…
              </div>
            ) : (
              <button
                onClick={() => {
                  if (remoteConnecting) {
                    useKoma.getState().req({ r: 'CancelRemoteConnect' })
                    useKoma.getState().cancelSwitching()
                    return
                  }
                  onCancel()
                }}
                className={`rounded-md border px-3 py-1.5 text-[12px] transition-colors ${
                  stuck
                    ? 'border-koma-accent bg-koma-accent/10 text-koma-accent opacity-100 hover:bg-koma-accent/20'
                    : 'border-koma-border bg-koma-panel text-koma-fg opacity-70 hover:opacity-100 hover:bg-koma-hover'
                }`}
              >
                Cancel
              </button>
            )}
          </>
        )}
      </div>
    </div>
  )
}

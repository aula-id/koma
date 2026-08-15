import { useCallback, useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import type { TutorialStep } from '../demos/first-run-tutorial'

/** Koma dark palette — 1:1 with theme.rs dark() */
const KOMA_THEME = {
  background: '#000000',
  foreground: '#e6e6e6',
  cursor: '#39ff14',
  cursorAccent: '#000000',
  selectionBackground: '#39ff14',
  selectionForeground: '#000000',
  black: '#000000',
  red: '#ff3c3c',
  green: '#39ff14',
  yellow: '#ffb43c',
  blue: '#50c8ff',
  magenta: '#c8d3f5',
  cyan: '#50c8ff',
  white: '#e6e6e6',
  brightBlack: '#adadad',
  brightRed: '#ff3c3c',
  brightGreen: '#00c853',
  brightYellow: '#ffb43c',
  brightBlue: '#50c8ff',
  brightMagenta: '#c8d3f5',
  brightCyan: '#50c8ff',
  brightWhite: '#ffffff',
}

export interface TuiTutorialProps {
  steps: TutorialStep[]
  /** Starting step index (default 0) */
  initialStep?: number
  /** Terminal columns (default 80) */
  cols?: number
  /** Terminal rows (default 24) — all screens are built to this exact height */
  rows?: number
}

export function TuiTutorial({
  steps,
  initialStep = 0,
  cols = 80,
  rows = 24,
}: TuiTutorialProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const [step, setStep] = useState(initialStep)
  const current = steps[step]
  const total = steps.length

  // Init terminal (once) — locked to cols×rows, no FitAddon
  useEffect(() => {
    if (!containerRef.current) return
    const term = new Terminal({
      theme: KOMA_THEME,
      fontFamily: "'KomaMono', 'JetBrains Mono', 'Fira Code', monospace",
      fontSize: 14,
      lineHeight: 1.3,
      cols,
      rows,
      cursorBlink: false,
      cursorStyle: 'block',
      allowProposedApi: true,
      disableStdin: true,
      convertEol: true,
      scrollback: 0,
      drawBoldTextInBrightColors: false,
    })
    term.open(containerRef.current)
    termRef.current = term

    return () => {
      term.dispose()
    }
  }, [cols, rows])

  // Write screen when step changes
  useEffect(() => {
    const term = termRef.current
    if (!term) return
    term.clear()
    term.reset()
    term.write(current.screen)
  }, [step, current.screen])

  const prev = useCallback(() => setStep((s) => Math.max(0, s - 1)), [])
  const next = useCallback(() => setStep((s) => Math.min(total - 1, s + 1)), [total])

  // Keyboard navigation
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
        e.preventDefault()
        prev()
      } else if (e.key === 'ArrowRight' || e.key === 'ArrowDown' || e.key === ' ') {
        e.preventDefault()
        next()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [prev, next])

  return (
    <div className="overflow-hidden rounded-lg border border-koma-border">
      {/* Step indicator */}
      <div className="flex items-center border-b border-koma-border bg-koma-panel px-4 py-2">
        <span className="text-xs text-koma-dim">
          Step {step + 1} of {total}
        </span>
        {/* Progress dots */}
        <div className="mx-3 flex gap-1.5">
          {steps.map((_, i) => (
            <button
              key={i}
              onClick={() => setStep(i)}
              className={`h-1.5 rounded-full transition-all ${
                i === step
                  ? 'w-4 bg-koma-accent'
                  : i < step
                    ? 'w-1.5 bg-koma-accent/40'
                    : 'w-1.5 bg-koma-dim/30'
              }`}
              aria-label={`Go to step ${i + 1}`}
            />
          ))}
        </div>
        <span className="ml-auto text-xs font-medium text-koma-fg">
          {current.title}
        </span>
      </div>

      {/* Narration */}
      <div className="border-b border-koma-border bg-koma-panel2 px-5 py-4">
        <p className="text-sm leading-relaxed text-koma-fg">{current.narration}</p>
        {current.points && current.points.length > 0 && (
          <ul className="mt-3 space-y-1.5">
            {current.points.map((pt, i) => (
              <li key={i} className="flex items-start gap-2 text-xs text-koma-dim">
                <span className="mt-0.5 text-koma-accent">&#x25cf;</span>
                {pt}
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Terminal — fixed pixel size from cols×rows, centred */}
      <div className="flex justify-center bg-black p-2">
        <div ref={containerRef} />
      </div>

      {/* Navigation */}
      <div className="flex items-center justify-between border-t border-koma-border bg-koma-panel px-4 py-2">
        <button
          onClick={prev}
          disabled={step === 0}
          className="rounded px-3 py-1 text-xs text-koma-dim transition hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"
        >
          &larr; Previous
        </button>
        <span className="text-[11px] text-koma-dim">
          Use arrow keys or spacebar to navigate
        </span>
        <button
          onClick={next}
          disabled={step === total - 1}
          className="rounded px-3 py-1 text-xs font-semibold text-koma-accent transition hover:bg-koma-hover disabled:opacity-30"
        >
          Next &rarr;
        </button>
      </div>
    </div>
  )
}

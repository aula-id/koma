import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

/** Koma dark palette — 1:1 with theme.rs dark() */
const KOMA_THEME = {
  background: '#0b0e14',
  foreground: '#e6e6e6',
  cursor: '#39ff14',
  cursorAccent: '#0b0e14',
  selectionBackground: '#39ff14',
  selectionForeground: '#0b0e14',
  // ANSI colors 0-7 (standard)
  black: '#0b0e14',
  red: '#ff3c3c',
  green: '#39ff14',
  yellow: '#ffb43c',
  blue: '#50c8ff',
  magenta: '#c8d3f5',
  cyan: '#50c8ff',
  white: '#e6e6e6',
  // ANSI colors 8-15 (bright)
  brightBlack: '#adadad',
  brightRed: '#ff3c3c',
  brightGreen: '#00c853',
  brightYellow: '#ffb43c',
  brightBlue: '#50c8ff',
  brightMagenta: '#c8d3f5',
  brightCyan: '#50c8ff',
  brightWhite: '#ffffff',
}

export interface TuiDemoProps {
  /** Script: array of { text, delay? } frames to play sequentially */
  script: Array<{ text: string; delay?: number }>
  /** Playback speed multiplier (1 = normal) */
  speed?: number
  /** Width in columns (default 80) */
  cols?: number
  /** Height in rows (default 24) */
  rows?: number
  /** Auto-play on mount */
  autoPlay?: boolean
  /** Show playback controls */
  controls?: boolean
}

export function TuiDemo({
  script,
  speed = 1,
  cols = 80,
  rows = 24,
  autoPlay = true,
  controls = true,
}: TuiDemoProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [playing, setPlaying] = useState(false)
  const [done, setDone] = useState(false)

  // Init terminal
  useEffect(() => {
    if (!containerRef.current) return

    const term = new Terminal({
      theme: KOMA_THEME,
      fontFamily: "'KomaMono', 'JetBrains Mono', 'Fira Code', monospace",
      fontSize: 14,
      lineHeight: 1.3,
      cols,
      rows,
      cursorBlink: true,
      cursorStyle: 'block',
      allowProposedApi: true,
      disableStdin: true,
      convertEol: true,
      scrollback: 0,
    })

    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(containerRef.current)
    termRef.current = term
    fitRef.current = fit

    return () => {
      if (timerRef.current) clearTimeout(timerRef.current)
      term.dispose()
    }
  }, [cols, rows])

  // Fit on resize
  useEffect(() => {
    const onResize = () => fitRef.current?.fit()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  const play = () => {
    const term = termRef.current
    if (!term || playing) return

    setPlaying(true)
    setDone(false)
    term.clear()
    term.reset()

    let i = 0
    const step = () => {
      if (i >= script.length) {
        setPlaying(false)
        setDone(true)
        return
      }
      const frame = script[i]
      term.write(frame.text)
      timerRef.current = setTimeout(step, (frame.delay ?? 50) / speed)
      i++
    }
    step()
  }

  const reset = () => {
    if (timerRef.current) clearTimeout(timerRef.current)
    termRef.current?.clear()
    termRef.current?.reset()
    setPlaying(false)
    setDone(false)
  }

  // Auto-play on mount
  useEffect(() => {
    if (autoPlay) {
      // small delay so the terminal is visible before writing
      timerRef.current = setTimeout(play, 300)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoPlay])

  return (
    <div className="overflow-hidden rounded-lg border border-koma-border">
      {controls && (
        <div className="flex items-center gap-3 border-b border-koma-border bg-koma-panel px-4 py-2">
          <button
            onClick={playing ? undefined : play}
            disabled={playing}
            className="rounded px-3 py-1 text-xs font-semibold text-koma-accent transition hover:bg-koma-hover disabled:opacity-40"
          >
            {playing ? 'playing...' : done ? 'replay' : '▶ play'}
          </button>
          <button
            onClick={reset}
            disabled={!playing && !done}
            className="rounded px-3 py-1 text-xs text-koma-dim transition hover:bg-koma-hover hover:text-koma-fg disabled:opacity-40"
          >
            reset
          </button>
          {speed !== 1 && (
            <span className="ml-auto text-[11px] text-koma-dim">{speed}x</span>
          )}
        </div>
      )}
      <div ref={containerRef} className="p-2" />
    </div>
  )
}

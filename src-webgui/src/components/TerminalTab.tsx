import { useEffect, useRef } from 'react'
import { useKoma } from '../store/koma'
import { isTabVisible, normalizeGroups } from '../store/editorGroups'

// xterm.js is loaded dynamically to avoid bloating the initial bundle.
// The actual Terminal class and addons are imported from @xterm/xterm.
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'

import '@xterm/xterm/css/xterm.css'
import { attachTerminalKeyGuards, normalizeTerminalInput } from '../lib/terminalInput'

type TerminalTabProps = {
  tab: { id: string; kind: 'terminal'; terminalId: string; title: string }
}

export function TerminalTab({ tab }: TerminalTabProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const req = useKoma((s) => s.req)
  const palette = useKoma((s) => s.palette)
  const isActive = useKoma((s) => isTabVisible(normalizeGroups(s.ui), tab.id))

  const terminalId = tab.terminalId

  // Derive xterm theme from the active Koma palette.
  const theme = {
    background: palette.bg,
    foreground: palette.fg,
    cursor: palette.accent,
    cursorAccent: palette.bg,
    selectionBackground: palette.panel,
    selectionForeground: palette.fg,
    // ANSI colors derived from palette semantic roles.
    black: palette.bg,
    red: palette.error,
    green: palette.success,
    yellow: palette.warn,
    blue: palette.info,
    magenta: palette.accent,
    cyan: palette.info,
    white: palette.fg,
    brightBlack: palette.dim,
    brightRed: palette.error,
    brightGreen: palette.success,
    brightYellow: palette.warn,
    brightBlue: palette.info,
    brightMagenta: palette.accent,
    brightCyan: palette.info,
    brightWhite: palette.fg,
  }

  // Mount xterm.js on first render.
  useEffect(() => {
    if (!containerRef.current) return

    const term = new Terminal({
      fontFamily: "'KomaMono', ui-monospace, 'JetBrains Mono', 'SFMono-Regular', Menlo, Consolas, monospace",
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      cursorStyle: 'bar',
      theme,
      allowProposedApi: true,
      scrollback: 10000,
    })

    const fitAddon = new FitAddon()
    const webLinksAddon = new WebLinksAddon()

    term.loadAddon(fitAddon)
    term.loadAddon(webLinksAddon)
    attachTerminalKeyGuards(term)
    term.open(containerRef.current)

    // Fit to container.
    requestAnimationFrame(() => fitAddon.fit())

    termRef.current = term
    fitRef.current = fitAddon

    // Register the writer callback so the push reducer can route PTY output here.
    if (!(globalThis as any).__terminalWriters) {
      ;(globalThis as any).__terminalWriters = {}
    }
    ;(globalThis as any).__terminalWriters[terminalId] = (data: string) => {
      term.write(data)
    }

    const pending = (globalThis as any).__terminalPending
    if (pending?.[terminalId]) { term.write(pending[terminalId]); delete pending[terminalId] }

    // Register exit handler.
    if (!(globalThis as any).__terminalExitHandlers) {
      ;(globalThis as any).__terminalExitHandlers = {}
    }
    ;(globalThis as any).__terminalExitHandlers[terminalId] = (code: number | null) => {
      term.write(`\r\n\x1b[90m[Process exited with code ${code ?? 'signal'}]\x1b[0m\r\n`)
    }

    // Forward keystrokes from xterm to the host PTY.
    const selectionDisposable = term.onSelectionChange(() => document.dispatchEvent(new CustomEvent('koma-terminal-selection', { detail: { selected: term.hasSelection() } })))
    const disposable = term.onData((data) => {
      req({ r: 'TerminalInput', id: terminalId, data: normalizeTerminalInput(data) })
    })

    return () => {
      selectionDisposable.dispose()
      disposable.dispose()
      delete (globalThis as any).__terminalWriters?.[terminalId]
      delete (globalThis as any).__terminalExitHandlers?.[terminalId]
      term.dispose()
      termRef.current = null
      fitRef.current = null
    }
    // Only run on mount/unmount — theme changes are handled separately.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [terminalId])

  // Update xterm theme when the palette changes.
  useEffect(() => {
    if (termRef.current) {
      termRef.current.options.theme = theme
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [palette])

  // Resize the PTY when the container resizes.
  useEffect(() => {
    const container = containerRef.current
    const term = termRef.current
    const fit = fitRef.current
    if (!container || !term || !fit) return

    const resize = () => {
      const scroll = term.buffer.active.viewportY
      const atBottom = scroll === term.buffer.active.baseY
      fit.fit()
      const dims = fit.proposeDimensions()
      if (dims) {
        req({ r: 'TerminalResize', id: terminalId, cols: dims.cols, rows: dims.rows })
      }
      if (atBottom) term.scrollToBottom()
      else term.scrollToLine(scroll)
    }
    const observer = new ResizeObserver(resize)
    window.addEventListener('koma-ui-scale', resize)
    observer.observe(container)

    // Initial fit + resize.
    fit.fit()
    const dims = fit.proposeDimensions()
    if (dims) {
      req({ r: 'TerminalResize', id: terminalId, cols: dims.cols, rows: dims.rows })
    }

    return () => { observer.disconnect(); window.removeEventListener('koma-ui-scale', resize) }
  }, [terminalId, req])

  // display:none inactive panes: refit when this terminal becomes visible again.
  useEffect(() => {
    if (!isActive) return
    const fit = fitRef.current
    if (!fit) return
    const raf = requestAnimationFrame(() => {
      fit.fit()
      const dims = fit.proposeDimensions()
      if (dims) {
        req({ r: 'TerminalResize', id: terminalId, cols: dims.cols, rows: dims.rows })
      }
    })
    return () => cancelAnimationFrame(raf)
  }, [isActive, terminalId, req])

  return (
    <div
      data-tour="terminal-content"
      ref={containerRef}
      className="h-full w-full overflow-hidden bg-koma-bg"
      style={{ padding: '4px 0 0 4px' }}
      onMouseDown={() => termRef.current?.focus()}
    />
  )
}

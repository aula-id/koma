import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { useKoma } from '../store/koma'
import { codingRequest, type WorkspaceRef } from '../lib/coding-service'
type Chunk = { seq: number; text: string }
export default function CodingTaskTerminal({ workspace, runId, chunks, running, onError }: { workspace: WorkspaceRef; runId: string; chunks: Chunk[]; running: boolean; onError: (error: string) => void }) {
  const container = useRef<HTMLDivElement>(null), terminal = useRef<Terminal | null>(null), pump = useRef<() => void>(() => {}), layout = useRef<() => void>(() => {})
  const current = useRef({ chunks, running, onError }); current.current = { chunks, running, onError }
  const palette = useKoma(s => s.palette)
  const theme = { background: palette.bg, foreground: palette.fg, cursor: palette.accent, cursorAccent: palette.bg, selectionBackground: palette.panel, selectionForeground: palette.fg, black: palette.bg, red: palette.error, green: palette.success, yellow: palette.warn, blue: palette.info, magenta: palette.accent, cyan: palette.info, white: palette.fg, brightBlack: palette.dim, brightRed: palette.error, brightGreen: palette.success, brightYellow: palette.warn, brightBlue: palette.info, brightMagenta: palette.accent, brightCyan: palette.info, brightWhite: palette.fg }
  useEffect(() => {
    if (!container.current) return
    const term = new Terminal({ fontFamily: "'KomaMono', ui-monospace, 'JetBrains Mono', 'SFMono-Regular', Menlo, Consolas, monospace", fontSize: 13, lineHeight: 1.2, cursorBlink: true, cursorStyle: 'bar', theme, scrollback: 10000, convertEol: true })
    const fit = new FitAddon(); term.loadAddon(fit); term.open(container.current); terminal.current = term
    let stopped = false, sequence = 0, pending = '', sending = false
    let inputTimer: ReturnType<typeof setTimeout>, resizeTimer: ReturnType<typeof setTimeout>
    const send = async () => {
      if (stopped || sending || !pending) return
      const data = pending.slice(0, 8192); pending = pending.slice(data.length); sending = true
      try { await codingRequest(workspace, { op: 'taskInput', runId, data }) }
      catch (e) { pending = ''; if (!stopped) current.current.onError(String(e)) }
      finally { sending = false; if (!stopped && pending) inputTimer = setTimeout(send, 0) }
    }
    const input = term.onData(data => { if (!current.current.running) return; if (pending.length + data.length > 65536) { current.current.onError('Terminal input is busy; paste a smaller block.'); return } pending += data; clearTimeout(inputTimer); inputTimer = setTimeout(send, 20) })
    pump.current = () => {
      const next = current.current.chunks.filter(c => c.seq > sequence)
      if (next.length && next[0].seq > sequence + 1) term.write('\r\n[Earlier terminal output omitted]\r\n')
      for (const chunk of next) { term.write(chunk.text); sequence = chunk.seq }
    }
    layout.current = () => { if (stopped) return; fit.fit(); if (current.current.running) { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => { if (!stopped) void codingRequest(workspace, { op: 'taskResize', runId, rows: term.rows, cols: term.cols }).catch(e => { if (!stopped && current.current.running) current.current.onError(String(e)) }) }, 100) } }
    const observer = new ResizeObserver(() => layout.current()); observer.observe(container.current); layout.current(); pump.current()
    return () => { stopped = true; clearTimeout(inputTimer); clearTimeout(resizeTimer); observer.disconnect(); input.dispose(); pump.current = () => {}; layout.current = () => {}; term.dispose(); terminal.current = null }
  }, [workspace.hostId, workspace.root, runId])
  useEffect(() => { pump.current() }, [chunks])
  useEffect(() => { layout.current(); if (terminal.current) terminal.current.options.disableStdin = !running }, [running])
  useEffect(() => { if (terminal.current) terminal.current.options.theme = theme }, [palette])
  return <div ref={container} className="min-h-0 flex-1 overflow-hidden px-2 py-1" aria-label="Interactive task terminal" />
}

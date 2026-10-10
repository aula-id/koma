import { pageRect, pagePoint } from '../lib/uiScale'
import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { SquareTerminal } from 'lucide-react'
import { useKoma } from '../store/koma'

import type { TerminalShell, TerminalShellReply } from '../lib/terminalShells'

export function TerminalPicker({ onSelect }: { onSelect: (shell: TerminalShell) => void }) {
  const req = useKoma(s => s.req)
  const remote = useKoma(s => s.remoteState)
  const context = ['ready', 'connected'].includes(remote.state) ? remote.hostId ?? 'local' : 'local'
  const [open, setOpen] = useState(false)
  const [reply, setReply] = useState<TerminalShellReply | null>(null)
  const [rect, setRect] = useState<DOMRect | null>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const menu = useRef<HTMLDivElement>(null)
  const request = useRef<string | null>(null)
  const activeContext = useRef(context)
  activeContext.current = context
  const close = (focus = false) => { setOpen(false); request.current = null; if (focus) trigger.current?.focus() }
  const discover = () => {
    const requestId = crypto.randomUUID()
    request.current = requestId
    setReply(null)
    req({ r: 'TerminalShells', request_id: requestId, context })
  }

  useEffect(() => { close(); setReply(null) }, [context, remote.state])
  useEffect(() => {
    const receive = (event: Event) => {
      const result = (event as CustomEvent<TerminalShellReply>).detail
      if (result.requestId === request.current && result.context === activeContext.current) setReply(result)
    }
    window.addEventListener('koma-terminal-shells', receive)
    return () => window.removeEventListener('koma-terminal-shells', receive)
  }, [])
  useEffect(() => {
    if (!open) return
    const update = () => setRect(trigger.current ? pageRect(trigger.current.getBoundingClientRect()) : null)
    update()
    menu.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus()
    const outside = (event: MouseEvent) => {
      if (!trigger.current?.contains(event.target as Node) && !menu.current?.contains(event.target as Node)) close()
    }
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); close(true) }
      if (event.key === 'Tab') close()
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
      event.preventDefault()
      const items = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])
      const current = items.indexOf(document.activeElement as HTMLButtonElement)
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (current + (event.key === 'ArrowUp' ? -1 : 1) + items.length) % items.length
      items[next]?.focus()
    }
    window.addEventListener('mousedown', outside)
    window.addEventListener('keydown', key)
    window.addEventListener('resize', update)
    window.addEventListener('scroll', update, true)
    return () => {
      window.removeEventListener('mousedown', outside)
      window.removeEventListener('keydown', key)
      window.removeEventListener('resize', update)
      window.removeEventListener('scroll', update, true)
    }
  }, [open])
  useEffect(() => {
    if (open && rect && !menu.current?.contains(document.activeElement)) menu.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus()
  }, [open, rect])
  useEffect(() => {
    if (!open || reply) return
    const timeout = window.setTimeout(() => {
      request.current = null
      setReply({ requestId: '', context, shells: [], error: 'Discovery timed out' })
    }, 20000)
    return () => window.clearTimeout(timeout)
  }, [open, reply, context])
  const rows = [{ label: 'Default shell' }, ...(reply?.shells ?? [])]
  return <>
    <button type="button" ref={trigger} title="New Terminal" aria-label="New Terminal" aria-haspopup="menu" aria-expanded={open}
      onClick={() => { if (open) close(); else { setOpen(true); discover() } }}
      onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); setOpen(true); discover() } }}
      className="pointer-events-auto flex h-[22px] flex-none items-center rounded-md border border-koma-border bg-koma-panel px-1.5 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover">
      <SquareTerminal size={13} className="flex-none" />
    </button>
    {open && rect && createPortal(<div ref={menu} onMouseDown={event => event.stopPropagation()} role="menu" aria-label="Terminal shells"
      style={{ position: 'fixed', top: rect.bottom + 6, left: Math.max(8, Math.min(rect.left, pagePoint(window.innerWidth) - 228)), width: 220, zIndex: 80, maxHeight: '70vh' }}
      className="overflow-y-auto rounded-md border border-koma-border bg-koma-panel py-1 text-xs text-koma-fg shadow-sm">
      {rows.map((shell, index) => <button type="button" key={shell.id ?? 'default'} role="menuitem" tabIndex={-1}
        className="block w-full px-2 py-1 text-left text-[12px] text-koma-fg opacity-75 transition-colors hover:bg-koma-hover hover:opacity-100 focus:bg-koma-hover focus:opacity-100 focus:outline-none"
        onClick={() => { onSelect(shell); close(true) }}>{shell.label}{index > 0 && rows.filter(row => row.label === shell.label).length > 1 ? ` (${index})` : ''}</button>)}
      {!reply && <div role="status" className="px-2 py-1 text-koma-dim">Finding shells…</div>}
      {reply?.error && <><div role="alert" className="px-2 py-1 text-koma-dim">Could not discover shells.</div><button type="button" role="menuitem" tabIndex={-1} className="w-full px-2 py-1 text-left text-[12px] text-koma-fg opacity-75 transition-colors hover:bg-koma-hover hover:opacity-100 focus:bg-koma-hover focus:opacity-100" onClick={discover}>Retry</button></>}
    </div>, document.body)}
  </>
}
